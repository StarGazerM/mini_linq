//! `replay <hole-136 capture round directory> [...]`.
use cuda_core::LaunchConfig1D;
use gpu_datalog_runtime::{Buffer, Pool, Result};
use join_lab::replay::{Lookups, Table, replay_kernels as kernels};
use serde_json::Value;
use std::{fs, io::Read, path::Path, sync::Arc};

const COLUMNS: [usize; 9] = [2, 2, 2, 1, 2, 2, 4, 2, 2];
const READS: [&str; 9] = ["delta varpointsto", "instruction_method[0, 1]", "heapallocation_type[0, 1]",
    "reachable[0]", "virtualmethodinvocation_simplename[0, 1]", "virtualmethodinvocation_descriptor[0, 1]",
    "methodlookup[0, 1, 2, 3]", "thisvar[0, 1]", "virtualmethodinvocation_base[1, 0]"];

fn read_table(path: &Path, words: usize) -> Result<Vec<u32>> {
    let mut f = fs::File::open(path)?;
    if f.metadata()?.len() != (words * 4) as u64 { return Err(format!("wrong size: {}", path.display()).into()); }
    let mut data = vec![0u32; words];
    // Every bit pattern is a valid u32; this byte slice exclusively borrows data.
    f.read_exact(unsafe { std::slice::from_raw_parts_mut(data.as_mut_ptr().cast(), words * 4) })?;
    #[cfg(target_endian = "big")]
    data.iter_mut().for_each(|x| *x = u32::from_le(*x));
    Ok(data)
}

fn tails(table: &[[u32; 2]], key: u32) -> impl Iterator<Item = u32> + '_ {
    table[table.partition_point(|r| r[0] < key)..].iter().take_while(move |r| r[0] == key).map(|r| r[1])
}

struct Oracle { rows: Vec<u64>, survivors: u32, invocations: u64 }

fn oracle(t: &[Vec<u32>]) -> Oracle {
    let pairs = |i: usize| t[i].as_chunks::<2>().0;
    let lookup = t[6].as_chunks::<4>().0;
    let mut answer = Oracle { rows: Vec::new(), survivors: 0, invocations: 0 };
    for &[heap, base] in pairs(0) {
        for (j, inv) in tails(pairs(8), base).enumerate() {
            answer.survivors += u32::from(j == 0);
            answer.invocations += 1;
            for method in tails(pairs(1), inv) {
                if t[3].binary_search(&method).is_err() { continue; }
                for ty in tails(pairs(2), heap) {
                    for name in tails(pairs(4), inv) {
                        for desc in tails(pairs(5), inv) {
                            let key = [name, desc, ty];
                            let start = lookup.partition_point(|r| r[..3] < key[..]);
                            for target in lookup[start..].iter().take_while(|r| r[..3] == key) {
                                for this in tails(pairs(7), target[3]) {
                                    answer.rows.push(u64::from(heap) << 32 | u64::from(this));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    answer.rows.sort_unstable();
    answer
}

fn checksum(rows: &[u64]) -> u64 {
    rows.iter().map(|x| {
        let mut h = x.wrapping_add(0x9e3779b97f4a7c15);
        h = (h ^ (h >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        h = (h ^ (h >> 27)).wrapping_mul(0x94d049bb133111eb);
        h ^ (h >> 31)
    }).fold(0, u64::wrapping_add)
}

fn verify(out: &Buffer<u64>, count: &Buffer<u32>, expected: &Oracle, manifest: usize) -> Result<()> {
    let n = count.to_host()?[0] as usize;
    if n != manifest { return Err(format!("candidate count {n} != manifest {manifest}").into()); }
    let mut rows = out.to_host_prefix(n)?;
    let sum = checksum(&rows);
    rows.sort_unstable();
    if sum != checksum(&expected.rows) || rows != expected.rows { return Err("CPU/GPU output multiset mismatch".into()); }
    Ok(())
}

fn clear(b: &Buffer<u32>) -> Result<()> {
    if b.bytes() != 0 { unsafe {
        cuda_core::simt::memory::memset_d8_async(b.cu_deviceptr(), 0, b.bytes(), b.pool().stream().cu_stream())?;
    } }
    Ok(())
}

fn median(pool: &Pool, mut reset: impl FnMut() -> Result<()>, mut launch: impl FnMut() -> Result<()>) -> Result<f64> {
    reset()?; launch()?;
    let flags = Some(cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT);
    let mut times = [0.0; 9];
    for t in &mut times {
        reset()?;
        let a = pool.stream().record_event(flags)?;
        launch()?;
        let b = pool.stream().record_event(flags)?;
        b.synchronize()?;
        *t = f64::from(a.elapsed_ms(&b)?);
    }
    times.sort_by(f64::total_cmp);
    Ok(times[4])
}

fn round(pool: &Arc<Pool>, module: &kernels::LoadedModule, path: &Path) -> Result<()> {
    let m: Value = serde_json::from_slice(&fs::read(path.join("manifest.json"))?)?;
    if m["hole"] != 136 || m["label"] != "varpointsto@3" || m["inputs"].as_array().map(Vec::len) != Some(9) {
        return Err("expected a varpointsto@3#136 capture".into());
    }
    let mut host = Vec::with_capacity(9);
    let mut sizes = [0u32; 9];
    for (i, cols) in COLUMNS.into_iter().enumerate() {
        if m["reads"][i] != READS[i] || m["inputs"][i]["columns"] != cols { return Err(format!("unexpected schema at input {i}").into()); }
        sizes[i] = m["inputs"][i]["rows"].as_u64().ok_or("missing row count")?.try_into()?;
        host.push(read_table(&path.join(format!("{i}.u32")), sizes[i] as usize * cols)?);
    }
    let capacity: u32 = m["candidates"].as_u64().ok_or("missing candidate oracle")?.try_into()?;
    let expected = oracle(&host);
    if expected.rows.len() != capacity as usize { return Err("CPU candidate count differs from manifest".into()); }
    let nwords = host[8].as_chunks::<2>().0.last().map_or(0, |r| r[0] / 32 + 1);
    let data = host.iter().map(|v| Buffer::from_host(pool.clone(), v)).collect::<Result<Vec<_>>>()?;
    drop(host);
    let table = |i: usize| Table { words: data[i].ptr(), rows: sizes[i] };
    let tables = Lookups { method: table(1), heap_type: table(2), reachable: table(3), name: table(4), descriptor: table(5), lookup: table(6), this_var: table(7), base: table(8) };
    let n = sizes[0];
    let delta = data[0].ptr().cast::<u64>();
    let delta_count = Buffer::from_host(pool.clone(), &[n])?;
    let bits = Buffer::<u32>::zeroed(pool.clone(), nwords as usize)?;
    let kept = unsafe { Buffer::<u64>::uninitialized(pool.clone(), n as usize)? };
    let survivors = Buffer::<u32>::zeroed(pool.clone(), 1)?;
    let out = unsafe { Buffer::<u64>::uninitialized(pool.clone(), capacity as usize)? };
    let count = Buffer::<u32>::zeroed(pool.clone(), 1)?;
    let stream = pool.stream();
    let config = |blocks| LaunchConfig1D::new(blocks, 256, 0);
    let build = module.prepare_replay_build(config(sizes[8].div_ceil(256).max(1)))?;
    let filter = module.prepare_replay_filter(config(n.div_ceil(256).max(1)))?;
    let base = module.prepare_replay_chain(config(n.div_ceil(256).max(1)))?;
    let blocks = cq_gpu_runtime::launch_grid_for_kernel(base.function(), n as usize, 256, 0)?;
    let compact = module.prepare_replay_chain(config(blocks))?;
    let index_ms = median(pool, || Ok(()), || {
        clear(&bits)?;
        Ok(module.replay_build(stream, &build, tables.base, bits.ptr())?)
    })?;
    let filter_once = || Ok(module.replay_filter(stream, &filter, delta, n, bits.ptr(), nwords, kept.ptr(), survivors.ptr())?);
    let base_once = || Ok(module.replay_chain(stream, &base, delta, delta_count.ptr(), tables, out.ptr(), capacity, count.ptr())?);
    let compact_once = || Ok(module.replay_chain(stream, &compact, kept.ptr(), survivors.ptr(), tables, out.ptr(), capacity, count.ptr())?);
    let base_ms = median(pool, || clear(&count), base_once)?;
    verify(&out, &count, &expected, capacity as usize)?;
    let filter_ms = median(pool, || clear(&survivors), filter_once)?;
    if survivors.to_host()?[0] != expected.survivors { return Err("CPU/GPU prefilter count mismatch".into()); }
    let chain_ms = median(pool, || clear(&count), compact_once)?;
    verify(&out, &count, &expected, capacity as usize)?;
    let both_ms = median(pool, || { clear(&count)?; clear(&survivors) }, || { filter_once()?; compact_once() })?;
    verify(&out, &count, &expected, capacity as usize)?;
    println!("{}\t{n}\t{}\t{}\t{capacity}\t{:016x}\t{}\t{blocks}\t{index_ms:.6}\t{base_ms:.6}\t{filter_ms:.6}\t{chain_ms:.6}\t{both_ms:.6}",
        m["iteration"], expected.survivors, expected.invocations, checksum(&expected.rows), nwords * 4);
    Ok(())
}

fn main() -> Result<()> {
    let pool = gpu_datalog_runtime::thread_pool()?;
    let linked = cuda_host::load_all_ptx_bundles_merged(pool.context())?;
    let module = unsafe { kernels::from_module(linked)? };
    let paths: Vec<_> = std::env::args_os().skip(1).collect();
    if paths.is_empty() { return Err("usage: replay <hole-136 round directory> [...]".into()); }
    println!("round\tscan\tsurvivors\tinvocations\tcandidates\tchecksum\tbitset_bytes\tcompact_blocks\tindex_ms\tbaseline_ms\tfilter_ms\tchain_ms\tcombined_ms");
    for path in paths { round(&pool, &module, Path::new(&path))?; }
    Ok(())
}
