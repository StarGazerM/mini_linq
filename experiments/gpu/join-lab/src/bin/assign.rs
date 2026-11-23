//! `assign H118_ROUND H119_ROUND [cpu|all|118|119|hybrid118|hybrid119]`.
use std::{fs::File,io::Read,path::Path,sync::Arc};
use cuda_core::LaunchConfig1D;
use gpu_datalog_runtime::{Buffer,DeviceRadixRun,Pool,RadixRunParams,Result};
use join_lab::assign::{AsIndex,AsWork,assign_kernels};

fn pairs(path:&Path,file:usize)->Result<Vec<[u32;2]>>{
    let mut f=File::open(path.join(format!("{file}.u32")))?;let n=usize::try_from(f.metadata()?.len())?;
    if !cfg!(target_endian="little") || n%8!=0{return Err("invalid capture encoding".into());}
    let mut v=vec![[0;2];n/8];f.read_exact(unsafe{std::slice::from_raw_parts_mut(v.as_mut_ptr().cast(),n)})?;Ok(v)
}
fn pack(kind:u32,g:u32,v:u32)->u64{let(h,t)=if kind==118{(v,g)}else{(g,v)};(h as u64)<<32|t as u64}
fn pct(v:&mut [usize])->[usize;4]{v.sort_unstable();[v[v.len()/2],v[(v.len()-1)*99/100],v[(v.len()-1)*999/1000],v[v.len()-1]]}
struct Case{kind:u32,round:u32,raw:u32,root:Vec<[u32;2]>,off:Vec<u32>,cols:Vec<u32>,decode:Vec<u32>,bit_row:Vec<u32>,dense:Vec<u32>,max_g:u32,tasks:u32,unique:Vec<u64>}
impl Case{
    fn read(path:&Path,kind:u32)->Result<Self>{
        let man:serde_json::Value=serde_json::from_reader(File::open(path.join("manifest.json"))?)?;
        let labels=if kind==118{["delta assign","varpointsto[1, 0]"]}else{["delta varpointsto","assign[0, 1]"]};
        if man["hole"]!=kind||man["reads"]!=serde_json::json!(labels){return Err(format!("incorrect hole {kind} inputs: {}",man["reads"]).into());}
        let round=man["iteration"].as_u64().ok_or("round")?.try_into()?;let raw=man["candidates"].as_u64().ok_or("count")?.try_into()?;
        let root=pairs(path,0)?;let rhs=pairs(path,1)?;assert!(root.is_sorted()&&rhs.is_sorted());
        let f=usize::from(kind==119);let g=1-f;let max_g=root.iter().map(|x|x[g]).max().unwrap();
        let max_from=rhs.last().unwrap()[0].max(root.iter().map(|p|p[f]).max().unwrap());let max_child=rhs.iter().map(|r|r[1]).max().unwrap();
        let mut ranks=vec![u32::MAX;max_child as usize+1];let mut off=vec![0;max_from as usize+2];
        for r in &rhs{off[r[0] as usize+1]+=1;ranks[r[1] as usize]=0;}
        let mut total=0;for v in &mut off{total+=*v;*v=total;}
        let mut decode=Vec::new();for (i,v) in ranks.iter_mut().enumerate(){if *v==0{*v=decode.len() as u32;decode.push(i as u32);}}
        let cols:Vec<_>=rhs.iter().map(|r|ranks[r[1] as usize]).collect();drop(rhs);drop(ranks);
        let words=decode.len().div_ceil(32);let mut bit_row=vec![u32::MAX;off.len()-1];let mut dense=Vec::new();
        for i in 0..bit_row.len(){if (off[i+1]-off[i]) as usize>=words{bit_row[i]=dense.len() as u32;dense.push(i as u32);}}
        let mut bitmaps=vec![0u32;words*dense.len()];
        for (j,&id) in dense.iter().enumerate(){let bits=&mut bitmaps[j*words..(j+1)*words];for &v in &cols[off[id as usize] as usize..off[id as usize+1] as usize]{bits[v as usize/32]|=1<<(v%32);}}
        // Oracle groups independently on CPU using raw relation fields. Empty
        // ranges carry no witnesses; report them separately, not as fanout.
        let mut grouped=root.clone();grouped.sort_unstable_by_key(|p|(p[g],p[f]));
        let(mut num,mut empty,mut ks,mut fan,mut tasks,mut split,mut partial_bytes,mut dense_edges)=(0u64,0,Vec::new(),Vec::new(),0usize,0usize,0usize,0usize);
        let mut unique=Vec::new();let mut bitmap=vec![0u32;words];let mut touched=Vec::new();let mut top=Vec::new();
        let mut patterns=std::collections::HashMap::<Vec<u32>,usize>::new();let mut pattern=Vec::new();
        let mut seen=vec![false;bit_row.len()];let(mut source_children,mut pattern_children)=(0usize,0usize);
        for gr in grouped.chunk_by(|x,y|x[g]==y[g]){let key=gr[0][g];let(mut k,mut ch)=(0usize,0usize);
            for p in gr{let from=p[f] as usize;let lo=off[from] as usize;let hi=off[from+1] as usize;if lo==hi{empty+=1;continue;}k+=1;ch+=hi-lo;
                pattern.push(from as u32);if !std::mem::replace(&mut seen[from],true){source_children+=hi-lo;}
                let row=bit_row[from];if row!=u32::MAX {dense_edges+=1;for (w,&v) in bitmaps[row as usize*words..(row as usize+1)*words].iter().enumerate(){if v!=0{if bitmap[w]==0{touched.push(w);}bitmap[w]|=v;}}}
                else{for &v in &cols[lo..hi]{let w=v as usize/32;if bitmap[w]==0{touched.push(w);}bitmap[w]|=1<<(v%32);}}
            }
            let begin=unique.len();for w in touched.drain(..){let mut x=std::mem::take(&mut bitmap[w]);while x!=0{let h=32*w+x.trailing_zeros() as usize;unique.push(pack(kind,key,decode[h]));x&=x-1;}}
            if k!=0 {ks.push(k);fan.push(ch);num+=ch as u64;top.push((ch,k,unique.len()-begin,key));if k>1{let n=k.div_ceil(256);tasks+=n;if n>1{split+=1;partial_bytes+=n*words*4;}}}
            if !pattern.is_empty(){if let Some(n)=patterns.get_mut(&pattern){*n+=1;pattern.clear();}else{patterns.insert(std::mem::take(&mut pattern),1);pattern_children+=ch;}}
        }
        assert_eq!(num,u64::from(raw));unique.sort_unstable();assert!(unique.windows(2).all(|p|p[0]<p[1]));top.sort_unstable_by(|a,b|b.cmp(a));
        println!("PATTERNS h{kind} round={round} unique_lists={} max_shares={} representative_children={pattern_children} resident_children_once={source_children}",patterns.len(),patterns.values().max().unwrap());
        let mut by_to=Vec::new();if kind==119{
            // Target-directed form uses delta heaps per from and resident incoming assign edges.
            let mut heap_count=vec![0usize;off.len()-1];for p in &root{heap_count[p[f] as usize]+=1;}
            let mut target_k=vec![0usize;decode.len()];let mut target_n=vec![0usize;decode.len()];
            for (from,&cnt) in heap_count.iter().enumerate(){if cnt>0{for &t in &cols[off[from] as usize..off[from+1] as usize]{target_k[t as usize]+=1;target_n[t as usize]+=cnt;}}}
            for (k,n) in target_k.into_iter().zip(target_n){if n!=0{by_to.push((k,n));}}
            let mut k:Vec<_>=by_to.iter().map(|x|x.0).collect();let mut n:Vec<_>=by_to.iter().map(|x|x.1).collect();assert_eq!(n.iter().sum::<usize>(),raw as usize);
            println!("TO_GROUPS h119 round={round} groups={} k_p50_p99_p999_max={:?} children_p50_p99_p999_max={:?}",k.len(),pct(&mut k),pct(&mut n));
        }
        println!("CASE h{kind} round={round} roots={} resident={} groups={} empty_edges={empty} raw={raw} unique={} k_p50_p99_p999_max={:?} children_p50_p99_p999_max={:?} top(ch,k,u,key)={:?} child_domain={} words={words} dense_ranges={} dense_edges={dense_edges} tile_ctas={tasks} split_groups={split} partial_write_bytes={partial_bytes}",root.len(),cols.len(),ks.len(),unique.len(),pct(&mut ks),pct(&mut fan),&top[..5.min(top.len())],decode.len(),dense.len());
        Ok(Self{kind,round,raw,root,off,cols,decode,bit_row,dense,max_g,tasks:tasks.try_into()?,unique})
    }
    fn old(&self,key:u64)->bool{let v=key as u32 as usize;if v+1>=self.off.len(){return false;}let Ok(h)=self.decode.binary_search(&((key>>32)as u32))else{return false};self.cols[self.off[v]as usize..self.off[v+1]as usize].binary_search(&(h as u32)).is_ok()}
}
fn overlap(a:&Case,b:&Case){assert_eq!(a.round,b.round);let(mut i,mut j,mut cross,mut old_union,mut both_old,mut old_a,mut old_b)=(0,0,0,0,0,0,0);
    while i<a.unique.len()||j<b.unique.len(){let x=a.unique.get(i).copied().unwrap_or(u64::MAX);let y=b.unique.get(j).copied().unwrap_or(u64::MAX);let v=x.min(y);let old=a.old(v) as usize;
        if x==v{i+=1;old_a+=old;}if y==v{j+=1;old_b+=old;}if x==y{cross+=1;both_old+=old;}old_union+=old;}
    let u=a.unique.len()+b.unique.len()-cross;println!("OVERLAP round={} old118={old_a} old119={old_b} cross={cross} cross_old={both_old} pair_unique={u} pair_old={old_union} pair_fresh={}",a.round,u-old_union);
}

struct Scan(Buffer<u32>);
unsafe impl cuda_host::KernelSliceArg for Scan{type Elem=u32;fn len(&self)->usize{self.0.len()}fn cu_deviceptr(&self)->u64{self.0.cu_deviceptr()}}
unsafe impl cuda_host::KernelSliceArgMut for Scan{}
struct Keys(Buffer<u64>);
impl rccl_cub::DeviceRange for Keys{fn len(&self)->usize{self.0.len()}}
unsafe impl cuda_host::KernelSliceArg for Keys{type Elem=u64;fn len(&self)->usize{self.0.len()}fn cu_deviceptr(&self)->u64{self.0.cu_deviceptr()}}
unsafe impl cuda_host::KernelSliceArgMut for Keys{}
fn timed(pool:&Pool,reps:usize,mut prep:impl FnMut()->Result<()>,mut op:impl FnMut()->Result<()>)->Result<f64>{
    prep()?;op()?;let flags=Some(cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT);let mut ts=Vec::new();for _ in 0..reps{prep()?;let a=pool.stream().record_event(flags)?;op()?;let b=pool.stream().record_event(flags)?;b.synchronize()?;ts.push(f64::from(a.elapsed_ms(&b)?));}ts.sort_by(f64::total_cmp);Ok(ts[ts.len()/2])}
fn gpu(pool:&Arc<Pool>,c:&Case,mode:&str)->Result<()> {
    let module=unsafe{assign_kernels::from_module(cuda_host::load_all_ptx_bundles_merged(pool.context())?)?};let stream=pool.stream();
    let up=|v:&[u32]|Buffer::from_host(pool.clone(),v);let alloc=|n|unsafe{Buffer::<u32>::uninitialized(pool.clone(),n)};
    let root=up(c.root.as_flattened())?;let off=up(&c.off)?;let cols=up(&c.cols)?;let decode=up(&c.decode)?;let bit_row=up(&c.bit_row)?;let dense=up(&c.dense)?;
    let words=c.decode.len().div_ceil(32);let bits=alloc((c.dense.len()*words).max(1))?;
    let x=AsIndex{root:root.ptr(),n:c.root.len().try_into()?,max_group:c.max_g,kind:c.kind,off:off.ptr(),cols:cols.ptr(),decode:decode.ptr(),domain:c.decode.len().try_into()?,bit_row:bit_row.ptr(),bits:bits.ptr(),words:words.try_into()?};
    let groups=std::cell::RefCell::new(Scan(alloc(c.max_g as usize+2)?));let cursor=alloc(c.max_g as usize+1)?;let edges=alloc(c.root.len())?;let parents=alloc(c.max_g as usize+1)?;
    let desc=unsafe{Buffer::<u64>::uninitialized(pool.clone(),c.max_g as usize+1)?};let partial=alloc((c.tasks as usize*words).max(1))?;let tiles=alloc((c.tasks as usize).max(1))?;
    let output=unsafe{Buffer::<u64>::uninitialized(pool.clone(),c.unique.len())?};let meta=alloc(4)?;
    let rep=alloc(c.max_g as usize+1)?;let sigtable=alloc(65536)?;
    let newkeys=||->Result<_>{Ok(Keys(unsafe{Buffer::uninitialized(pool.clone(),c.root.len())?}))};
    let(mut k1,mut k2)=(newkeys()?,newkeys()?);
    let mut w=AsWork{groups:groups.borrow().0.ptr(),cursor:cursor.ptr(),edges:edges.ptr(),parents:parents.ptr(),desc:desc.ptr(),partial:partial.ptr(),tiles:tiles.ptr(),tile_cap:c.tasks,output:output.ptr(),cap:c.unique.len().try_into()?,meta:meta.ptr(),sorted:k1.0.ptr(),rep:rep.ptr(),sigtable:sigtable.ptr(),sigmask:(sigtable.len()-1) as u32};
    let cub=rccl_cub::cuda::Program::load(pool.context())?;let on=cub.on(stream)?;let cfg=||LaunchConfig1D::new(852,256,0);
    let init=|ptr,len|->Result<()>{unsafe{cuda_core::simt::memory::memset_d8_async(ptr,0,len,stream.cu_stream())?;}Ok(())};
    let bit_kernel=module.prepare_as_bit_index(cfg())?;
    let bit_ms=timed(pool,5,||Ok(()),||{init(bits.cu_deviceptr(),bits.len()*4)?;module.as_bit_index(stream,&bit_kernel,x,dense.ptr(),c.dense.len() as u32)?;Ok(())})?;
    let shared=words as u32*4;if shared>98304{return Err(format!("{} bytes exceeds tile shared capacity",shared).into());}
    let group=module.prepare_as_group(cfg())?;let tasks=module.prepare_as_tasks(cfg())?;let tile=module.prepare_as_tile(LaunchConfig1D::new(852,256,shared))?;let finish=module.prepare_as_finish(cfg())?;
    let mut ws=rccl_cub::device_scan_workspace_in::<_,u32>(pool.resource(),&on,groups.borrow().0.len())?;
    let keys=module.prepare_as_keys(cfg())?;let intern=module.prepare_as_intern(cfg())?;
    let mut sort_ws=rccl_cub::device_radix_sort_keys_buffer_workspace_in::<u64,_>(pool.resource(),&on,c.root.len())?;
    for (name,hybrid,reuse) in [("csr",false,false),("hybrid",true,false),("reuse",false,true)]{
        if mode.starts_with("hybrid") && !hybrid || mode.starts_with("reuse") && !reuse{continue;}
        let mut schedule=||->Result<AsWork>{init(meta.cu_deviceptr(),16)?;init(groups.borrow().0.cu_deviceptr(),groups.borrow().0.len()*4)?;
            if reuse{init(sigtable.cu_deviceptr(),sigtable.len()*4)?;module.as_keys(stream,&keys,x,w,k1.0.ptr())?;
                let mut pair=rccl_cub::DoubleBuffer::new(&mut k1,&mut k2).ok_or("unequal keys")?;
                rccl_cub::device_radix_sort_keys_buffer(&on,&mut pair,0..64,&mut sort_ws)?;w.sorted=pair.current().0.ptr();}
            else{init(cursor.cu_deviceptr(),cursor.len()*4)?;module.as_group(stream,&group,x,w,false)?;}
            rccl_cub::device_exclusive_sum_in_place(&on,&mut *groups.borrow_mut(),&mut ws)?;
            if reuse{module.as_intern(stream,&intern,x,w)?;}else{module.as_group(stream,&group,x,w,true)?;}
            module.as_tasks(stream,&tasks,x,w,256,reuse)?;Ok(w)};
        let schedule_ms=timed(pool,5,||Ok(()),||schedule().map(|_|()))?;
        let ms=timed(pool,5,||Ok(()),||{let launch=schedule()?;module.as_tile(stream,&tile,x,launch,256,hybrid,reuse)?;module.as_finish(stream,&finish,x,launch,reuse)?;Ok(())})?;
        let counts=meta.to_host()?;assert_eq!(counts[0] as usize,c.unique.len());if !reuse{assert_eq!(counts[2],c.tasks);}assert_eq!(counts[3],0);
        let mut actual=output.to_host()?;actual.sort_unstable();assert!(actual==c.unique,"entire distinct candidate set mismatch");
        let pubstate=DeviceRadixRun::<u64,u64,()>::new_set(pool.clone(),RadixRunParams{ratio:4,radix_bits:0,merge_tile_rows:32,block_threads:256},64)?;
        let published=std::cell::RefCell::new(None);
        let pub_ms=timed(pool,3,||{published.borrow_mut().take();Ok(())},||{*published.borrow_mut()=Some(unsafe{pubstate.sort_known_unique(&output,c.unique.len())?});Ok(())})?;
        assert!(published.into_inner().unwrap().buffer().to_host()?==c.unique,"sorted grouped output mismatch");
        println!("GPU h{} round={} method={name} join_ms={ms:.4} schedule_ms={schedule_ms:.4} known_unique_sort_ms={pub_ms:.4} bitmap_build_ms={bit_ms:.4} bitmap_bytes={} partial_alloc_bytes={} actual_tiles={} unique={} PASS",c.kind,c.round,bits.len()*4,partial.len()*4,counts[2],counts[0]);
    }
    if mode.starts_with("hybrid")||mode.starts_with("reuse"){return Ok(());}
    let mut offsets=Scan(alloc(c.root.len()+1)?);let mut os=rccl_cub::device_scan_workspace_in::<_,u32>(pool.resource(),&on,offsets.0.len())?;let lengths=module.prepare_as_lengths(cfg())?;
    let pre=timed(pool,3,||Ok(()),||{init(offsets.0.cu_deviceptr()+4*c.root.len() as u64,4)?;module.as_lengths(stream,&lengths,x,offsets.0.ptr())?;rccl_cub::device_exclusive_sum_in_place(&on,&mut offsets,&mut os)?;Ok(())})?;
    let chunk=(1usize<<28)-(1<<20);let bag=unsafe{Buffer::<u64>::uninitialized(pool.clone(),chunk.min(c.raw as usize))?};let rk=module.prepare_as_raw(cfg())?;
    let emit=|first:usize,n:usize|->Result<()>{module.as_raw(stream,&rk,x,offsets.0.ptr(),first.try_into()?,n.try_into()?,bag.ptr())?;Ok(())};
    let emit_ms=timed(pool,3,||Ok(()),||{for first in (0..c.raw as usize).step_by(chunk){emit(first,(c.raw as usize-first).min(chunk))?;}Ok(())})?;
    // Production-sized bounded RCCL portions. Accumulate their exact union,
    // including subtraction against earlier portions, not just local unique.
    let state=std::cell::RefCell::new(None);let mut selected=Vec::new();
    let whole=timed(pool,3,||{*state.borrow_mut()=Some(DeviceRadixRun::<u64,u64,()>::new_set(pool.clone(),RadixRunParams{ratio:4,radix_bits:0,merge_tile_rows:32,block_threads:256},64)?);Ok(())},||{
        selected.clear();let mut slot=state.borrow_mut();let set=slot.as_mut().unwrap();for first in (0..c.raw as usize).step_by(chunk){let n=(c.raw as usize-first).min(chunk);emit(first,n)?;let new=unsafe{set.fresh(&bag,n)?};selected.push(new.clone());unsafe{set.append_sorted(new)?;}}Ok(())})?;
    let mut actual=Vec::new();for part in selected {actual.extend(part.buffer().to_host()?);}actual.sort_unstable();assert!(actual==c.unique,"raw sort-unique set mismatch");
    println!("RAW h{} round={} root_scan_ms={pre:.4} raw_emit_ms={emit_ms:.4} emit_sort_union_ms={whole:.4} raw={} unique={} PASS",c.kind,c.round,c.raw,actual.len());Ok(())
}
fn main()->Result<()>{let v:Vec<_>=std::env::args().skip(1).collect();if !(2..=3).contains(&v.len()){return Err("assign H118_ROUND H119_ROUND [all|cpu|118|119|hybrid118|hybrid119|reuse|reuse118|reuse119]".into());}
    let mode=v.get(2).map(String::as_str).unwrap_or("all");if !["all","cpu","118","119","hybrid118","hybrid119","reuse","reuse118","reuse119"].contains(&mode){return Err("unknown mode".into());}
    let a=Case::read(Path::new(&v[0]),118)?;let b=Case::read(Path::new(&v[1]),119)?;overlap(&a,&b);
    if mode!="cpu"{let pool=gpu_datalog_runtime::thread_pool()?;for c in [&a,&b]{if mode=="all"||mode=="reuse"||mode.ends_with(&c.kind.to_string()){gpu(&pool,c,mode)?;}}}Ok(())}
