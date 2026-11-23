//! `probe [small|large|all] [distribution|all] [1|4|8|all] [method|all]`.
use cuda_core::LaunchConfig1D;
use gpu_datalog_runtime::{Buffer, Pool, Result};
use join_lab::probe::probe_kernels as kernels;
use std::sync::Arc;

struct SortColumn(Buffer<u64>);
impl rccl_cub::DeviceRange for SortColumn { fn len(&self) -> usize { self.0.len() } }
// SAFETY: this private wrapper owns the entire allocation while RCCL sorts it.
unsafe impl cuda_host::KernelSliceArg for SortColumn {
    type Elem = u64;
    fn cu_deviceptr(&self) -> u64 { self.0.cu_deviceptr() }
    fn len(&self) -> usize { self.0.len() }
}
unsafe impl cuda_host::KernelSliceArgMut for SortColumn {}

// First launch warms the operator; events bracket only the measured launch(es).
fn timed(pool: &Pool, mut prepare: impl FnMut() -> Result<()>, mut launch: impl FnMut() -> Result<()>) -> Result<f64> {
    prepare()?; launch()?;
    let flags = Some(cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT);
    let mut samples = [0.0; 7];
    for t in &mut samples {
        prepare()?;
        let start = pool.stream().record_event(flags)?;
        launch()?;
        let stop = pool.stream().record_event(flags)?;
        stop.synchronize()?;
        *t = f64::from(start.elapsed_ms(&stop)?);
    }
    samples.sort_by(f64::total_cmp);
    Ok(samples[3])
}

fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

struct Case { a: Vec<u64>, b: Vec<u64>, index: Vec<u64>, bits: Vec<u32>, oracle: (u64, u64) }
fn generate(n: usize, m: usize, distribution: &str) -> Case {
    // One random key in each pair of slots, the other slot is a genuine in-range miss.
    let index: Vec<_> = (0..m as u64).map(|k| 2*k + (mix(k+177) & 1)).collect();
    let mut bits = vec![0u32; (2*m).div_ceil(32)];
    for &k in &index { bits[k as usize / 32] |= 1 << (k % 32); }
    let mut cdf = Vec::new();
    let mut z = 0.;
    if distribution == "zipf" {
        for k in 1..=m { z += (k as f64).powf(-1.1); cdf.push(z); }
    }
    let mut a = Vec::with_capacity(n);
    let mut b = Vec::with_capacity(n);
    let (mut count, mut checksum) = (0, 0u64);
    for i in 0..n as u64 {
        let r = mix(i+901);
        let bin = if distribution == "zipf" {
            let p = (r >> 11) as f64 * (1.0 / (1u64 << 53) as f64) * z;
            cdf.partition_point(|&x| x <= p).min(m-1)
        } else { r as usize & (m-1) };
        let coin = mix(i+0x1234567) % 10_000;
        let key = match distribution {
            "hot" if r >> 63 != 0 => index[0] ^ 1,
            "hot" => index[bin] ^ u64::from(coin >= 40),
            "uniform" | "zipf" => index[bin] ^ u64::from(coin >= 20),
            "miss" => index[bin] ^ 1,
            "hit30" => index[bin] ^ u64::from(coin >= 3000),
            _ => unreachable!(),
        };
        let value = 3*i + 7;
        a.push(value); b.push(key);
        // Independent oracle: do not consult the GPU bitmap or the generator's hit coin.
        if index.binary_search(&key).is_ok() {
            count += 1; checksum = checksum.wrapping_add(value ^ key.rotate_left(21));
        }
    }
    Case { a, b, index, bits, oracle: (count, checksum) }
}

fn one(pool: &Arc<Pool>, n: usize, m: usize, distribution: &str, run_filter: &str, method: &str) -> Result<()> {
    let case = generate(n, m, distribution);
    let stream = pool.stream();
    // The binary emits RCCL specializations and shares its package name with
    // the kernel library. Bind the linked cubin, not the first package bundle.
    let linked = cuda_host::load_all_ptx_bundles_merged(pool.context())?;
    let module = unsafe { kernels::from_module(linked)? };
    let a = Buffer::from_host(pool.clone(), &case.a)?;
    let b = Buffer::from_host(pool.clone(), &case.b)?;
    let index = Buffer::from_host(pool.clone(), &case.index)?;
    let bits = Buffer::from_host(pool.clone(), &case.bits)?;
    let flags = unsafe { Buffer::<u8>::uninitialized(pool.clone(), 2*m)? };
    let grid = cq_gpu_runtime::launch_grid_for_kernel(module.prepare_bitmap_probe(LaunchConfig1D::new(1,256,0))?.function(), usize::MAX, 256, 0)?;
    let cfg = || LaunchConfig1D::new(grid, 256, 0);
    let out = Buffer::<u64>::zeroed(pool.clone(), grid as usize * 16)?;
    let searches = module.prepare_search_runs(cfg())?;
    let merge = module.prepare_merge_path(cfg())?;
    let bitmap = module.prepare_bitmap_probe(cfg())?;
    let heads = module.prepare_distinct_heads(cfg())?;
    let expand = module.prepare_expand_heads(cfg())?;
    let has = |name| method == "all" || method == name;
    let (nn, mm) = (n as u32, m as u32);
    let report = |runs: &str, name: &str, ms: f64, sort_ms: f64| -> Result<()> {
        let totals = out.to_host()?.chunks_exact(2).fold((0,0u64), |(c,s), x| (c+x[0],s.wrapping_add(x[1])));
        if totals != case.oracle { return Err(format!("{distribution} R={runs} {name}: GPU {totals:?} != CPU {:?}", case.oracle).into()); }
        println!("{n}\t{m}\t{distribution}\t{runs}\t{name}\t{ms:.6}\t{sort_ms:.6}\t{:.3}\t{:.3}\t{}\t{:016x}\tPASS", n as f64/ms/1e6, n as f64/(ms+sort_ms)/1e6, totals.0, totals.1);
        Ok(())
    };
    if has("runs") {
        for (r, label) in [(1,"1"),(4,"4"),(8,"8")] {
            if run_filter != "all" && run_filter != label { continue; }
            // Interleaving ranks across runs makes their key ranges overlap fully.
            let keys = if r == 1 { None } else {
                let sorted_runs: Vec<_> = (0..r).flat_map(|j| case.index.iter().skip(j).step_by(r).copied()).collect();
                Some(Buffer::from_host(pool.clone(), &sorted_runs)?)
            };
            let ptr = keys.as_ref().unwrap_or(&index).ptr();
            let ms = timed(pool, || Ok(()), || Ok(module.search_runs(stream,&searches,a.ptr(),b.ptr(),nn,ptr,mm/r as u32,r as u32,out.ptr())?))?;
            report(label,"runs",ms,0.)?;
        }
    }
    if has("merged") {
        let ms = timed(pool, || Ok(()), || Ok(module.search_runs(stream,&searches,a.ptr(),b.ptr(),nn,index.ptr(),mm,1,out.ptr())?))?;
        report("all","merged",ms,0.)?;
    }
    if has("bitmap") {
        let ms = timed(pool, || Ok(()), || Ok(module.bitmap_probe(stream,&bitmap,a.ptr(),b.ptr(),nn,bits.ptr(),out.ptr())?))?;
        report("all","bitmap",ms,0.)?;
    }
    if has("ascending") || has("mergepath") || has("distinct") {
        let cub = rccl_cub::cuda::Program::load(pool.context())?;
        let on = cub.on(stream)?;
        let mut scratch = rccl_cub::device_radix_sort_pairs_buffer_workspace_in::<u64,u64,_>(pool.resource(), &on, n)?;
        let column = || -> Result<_> { Ok(SortColumn(unsafe { Buffer::uninitialized(pool.clone(), n)? })) };
        let (mut a1, mut a2, mut b1, mut b2) = (column()?,column()?,column()?,column()?);
        let (dst_a,dst_b) = (a1.0.cu_deviceptr(),b1.0.cu_deviceptr());
        let (mut sorted_a,mut sorted_b) = (a1.0.ptr(), b1.0.ptr());
        let sort_ms = timed(pool, || {
            // Restore original unsorted inputs BETWEEN measurements (sort mutates its inputs).
            unsafe {
                cuda_core::simt::memory::memcpy_dtod_async(dst_a,a.cu_deviceptr(),n*8,stream.cu_stream())?;
                cuda_core::simt::memory::memcpy_dtod_async(dst_b,b.cu_deviceptr(),n*8,stream.cu_stream())?;
            }
            Ok(())
        }, || {
            let mut double = rccl_cub::DoubleBuffer::paired((&mut b1,&mut a1),(&mut b2,&mut a2)).ok_or("unequal sort buffers")?;
            rccl_cub::device_radix_sort_pairs_buffer(&on, &mut double, 0..(2*m).ilog2(), &mut scratch)?;
            sorted_b = double.current().0.0.ptr(); sorted_a = double.current().1.0.ptr();
            Ok(())
        })?;
        if has("ascending") {
            let ms = timed(pool, || Ok(()), || Ok(module.search_runs(stream,&searches,sorted_a,sorted_b,nn,index.ptr(),mm,1,out.ptr())?))?;
            report("all","ascending",ms,sort_ms)?;
        }
        if has("mergepath") {
            let ms = timed(pool, || Ok(()), || Ok(module.merge_path(stream,&merge,sorted_a,sorted_b,nn,index.ptr(),mm,out.ptr())?))?;
            report("all","mergepath",ms,sort_ms)?;
        }
        if has("distinct") {
            let ms = timed(pool, || Ok(()), || {
                module.distinct_heads(stream,&heads,sorted_b,nn,index.ptr(),mm,flags.ptr())?;
                module.expand_heads(stream,&expand,sorted_a,sorted_b,nn,flags.ptr(),out.ptr())?;
                Ok(())
            })?;
            report("all","distinct",ms,sort_ms)?;
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let arg = |i| args.get(i).map(String::as_str).unwrap_or("all");
    let (size,dist,r,method) = (arg(0),arg(1),arg(2),arg(3));
    let ds = ["uniform","zipf","hot","miss","hit30"];
    if args.len()>4 || !["all","small","large"].contains(&size) || (dist!="all" && !ds.contains(&dist)) || !["all","1","4","8"].contains(&r) || !["all","runs","merged","bitmap","ascending","mergepath","distinct"].contains(&method) {
        return Err("usage: probe [small|large|all] [uniform|zipf|hot|miss|hit30|all] [1|4|8|all] [runs|merged|ascending|mergepath|distinct|bitmap|all]".into());
    }
    let pool = gpu_datalog_runtime::thread_pool()?;
    println!("rows\tkeys\tdist\tR\tmethod\tprobe_ms\tsort_ms\tprobe_Grows/s\tincl_sort_Grows/s\thits\tchecksum\toracle");
    for (s,n,m) in [("small",1<<20,1<<17),("large",1<<23,1<<22)] {
        if size != "all" && s != size { continue; }
        for d in ds { if dist == "all" || d == dist { one(&pool,n,m,d,r,method)?; } }
    }
    Ok(())
}
