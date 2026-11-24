//! `maint <scala capture dir> [1|3]`: replay sorted admission at round 186.
use cuda_core::{DeviceBuffer as D, LaunchConfig1D};
use gpu_datalog_runtime::{Buffer, Pool, Result};
use join_lab::maint::{MaintRun, MaintBitmap, SHARED, TILE, maint_kernels as kernels};
use rccl_cub::{DeviceSliceRead, DeviceSliceWrite, DeviceSetOperationWorkspace, thrust_integration as cub};
use std::{fs::File, io::Read, path::Path, sync::Arc};

const COMMITTED: usize = 298_465_609;
const FRESH: usize = 5_928_628;
const UNIQUE: usize = 9_730_816;
const RUNS: [usize; 3] = [251_565_383, 41_215_005, 5_685_221];

fn pairs(path: &Path, n: usize) -> Result<Vec<u64>> {
    let mut f = File::open(path)?;
    if f.metadata()?.len() != n as u64 * 8 { return Err(format!("wrong length: {}", path.display()).into()); }
    let mut v = vec![0u64; n];
    // All bit patterns are valid u64s; the exclusive byte view does not escape.
    f.read_exact(unsafe { std::slice::from_raw_parts_mut(v.as_mut_ptr().cast(), 8*n) })?;
    for x in &mut v { *x = u64::from_le(*x).rotate_left(32); }
    if !v.windows(2).all(|w| w[0] < w[1]) { return Err("capture is not a sorted set".into()); }
    Ok(v)
}

fn time(pool: &Pool, mut op: impl FnMut() -> Result<()>) -> Result<f64> {
    op()?;
    let flags = Some(cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT);
    let mut ts = [0.; 9];
    for t in &mut ts {
        let a = pool.stream().record_event(flags)?; op()?;
        let b = pool.stream().record_event(flags)?; b.synchronize()?;
        *t = f64::from(a.elapsed_ms(&b)?);
    }
    ts.sort_by(f64::total_cmp); Ok(ts[4])
}

// The full committed set and its newest run are captured. The older two
// components lost their boundaries on download; split by proportional rank
// so they overlap in key range and retain their *actual* 186 row counts.
fn split(old: &[u64], newest: Vec<u64>) -> Result<Vec<Vec<u64>>> {
    let (mut a, mut b) = (Vec::with_capacity(RUNS[0]), Vec::with_capacity(RUNS[1]));
    let (mut i, mut acc) = (0, 0);
    for &x in old {
        if newest.get(i) == Some(&x) { i += 1; continue; }
        acc += RUNS[1];
        if acc >= RUNS[0]+RUNS[1] { acc -= RUNS[0]+RUNS[1]; b.push(x); } else { a.push(x); }
    }
    if i != newest.len() || a.len()!=RUNS[0] || b.len()!=RUNS[1] { return Err("wrong LSM partition".into()); }
    Ok(vec![a, b, newest])
}

fn checksum(xs: &[u64]) -> u64 {
    xs.iter().fold(0, |h,&v| h.wrapping_add((v ^ v.rotate_left(23)).wrapping_mul(0x9e3779b97f4a7c15)))
}

// A direct RCCL set difference for each sorted run. Preflight obtains the
// actual intermediate counts; replays use those same prefixes, thereby giving
// RCCL its best case (no per-stage allocations or D2H readbacks in the timing).
fn rccl(pool: &Pool, on: &rccl_cub::cuda::Launch<'_, '_>, input: &D<u64>, runs: &[D<u64>], expected: &[u64]) -> Result<f64> {
    let stream = pool.stream();
    let mut stages = Vec::with_capacity(runs.len());
    let mut temp = Vec::<D<u64>>::with_capacity(runs.len());
    for r in runs {
        let src = temp.last().unwrap_or(input);
        let w = on.set_workspace_counts::<u64,u64>(src.len(),r.len())?;
        if w.wide_indices { return Err("expected 32-bit set indices".into()); }
        let mut work = DeviceSetOperationWorkspace::__from_parts(D::<usize>::zeroed(stream,1)?,
            D::<u64>::zeroed(stream,w.tile_descriptors)?,D::<u64>::zeroed(stream,w.partitions)?,D::<u8>::zeroed(stream,w.virtual_shared_bytes)?);
        let mut dst = D::<u64>::zeroed(stream,src.len())?;
        on.set_keys::<i32,u64,_,cub::SetDifference,_,_,_,_>(src,r,&mut work,Some(&mut dst),u64::cmp)?;
        let len = work.__parts_mut::<i32>().0.to_host_vec(stream)?[0];
        if len>dst.len() { return Err("RCCL returned oversized difference".into()); }
        let (ptr, _, ctx) = dst.into_raw_parts();
        // `zeroed` owns a synchronous cuMemAlloc. Keep ownership intact, but
        // expose only the observed output prefix to the following operation.
        temp.push(unsafe { D::from_raw_parts(ptr,len,ctx) });
        stages.push(work);
    }
    let ms = time(pool, || {
        for (i,(r,w)) in runs.iter().zip(&mut stages).enumerate() {
            let (before,after) = temp.split_at_mut(i);
            on.set_keys::<i32,u64,_,cub::SetDifference,_,_,_,_>(before.last().unwrap_or(input),r,w,Some(&mut after[0]),u64::cmp)?;
        }
        Ok(())
    })?;
    if stages.last_mut().unwrap().__parts_mut::<i32>().0.to_host_vec(stream)?[0] != expected.len()
        || temp.last().unwrap().to_host_vec(stream)? != expected { return Err("RCCL sorted difference mismatch".into()); }
    Ok(ms)
}

const CODES: u32 = 1<<22;
const HEAPS: u32 = 65_536;
const PAGES: u32 = 3<<20;

struct Index { buf: Vec<D<u32>>, hash: bool }
impl Index {
    fn new(pool: &Pool, hash: bool) -> Result<Self> {
        let sizes = if hash { vec![1<<30] } else { vec![CODES as usize,(HEAPS/2048*CODES) as usize,PAGES as usize*64,3] };
        Ok(Self { buf: sizes.into_iter().map(|len|D::zeroed(pool.stream(),len)).collect::<std::result::Result<_,_>>()?,hash })
    }
    fn restore(&self, pool: &Pool, src: &Self) -> Result<()> {
        for (a,b) in self.buf.iter().zip(&src.buf) {
            unsafe { cuda_core::simt::memory::memcpy_dtod_async(a.cu_deviceptr(),b.cu_deviceptr(),b.len()*4,pool.stream().cu_stream())?; }
        }
        Ok(())
    }
    fn metadata(&self, pool: &Pool) -> Result<Vec<u32>> {
        if self.hash { return Ok(vec![]); }
        let m=self.buf[3].to_host_vec(pool.stream())?;
        if m[2]!=0 { return Err(format!("bitmap capacity exceeded: {m:?}").into()); }
        Ok(m)
    }
    fn admit(&self, pool: &Pool, module: &kernels::LoadedModule, rows: &D<u64>, flags: *mut u32) -> Result<()> {
        let s=pool.stream();
        let n=u32::try_from(rows.len())?;
        let cfg=||LaunchConfig1D::new(n.div_ceil(256),256,0);
        let ap=rows.cu_deviceptr() as *const u64;
        if self.hash {
            module.maint_hash_insert(s,&module.prepare_maint_hash_insert(cfg())?,ap,n,self.buf[0].cu_deviceptr() as _,(1<<29)-1,flags)?;
        } else {
            let t=MaintBitmap { heaps:self.buf[0].cu_deviceptr() as _,directory:self.buf[1].cu_deviceptr() as _,
                bits:self.buf[2].cu_deviceptr() as _,meta:self.buf[3].cu_deviceptr() as _,codes:CODES,heap_limit:HEAPS,page_limit:PAGES };
            module.maint_bitmap_heaps(s,&module.prepare_maint_bitmap_heaps(cfg())?,ap,n,t)?;
            module.maint_bitmap_pages(s,&module.prepare_maint_bitmap_pages(cfg())?,ap,n,t)?;
            module.maint_bitmap_test_set(s,&module.prepare_maint_bitmap_test_set(cfg())?,ap,n,t,flags)?;
        }
        Ok(())
    }
}

fn indexed(pool: &Arc<Pool>, module: &kernels::LoadedModule, old: &[u64], candidates: &[u64], expected: &[u64], hash: bool) -> Result<()> {
    if old.first()==Some(&0) || candidates.first()==Some(&0) { return Err("benchmark reserves zero packed word".into()); }
    let s=pool.stream();
    let base=Index::new(pool,hash)?;
    let dest=Index::new(pool,hash)?;
    let old=D::from_host(s,old)?;
    let a=D::from_host(s,candidates)?;
    let flags=unsafe { Buffer::<u32>::uninitialized(pool.clone(),UNIQUE)? };
    let out=unsafe { Buffer::<u64>::uninitialized(pool.clone(),UNIQUE)? };
    let mut count=D::<u64>::zeroed(s,1)?;
    let cuda=rccl_cub::cuda::Program::load(pool.context())?;
    let on=cuda.on(s)?;
    let mut work=cub::compaction_workspace_in(pool.resource(),&on,UNIQUE)?;
    let timing=Some(cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT);
    let t=s.record_event(timing)?; base.admit(pool,module,&old,std::ptr::null_mut())?;
    let e=s.record_event(timing)?; e.synchronize()?;
    println!("{} init: {:.3} ms, {:?}, reserved {} bytes",if hash {"hash"}else{"bitmap"},t.elapsed_ms(&e)?,
             base.metadata(pool)?,base.buf.iter().map(|b|b.len()*4).sum::<usize>());
    let mut marks=[0.;9]; let mut times=[0.;9];
    for i in 0..10 {
        dest.restore(pool,&base)?;
        let t=s.record_event(timing)?; dest.admit(pool,module,&a,flags.ptr())?; let m=s.record_event(timing)?;
        cub::device_select_flagged(&on,DeviceSliceRead::new(a.cu_deviceptr() as *const u64),DeviceSliceRead::new(flags.ptr()),
                DeviceSliceWrite::new(out.ptr()),&mut count,UNIQUE,&mut work)?;
        let e=s.record_event(timing)?; e.synchronize()?;
        if i!=0 { marks[i-1]=t.elapsed_ms(&m)?; times[i-1]=t.elapsed_ms(&e)?; }
    }
    let meta=dest.metadata(pool)?;
    if count.to_host_vec(s)?[0]!=FRESH as u64 || out.to_host_prefix(FRESH)?!=expected { return Err("indexed stable difference mismatch".into()); }
    dest.admit(pool,module,&a,flags.ptr())?;
    if flags.to_host_prefix(UNIQUE)?.iter().any(|&f|f!=0) {return Err("index lost prior admission".into());}
    marks.sort_by(f32::total_cmp);times.sort_by(f32::total_cmp);
    println!("-\t{}\t{:.6}\t{:.6}\t{}\t{:016x}\t{:?}",if hash {"hash"}else{"bitmap"},marks[4],times[4],FRESH,checksum(expected),meta);
    Ok(())
}

fn one(pool: &Arc<Pool>, module: &kernels::LoadedModule, root: &Path, k: usize, index: Option<bool>) -> Result<()> {
    let old = pairs(&root.join("varpointsto@3__h132/186/2.u32"),COMMITTED)?;
    let expected = pairs(&root.join("varpointsto@3__h136/187/0.u32"),FRESH)?;
    // Known committed keys selected uniformly by rank. Fresh keys are the
    // real next delta. Do not claim to know which old keys were reproposed.
    let hits = UNIQUE-FRESH;
    let mut candidates = Vec::with_capacity(UNIQUE);
    candidates.extend_from_slice(&expected);
    candidates.extend((0..hits).map(|i| old[i*old.len()/hits]));
    candidates.sort_unstable();
    if !candidates.windows(2).all(|s|s[0]<s[1]) { return Err("candidates are not unique".into()); }
    // Check the oracle with an independent sorted CPU difference, including
    // the case of a bad cross-round capture that reuses an old key as fresh.
    let (mut i,mut j)=(0,0);
    for &v in &candidates {
        while i<old.len() && old[i]<v { i+=1; }
        if old.get(i)!=Some(&v) { if expected.get(j)!=Some(&v) { return Err("CPU difference mismatch".into()); } j+=1; }
    }
    if j!=expected.len() { return Err("CPU fresh cardinality mismatch".into()); }
    if let Some(hash)=index {return indexed(pool,module,&old,&candidates,&expected,hash);}
    let host_runs = if k==1 { vec![old] } else { split(&old,pairs(&root.join("varpointsto@3__h136/186/0.u32"),RUNS[2])?)? };
    let stream=pool.stream();
    let a=D::from_host(stream,&candidates)?;
    let runs=host_runs.into_iter().map(|v|D::from_host(stream,&v)).collect::<std::result::Result<Vec<_>,_>>()?;
    let desc: Vec<_> = runs.iter().map(|r|MaintRun{ptr:r.cu_deviceptr() as _,n:r.len() as u32}).collect();
    let descriptors=Buffer::from_host(pool.clone(),&desc)?;
    let n=UNIQUE as u32;
    let flags=unsafe { Buffer::<u32>::uninitialized(pool.clone(),UNIQUE)? };
    let out=unsafe { Buffer::<u64>::uninitialized(pool.clone(),UNIQUE)? };
    let mut count=D::<u64>::zeroed(stream,1)?;
    let cuda=rccl_cub::cuda::Program::load(pool.context())?;
    let on=cuda.on(stream)?;
    let mut work=cub::compaction_workspace_in(pool.resource(),&on,UNIQUE)?;
    let ap=a.cu_deviceptr() as *const u64;
    let grid=|blocks| LaunchConfig1D::new(blocks,256,0);
    let binary=module.prepare_maint_binary(grid(n.div_ceil(256)))?;
    let mut cuts=Vec::new();
    for run in &desc {
        let tiles=(run.n+n).div_ceil(TILE);
        cuts.push((unsafe { Buffer::<u32>::uninitialized(pool.clone(),tiles as usize+1)? },
                   module.prepare_maint_cuts(grid((tiles+1).div_ceil(256)))?,
                   module.prepare_maint_merge(LaunchConfig1D::new(tiles,256,SHARED))?));
    }
    let width=n.div_ceil(128)+1;
    let windows=unsafe { Buffer::<u32>::uninitialized(pool.clone(),width as usize*k)? };
    let window=module.prepare_maint_windows(grid(width.div_ceil(256)))?;
    let bounded=module.prepare_maint_bounded(grid(n.div_ceil(256)))?;
    for method in ["binary","mergepath","bounded"] {
        let mark = || -> Result<()> {
            match method {
                "binary" => module.maint_binary(stream,&binary,ap,n,descriptors.ptr(),k as u32,flags.ptr())?,
                "mergepath" => for (i,(run,(cut,part,merge))) in desc.iter().zip(&cuts).enumerate() {
                    module.maint_cuts(stream,part,ap,n,*run,cut.ptr())?;
                    module.maint_merge(stream,merge,ap,n,*run,cut.ptr(),i==0,flags.ptr())?;
                },
                _ => { module.maint_windows(stream,&window,ap,n,descriptors.ptr(),k as u32,windows.ptr())?;
                       module.maint_bounded(stream,&bounded,ap,n,descriptors.ptr(),k as u32,windows.ptr(),flags.ptr())?; }
            }
            Ok(())
        };
        let mark_ms=time(pool,mark)?;
        let ms=time(pool, || {
            mark()?;
            Ok(cub::device_select_flagged(&on,DeviceSliceRead::new(ap),DeviceSliceRead::new(flags.ptr()),
                DeviceSliceWrite::new(out.ptr()),&mut count,UNIQUE,&mut work)?)
        })?;
        if count.to_host_vec(stream)?[0]!=FRESH as u64 || out.to_host_prefix(FRESH)?!=expected {
            return Err(format!("{method} sorted difference mismatch").into());
        }
        println!("{k}\t{method}\t{mark_ms:.6}\t{ms:.6}\t{}\t{:016x}",expected.len(),checksum(&expected));
    }
    println!("{k}\trccl-set-difference\t-\t{:.6}\t{}\t{:016x}",rccl(pool,&on,&a,&runs,&expected)?,expected.len(),checksum(&expected));
    Ok(())
}

fn main() -> Result<()> {
    let mut args=std::env::args_os().skip(1);
    let dir=args.next().ok_or("usage: maint <scala capture dir> [1|3] [bitmap|hash]")?;
    let k=args.next().map(|s| s.to_string_lossy().parse::<usize>()).transpose()?;
    if k.is_some_and(|n|n!=1&&n!=3) {return Err("expected 1 or 3 retained runs".into());}
    let index=match args.next().as_deref().and_then(|s|s.to_str()) {None=>None,Some("bitmap")=>Some(false),Some("hash")=>Some(true),_=>return Err("expected bitmap or hash".into())};
    let pool=gpu_datalog_runtime::thread_pool()?;
    let module=unsafe { kernels::from_module(cuda_host::load_all_ptx_bundles_merged(pool.context())?)? };
    println!("runs\tmethod\tmark_ms\tstable_difference_ms\tfresh\tchecksum");
    for r in [1,3] { if k.is_none_or(|n|n==r) { one(&pool,&module,Path::new(&dir),r,index)?; if index.is_some(){break;} } }
    Ok(())
}
