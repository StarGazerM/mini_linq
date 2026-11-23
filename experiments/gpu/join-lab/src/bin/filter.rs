//! `filter`: multi-version mask prefilter vs per-version binary search,
//! synthetic class-A deltas, CPU oracle on every configuration.
use cuda_core::LaunchConfig1D;
use gpu_datalog_runtime::{Buffer, Pool, Result};
use join_lab::filter::filter_kernels as kernels;
use std::sync::Arc;

fn hash(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

fn time(pool: &Arc<Pool>, repeat: usize, mut launch: impl FnMut() -> Result<()>) -> Result<f64> {
    launch()?;
    let stream = pool.stream();
    let mut samples = Vec::new();
    for _ in 0..repeat {
        let flags = Some(cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT);
        let start = stream.record_event(flags)?;
        launch()?;
        let end = stream.record_event(flags)?;
        end.synchronize()?;
        samples.push(start.elapsed_ms(&end)? as f64);
    }
    samples.sort_by(f64::total_cmp);
    Ok(samples[repeat / 2])
}

fn main() -> Result<()> {
    let pool = gpu_datalog_runtime::thread_pool()?;
    let stream = pool.stream();
    let module = unsafe { kernels::load(pool.context())? };
    let config = |blocks| LaunchConfig1D::new(blocks, 256, 0);
    let resident = cq_gpu_runtime::launch_grid_for_kernel(module.prepare_filter_mask(config(1))?.function(), usize::MAX, 256, 0)?;
    let n: u64 = 1 << 26;
    let domain: u32 = 8 << 20;
    let seed = 0x5eed;
    let rows = unsafe { Buffer::<u64>::uninitialized(pool.clone(), n as usize)? };
    module.synth_delta(stream, &module.prepare_synth_delta(config(resident))?, rows.ptr(), n, domain, seed)?;
    let host_vars: Vec<u32> = (0..n).map(|i| (hash(i ^ seed) as u32) % domain).collect();
    println!("case\tversions\tkeys_per_version\thits_total\tmask_ms\tmask_G_rows_s\twarp_ms\twarp_G_rows_s\tsearch_ms\tsearch_G_rows_s\tbound_ms\toracle");
    for (versions, keys) in [(1u32, 20_000u32), (4, 20_000), (16, 20_000), (1, 2_500_000), (16, 200_000)] {
        // Version v's index keys: `keys` distinct random codes.
        let mut sets: Vec<Vec<u32>> = Vec::new();
        for v in 0..versions {
            let mut set: Vec<u32> = (0..keys as u64 * 2).map(|i| (hash(i ^ ((v as u64 + 1) << 40)) as u32) % domain).collect();
            set.sort_unstable();
            set.dedup();
            set.truncate(keys as usize);
            sets.push(set);
        }
        let stride = sets.iter().map(Vec::len).max().unwrap() as u32;
        let mut flat = vec![u32::MAX; (stride * versions) as usize];
        let mut lengths = Vec::new();
        let mut host_mask = vec![0u32; domain as usize];
        for (v, set) in sets.iter().enumerate() {
            flat[v * stride as usize..v * stride as usize + set.len()].copy_from_slice(set);
            lengths.push(set.len() as u32);
            for &k in set { host_mask[k as usize] |= 1 << v; }
        }
        let mut expected = vec![0u64; versions as usize];
        for &var in &host_vars {
            let mut bits = host_mask[var as usize];
            while bits != 0 { expected[bits.trailing_zeros() as usize] += 1; bits &= bits - 1; }
        }
        let keys_dev = Buffer::from_host(pool.clone(), &flat)?;
        let lengths_dev = Buffer::from_host(pool.clone(), &lengths)?;
        let mask = Buffer::<u32>::zeroed(pool.clone(), domain as usize)?;
        for (v, set) in sets.iter().enumerate() {
            module.set_mask(stream, &module.prepare_set_mask(config(resident))?, unsafe { keys_dev.ptr().add(v * stride as usize) }, set.len() as u32, v as u32, mask.ptr())?;
        }
        let total: u64 = expected.iter().sum();
        let capacity = expected.iter().copied().max().unwrap().max(1);
        let outs = unsafe { Buffer::<u64>::uninitialized(pool.clone(), (capacity * versions as u64) as usize)? };
        let counts = Buffer::<u64>::zeroed(pool.clone(), 32)?;
        let p = module.prepare_filter_mask(config(resident))?;
        let mask_ms = time(&pool, 7, || {
            unsafe { cuda_core::simt::memory::memset_d8_async(counts.cu_deviceptr(), 0, 32 * 8, stream.cu_stream())?; }
            Ok(module.filter_mask(stream, &p, rows.ptr(), n, mask.ptr(), outs.ptr(), capacity, counts.ptr())?)
        })?;
        let got_mask = counts.to_host()?;
        let p = module.prepare_filter_search(config(resident))?;
        let search_ms = time(&pool, 3, || {
            unsafe { cuda_core::simt::memory::memset_d8_async(counts.cu_deviceptr(), 0, 32 * 8, stream.cu_stream())?; }
            Ok(module.filter_search(stream, &p, rows.ptr(), n, keys_dev.ptr(), stride, lengths_dev.ptr(), versions, counts.ptr())?)
        })?;
        let got_search = counts.to_host()?;
        let p = module.prepare_filter_mask_warp(config(resident))?;
        let warp_ms = time(&pool, 7, || {
            unsafe { cuda_core::simt::memory::memset_d8_async(counts.cu_deviceptr(), 0, 32 * 8, stream.cu_stream())?; }
            Ok(module.filter_mask_warp(stream, &p, rows.ptr(), n, mask.ptr(), versions, outs.ptr(), capacity, counts.ptr())?)
        })?;
        let got_warp = counts.to_host()?;
        // Row checksum per version: the surviving rows themselves, not only counts.
        let host_outs = outs.to_host()?;
        let mut expected_sum = vec![0u64; versions as usize];
        for i in 0..n {
            let h = hash(i ^ seed);
            let row = ((h >> 40) << 32) | ((h as u32) % domain) as u64;
            let mut bits = host_mask[(row & 0xffff_ffff) as usize];
            while bits != 0 { let v = bits.trailing_zeros() as usize; expected_sum[v] = expected_sum[v].wrapping_add(row); bits &= bits - 1; }
        }
        let sums_ok = (0..versions as usize).all(|v| {
            let start = v * capacity as usize;
            host_outs[start..start + expected[v] as usize].iter().fold(0u64, |a, &r| a.wrapping_add(r)) == expected_sum[v]
        });
        let ok = sums_ok && (0..versions as usize).all(|v| got_mask[v] == expected[v] && got_search[v] == expected[v] && got_warp[v] == expected[v]);
        // Bound: stream n rows (8 B) at 890 GB/s plus one random L2 load per row at 96 G/s.
        let bound = n as f64 * 8.0 / 890e9 * 1e3 + n as f64 / 96e9 * 1e3;
        println!("synthetic\t{versions}\t{keys}\t{total}\t{mask_ms:.3}\t{:.1}\t{warp_ms:.3}\t{:.1}\t{search_ms:.3}\t{:.2}\t{bound:.3}\t{}",
            n as f64 / mask_ms / 1e6, n as f64 / warp_ms / 1e6, n as f64 / search_ms / 1e6, if ok { "pass" } else { "FAIL" });
    }
    Ok(())
}
