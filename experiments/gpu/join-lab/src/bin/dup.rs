//! `dup CAPTURE_ROUND [raw|group|hash|subtract|hash-subtract|heap|union|all|cpu]`.
//! Oracle accounting uses raw symbols; the GPU never receives the oracle set or witness lists.
use cuda_core::LaunchConfig1D;
use gpu_datalog_runtime::{Buffer, Pool, Result};
use join_lab::dup::{DupIndex, DupWork, dup_kernels};
use std::{fs::File, io::Read, path::Path, sync::Arc};

fn rows<const N: usize>(dir: &Path, input: usize) -> Result<Vec<[u32; N]>> {
    let mut f=File::open(dir.join(format!("{input}.u32")))?;
    let bytes=usize::try_from(f.metadata()?.len())?;
    if !cfg!(target_endian="little") || bytes%(4*N)!=0 {return Err("invalid capture encoding".into());}
    let mut r=vec![[0;N]; bytes/(4*N)];
    // SAFETY: all byte patterns are valid and r owns exactly `bytes` bytes.
    f.read_exact(unsafe{std::slice::from_raw_parts_mut(r.as_mut_ptr().cast(),bytes)})?; Ok(r)
}
fn offsets(n:usize, keys: impl Iterator<Item=u32>) -> Vec<u32> {
    let mut off=vec![0;n+1];for k in keys {off[k as usize+1]+=1;}
    let mut total=0;for v in &mut off {total+=*v;*v=total;}off
}
fn range(off:&[u32], key:u32)->std::ops::Range<usize> {off[key as usize] as usize..off[key as usize+1] as usize}
fn pack(h:u32,to:u32)->u64 {(u64::from(h)<<32)|u64::from(to)}

struct Case {
    delta:Vec<[u32;3]>, reach:Vec<u32>, vp_off:Vec<u32>, vars:Vec<u32>,
    sig_min:u32, sig_off:Vec<u32>, loads:Vec<[u32;3]>, decode:Vec<u32>,
    raw:usize, grouped:usize, sub:usize, scratch:usize, nseg:usize, ngroup:usize, unique:Vec<u64>, fresh:Vec<u64>,
}
impl Case {
    fn new(dir:&Path)->Result<Self> {
        let manifest:serde_json::Value=serde_json::from_reader(File::open(dir.join("manifest.json"))?)?;
        if manifest["hole"]!=132 || manifest["reads"]!=serde_json::json!(["delta instancefieldpointsto","reachable[0]","varpointsto[0, 1]","loadinstancefield[1, 0, 2, 3]"]) {return Err("not the hole 132 capture layout".into());}
        let delta=rows::<3>(dir,0)?;let methods=rows::<1>(dir,1)?;let vp=rows::<2>(dir,2)?;let mut ld=rows::<4>(dir,3)?;
        assert!(delta.is_sorted() && vp.is_sorted());
        let heapmax=vp.last().unwrap()[0].max(delta.iter().map(|r|r[0].max(r[2])).max().unwrap());
        let vp_off=offsets(heapmax as usize+1,vp.iter().map(|r|r[0]));let vars:Vec<_>=vp.iter().map(|r|r[1]).collect();drop(vp);
        let contains=|h,v|vars[range(&vp_off,h)].binary_search(&v).is_ok();
        let sig_min=ld.iter().map(|r|r[0]).chain(delta.iter().map(|r|r[1])).min().unwrap();
        let sig_max=ld.iter().map(|r|r[0]).chain(delta.iter().map(|r|r[1])).max().unwrap();
        let sig_off=offsets((sig_max-sig_min+1) as usize,ld.iter().map(|r|r[0]-sig_min));
        let max_m=ld.iter().map(|r|r[3]).chain(methods.iter().map(|r|r[0])).max().unwrap();
        let mut reach=vec![0;max_m as usize/32+1];for [m] in methods {reach[m as usize/32]|=1<<(m%32);}
        let active=|m:u32|reach[m as usize/32]&(1<<(m%32))!=0;
        let mut decode:Vec<_>=ld.iter().map(|r|r[2]).collect();decode.sort_unstable();decode.dedup();
        // Keep all witnesses, including different methods of the same (target,base).
        ld.sort_unstable_by_key(|&[s,b,t,m]|(s,t,b,m));
        let loads=ld.iter().map(|&[_,b,t,m]|[decode.binary_search(&t).unwrap() as u32,b,m]).collect();
        let mut by_group=delta.clone();by_group.sort_unstable_by_key(|&[h,s,b]|(s,b,h));
        let (mut raw,mut repeated,mut samebase,mut multibase,mut scratch,mut raw_loads,mut ngroup,mut tcount,mut reach_probes)=(0usize,0,0,0,0,0,0,0,0);
        let mut emitted=Vec::new(); let mut repeat_rows=0;
        for group in by_group.chunk_by(|a,b|a[1..]==b[1..]) {
            ngroup+=1;let [_,sig,bh]=group[0];let ls=&ld[range(&sig_off,sig-sig_min)];scratch+=ls.len();raw_loads+=ls.len()*group.len();
            let heaps:Vec<_>=group.chunk_by(|a,b|a[0]==b[0]).map(|r|r[0][0]).collect();repeat_rows+=group.len()-heaps.len();
            for target in ls.chunk_by(|a,b|a[2]==b[2]) {let mut witnesses=0;let mut bases=0;
                for base in target.chunk_by(|a,b|a[1]==b[1]) {let active_count=base.iter().filter(|r|active(r[3])).count();
                    reach_probes+=active_count; if active_count>0 && contains(bh,base[0][1]) {bases+=1;witnesses+=active_count;}
                }
                if witnesses>0 {raw+=group.len()*witnesses;repeated+=(group.len()-heaps.len())*witnesses;
                    samebase+=heaps.len()*(witnesses-bases);multibase+=heaps.len()*(bases-1);tcount+=1;
                    emitted.extend(heaps.iter().map(|&h|pack(h,target[0][2])));
                }
            }
        }
        assert_eq!(raw as u64,manifest["candidates"].as_u64().unwrap());let grouped=emitted.len();emitted.sort_unstable();
        let (mut sub,mut unique,mut fresh)=(0,Vec::new(),Vec::new());
        for equal in emitted.chunk_by(|a,b|a==b) {let k=equal[0];unique.push(k);if !contains((k>>32) as u32,k as u32){sub+=equal.len();fresh.push(k);}}
        let cross=grouped-unique.len();assert_eq!(raw,repeated+samebase+multibase+cross+unique.len());
        let nheap=delta.chunk_by(|a,b|a[0]==b[0]).count();
        let(mut nseg,mut word_reads,mut union_loads)=(0,0,0);
        for seg in delta.chunk_by(|a,b|a[..2]==b[..2]) {let len=range(&sig_off,seg[0][1]-sig_min).len();nseg+=1;word_reads+=len.div_ceil(32)*seg.len();union_loads+=len;}
        println!("WORK heap_sig_segments={nseg} union_word_reads={word_reads} union_loads={union_loads}");
        println!("ORACLE delta={} heaps={} groups={} repeated_delta_rows={repeat_rows} raw_loads={raw_loads} group_loads={scratch} reachable_group_loads={reach_probes} group_targets={tcount} raw={raw} repeated={repeated} within_base={samebase} across_base={multibase} grouped={grouped} across_group={cross} unique={} old={} fresh={} sub={sub} vp_rows={} vp_dir_bytes={} targets={} max_heap={} max_var={} max_heap_range={}",delta.len(),nheap,ngroup,unique.len(),unique.len()-fresh.len(),fresh.len(),vars.len(),vp_off.len()*4,decode.len(),heapmax,vars.iter().max().unwrap(),vp_off.windows(2).map(|p|p[1]-p[0]).max().unwrap());
        // The real captures are set deltas. The accounting above also detects a bad duplicate delta.
        assert_eq!(repeat_rows,0,"GPU replay requires canonical delta rows");
        Ok(Self{delta,reach,vp_off,vars,sig_min,sig_off,loads,decode,raw,grouped,sub,scratch,nseg,ngroup,unique,fresh})
    }
}
struct Col(Buffer<u64>);
impl rccl_cub::DeviceRange for Col { fn len(&self)->usize {self.0.len()} }
// SAFETY: owns the full mutable key buffer throughout each sort.
unsafe impl cuda_host::KernelSliceArg for Col {type Elem=u64;fn cu_deviceptr(&self)->u64{self.0.cu_deviceptr()}fn len(&self)->usize{self.0.len()}}
unsafe impl cuda_host::KernelSliceArgMut for Col {}
fn timed(pool:&Pool,mut prep:impl FnMut()->Result<()>,mut launch:impl FnMut()->Result<()>)->Result<f64> {
    prep()?;launch()?; let flags=Some(cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT);let mut samples=[0.;5];
    for s in &mut samples {prep()?;let a=pool.stream().record_event(flags)?;launch()?;let b=pool.stream().record_event(flags)?;b.synchronize()?;*s=f64::from(a.elapsed_ms(&b)?);}
    samples.sort_by(f64::total_cmp);Ok(samples[2])
}
fn sort_cost(pool:&Arc<Pool>,out:&Buffer<u64>,n:usize)->Result<f64> {
    if n<2{return Ok(0.);}
    let cub=rccl_cub::cuda::Program::load(pool.context())?;let on=cub.on(pool.stream())?;
    let mut ws=rccl_cub::device_radix_sort_keys_buffer_workspace_in::<u64,_>(pool.resource(),&on,n)?;
    let col=||->Result<_>{Ok(Col(unsafe{Buffer::uninitialized(pool.clone(),n)?}))};let(mut a,mut b)=(col()?,col()?);let dst=a.0.cu_deviceptr();
    timed(pool,||{unsafe{cuda_core::simt::memory::memcpy_dtod_async(dst,out.cu_deviceptr(),n*8,pool.stream().cu_stream())?;}Ok(())},||{
        let mut pair=rccl_cub::DoubleBuffer::new(&mut a,&mut b).ok_or("unequal key buffers")?;
        rccl_cub::device_radix_sort_keys_buffer(&on,&mut pair,0..64,&mut ws)?;Ok(())})
}
fn gpu(pool:&Arc<Pool>,c:&Case,filter:&str)->Result<()> {
    let module=unsafe{dup_kernels::from_module(cuda_host::load_all_ptx_bundles_merged(pool.context())?)?};let stream=pool.stream();
    let up=|v:&[u32]|Buffer::from_host(pool.clone(),v);
    let delta=up(c.delta.as_flattened())?;let loads=up(c.loads.as_flattened())?;let sig=up(&c.sig_off)?;let reach=up(&c.reach)?;let vo=up(&c.vp_off)?;let vv=up(&c.vars)?;let decode=up(&c.decode)?;
    let x=DupIndex{delta:delta.ptr(),n:c.delta.len().try_into()?,loads:loads.ptr(),sig_off:sig.ptr(),sig_min:c.sig_min,reachable:reach.ptr(),vp_off:vo.ptr(),vp_vars:vv.ptr(),decode:decode.ptr(),targets:c.decode.len().try_into()?};
    let alloc32=|n|unsafe{Buffer::<u32>::uninitialized(pool.clone(),n)};let alloc64=|n|unsafe{Buffer::<u64>::uninitialized(pool.clone(),n)};
    let group=alloc64((2*c.ngroup).next_power_of_two())?;let slot=alloc32(c.delta.len())?;let ranges=alloc64(group.len())?;let ts=alloc32(c.scratch.max(1))?;
    let heaps=alloc64(c.nseg)?;let seen=alloc64((2*c.unique.len()).max(1).next_power_of_two())?;let out=alloc64(c.raw)?;let meta=alloc32(4)?;
    let w=DupWork{groups:group.ptr(),group_mask:(group.len()-1).try_into()?,slot:slot.ptr(),t_range:ranges.ptr(),t:ts.ptr(),t_cap:ts.len().try_into()?,heaps:heaps.ptr(),heap_cap:heaps.len().try_into()?,seen:seen.ptr(),seen_mask:(seen.len()-1).try_into()?,out:out.ptr(),out_cap:out.len().try_into()?,meta:meta.ptr()};
    let clear=|ptr,sz|->Result<()> {unsafe{cuda_core::simt::memory::memset_d8_async(ptr,0,sz,stream.cu_stream())?;}Ok(())};
    let cfg=||LaunchConfig1D::new(852,256,0);let raw=module.prepare_dup_raw(cfg())?;let groups=module.prepare_dup_index_groups(cfg())?;let targets=module.prepare_dup_targets(cfg())?;let emit=module.prepare_dup_emit(cfg())?;
    let shared=x.targets.div_ceil(32)*4;if shared>32768 {return Err("load target dictionary exceeds heap tile shared-memory contract".into());}
    let tile=module.prepare_dup_heap_tile(LaunchConfig1D::new(852,256,shared))?;
    let union=module.prepare_dup_union(cfg())?;
    for (method,mode,n,expected) in [("raw",0,c.raw,&c.unique),("group",0,c.grouped,&c.unique),("hash",1,c.unique.len(),&c.unique),("subtract",2,c.sub,&c.fresh),("hash-subtract",3,c.fresh.len(),&c.fresh),("heap",0,c.fresh.len(),&c.fresh),("union",3,c.fresh.len(),&c.fresh)] {
        if filter!="all" && filter!=method {continue;}
        let is_group=method!="raw";let is_heap=method=="heap";let is_union=method=="union";
        let init=||->Result<()> {clear(meta.cu_deviceptr(),16)?;if is_group {clear(group.cu_deviceptr(),group.len()*8)?;}if mode&1!=0 {clear(seen.cu_deviceptr(),seen.len()*8)?;}Ok(())};
        let index=||->Result<()> {module.dup_index_groups(stream,&groups,x,w,if is_union{2}else{u32::from(is_heap)})?;Ok(())};
        let lists=||->Result<()> {module.dup_targets(stream,&targets,x,w,is_union)?;Ok(())};
        let generate=||->Result<()> {if is_union {module.dup_union(stream,&union,x,w)?;}else if is_heap {module.dup_heap_tile(stream,&tile,x,w)?;}else if is_group {module.dup_emit(stream,&emit,x,w,mode)?;}else{module.dup_raw(stream,&raw,x,w)?;}Ok(())};
        let ms=timed(pool,||Ok(()),||{init()?;if is_group{index()?;lists()?;}generate()})?;
        let counts=meta.to_host()?;assert_eq!(counts[3],0,"{method} overflow");assert_eq!(counts[0] as usize,n,"{method} rows");
        let mut actual=vec![0u64;n];unsafe {cuda_core::simt::memory::memcpy_dtoh_async(actual.as_mut_ptr(),out.cu_deviceptr(),n*8,stream.cu_stream())?;}stream.synchronize()?;
        actual.sort_unstable();actual.dedup();assert!(actual==*expected,"{method}: full output set mismatch");
        if mode&2==0 && !is_heap {actual.retain(|&k|c.vars[range(&c.vp_off,(k>>32) as u32)].binary_search(&(k as u32)).is_err());assert!(actual==c.fresh,"{method}: fresh set mismatch");}
        let sort=sort_cost(pool,&out,n)?;
        // Stage medians are diagnostic, not additive: each has its own warm cache state.
        let index_ms=if is_group {timed(pool,init,index)?}else{0.};
        let list_ms=if is_group {timed(pool,||clear(meta.cu_deviceptr()+4,4),lists)?}else{0.};
        let emit_ms=timed(pool,||{clear(meta.cu_deviceptr(),4)?;if mode&1!=0 {clear(seen.cu_deviceptr(),seen.len()*8)?;}Ok(())},generate)?;
        println!("TIMING {method} ms={ms:.4} rows={n} sort64_ms={sort:.4} index_ms={index_ms:.4} targets_ms={list_ms:.4} emit_ms={emit_ms:.4} seen_bytes={} PASS",if mode&1!=0{seen.len()*8}else{0});
    } Ok(())
}
fn main()->Result<()> {
    let mut args=std::env::args().skip(1);let round=args.next().ok_or("dup CAPTURE_ROUND [all|cpu|raw|group|hash|subtract|hash-subtract|heap|union]")?;let method=args.next().unwrap_or_else(||"all".into());
    if args.next().is_some() || !["all","cpu","raw","group","hash","subtract","hash-subtract","heap","union"].contains(&method.as_str()){return Err("invalid replay method".into());}
    println!("ROUND {round}");let c=Case::new(Path::new(&round))?;if method!="cpu"{gpu(&gpu_datalog_runtime::thread_pool()?,&c,&method)?;}Ok(())
}
