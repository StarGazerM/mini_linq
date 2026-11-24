//! `hashdedup bench [UNIQUE FACTOR scattered|adjacent]` or `hashdedup census TRACE`.
use cuda_core::LaunchConfig1D;
use gpu_datalog_runtime::{Buffer, Pool, Result};
use join_lab::hashdedup::{HD_BITS, HD_MULT, hd_kernels};
use rccl_cub::{DeviceSliceRead, DeviceSliceWrite};
use std::{fs::File, io::{BufRead, BufReader}, sync::Arc};

fn census(path: &str) -> Result<()> {
    // Only canonical portions and subtraction of their unique rows can occur
    // inside fresh_batches. Publication may reorder known-unique rows: ignore
    // those sorts. Before the first admission, ignore unrelated EDB setup.
    let mut steps = Vec::<(bool, u64, u64)>::new();
    let (mut admissions,mut completed)=(0usize,false);
    println!("stratum\tphase\tround\trelation\tcandidates\tunique\tfresh\tword_bytes");
    for line in BufReader::new(File::open(path)?).lines() {
        let line = line?;
        if line.starts_with("gpu_evaluation_seconds\t") { completed=true; }
        let field = |key: &str| -> Result<&str> {
            line.split('\t').find_map(|s| s.strip_prefix(key)).ok_or_else(||format!("missing {key} in {line}").into())
        };
        if line.starts_with("gpu_datalog_step\t") {
            let name = field("step=")?;
            if name == "sort/portion-unique" || name == "subtract" {
                steps.push((name == "sort/portion-unique", field("rows=")?.parse()?, field("bytes=")?.parse()?));
            }
        } else if line.starts_with("gpu_datalog_admission\t") {
            let c = field("candidates=")?.parse::<u64>()?;
            let f = field("admitted=")?.parse::<u64>()?;
            let mut u = f;
            let mut width = 0;
            if c != 0 {
                let (mut raw, mut portions, mut subtractions) = (0, 0, 0);
                let mut last_sub = None;
                for &(sort, rows, bytes) in steps.iter().rev() {
                    if sort {
                        raw += rows; portions += 1;
                        assert_eq!(bytes % rows, 0); width = bytes / rows;
                        if raw >= c { break; }
                    } else {
                        last_sub.get_or_insert(rows); subtractions += 1;
                    }
                }
                if raw != c {return Err(format!("nonmatching portion inputs ({raw} != {c}): {line}").into());}
                // One inter-portion subtract per later nonempty portion. A final
                // subtract (if the committed set is nonempty) consumes the union
                // of ALL portions; its input is exactly the within-round count.
                if subtractions == portions { u = last_sub.unwrap(); }
                else if subtractions + 1 != portions {return Err(format!("ambiguous subtract stages: {line}").into());}
            }
            if !(f <= u && u <= c) { return Err(format!("invalid census count: {line}").into()); }
            println!("{}\t{}\t{}\t{}\t{c}\t{u}\t{f}\t{width}",field("stratum=")?,field("phase=")?,field("round=")?,field("relation=")?);
            admissions+=1;
            steps.clear();
        }
    }
    if !completed || admissions==0 {return Err("incomplete run or missing admission trace".into());}
    Ok(())
}

struct Keys(Buffer<u64>);
impl rccl_cub::DeviceRange for Keys {fn len(&self)->usize {self.0.len()}}
// SAFETY: the wrapper owns the named initialized prefix during the CUDA operation.
unsafe impl cuda_host::KernelSliceArg for Keys {
    type Elem=u64;
    fn len(&self)->usize {self.0.len()}
    fn cu_deviceptr(&self)->u64 {self.0.cu_deviceptr()}
}
unsafe impl cuda_host::KernelSliceArgMut for Keys {}

fn time(pool:&Pool, mut prepare:impl FnMut()->Result<()>, mut op:impl FnMut()->Result<()>) -> Result<f64> {
    let flags=Some(cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT);
    prepare()?; op()?;
    let mut samples=[0.;5];
    for s in &mut samples {
        prepare()?; let begin=pool.stream().record_event(flags)?;
        op()?; let end=pool.stream().record_event(flags)?; end.synchronize()?;
        *s=f64::from(begin.elapsed_ms(&end)?);
    }
    samples.sort_by(f64::total_cmp); Ok(samples[2])
}

fn check(keys:&Buffer<u64>, n:usize, sorted:bool)->Result<()> {
    let pool=keys.pool();
    // The odd multiplier is invertible modulo 2^44. Distinct decoded ranks in
    // [0,U) plus length U prove exact set equality without a 100M-key CPU sort.
    let mut inv=1u64;
    for _ in 0..6 { inv=inv.wrapping_mul(2u64.wrapping_sub(HD_MULT.wrapping_mul(inv))); }
    let mut seen=vec![0u32;n.div_ceil(32)];
    let mut chunk=vec![0u64;n.min(1<<20)];
    let mut last=None;
    for start in (0..n).step_by(chunk.len()) {
        let k=chunk.len().min(n-start);
        unsafe {cuda_core::simt::memory::memcpy_dtoh_async(chunk.as_mut_ptr(),keys.cu_deviceptr()+(8*start) as u64,k*8,pool.stream().cu_stream())?;}
        pool.stream().synchronize()?;
        for &key in &chunk[..k] {
            let rank=(key.wrapping_mul(inv) & ((1u64<<HD_BITS)-1)) as usize;
            if key>=1<<HD_BITS || rank>=n {return Err("out-of-domain GPU output".into());}
            let bit=1<<(rank%32);
            if seen[rank/32]&bit!=0 || (sorted && last.is_some_and(|x|x>=key)) {return Err("duplicate or unsorted GPU key".into());}
            seen[rank/32]|=bit; last=Some(key);
        }
    }
    Ok(())
}

// Unsorted input is restored BETWEEN samples so radix never benchmarks its
// own already-sorted output. sort(U) of the winning keys uses no unique pass.
fn sort(pool:&Arc<Pool>, src:&Buffer<u64>, n:usize, unique:Option<usize>)->Result<f64> {
    let cub=rccl_cub::cuda::Program::load(pool.context())?;
    let on=cub.on(pool.stream())?;
    let mut scratch=rccl_cub::device_radix_sort_keys_buffer_workspace_in::<u64,_>(pool.resource(),&on,n)?;
    let mut select=if unique.is_some() {Some(rccl_cub::thrust_integration::compaction_workspace_in(pool.resource(),&on,n)?)} else {None};
    let alloc=|n|->Result<_>{Ok(Keys(unsafe{Buffer::uninitialized(pool.clone(),n)?}))};
    let (mut first, mut second, mut count)=(alloc(n)?,alloc(n)?,alloc(1)?);
    let output=alloc(unique.unwrap_or(1))?;
    let dst=first.0.cu_deviceptr(); let mut first_result=false;
    let ms=time(pool, ||{
        unsafe{cuda_core::simt::memory::memcpy_dtod_async(dst,src.cu_deviceptr(),n*8,pool.stream().cu_stream())?;}Ok(())
    }, ||{
        let mut pair=rccl_cub::DoubleBuffer::new(&mut first,&mut second).ok_or("unequal radix buffers")?;
        rccl_cub::device_radix_sort_keys_buffer(&on,&mut pair,0..HD_BITS,&mut scratch)?;
        first_result=pair.current().0.cu_deviceptr()==dst;
        if let Some(work)=&mut select {
            rccl_cub::thrust_integration::device_unique_copy_by(&on,DeviceSliceRead::new(pair.current().0.ptr().cast_const()),DeviceSliceWrite::new(output.0.ptr()),&mut count,|a:u64,b:u64|a==b,n,work)?;
        }
        Ok(())
    })?;
    if let Some(expected)=unique {
        assert_eq!(count.0.to_host()?[0],expected as u64);
        check(&output.0,expected,true)?;
    } else { check(if first_result {&first.0}else{&second.0},n,true)?; }
    Ok(ms)
}

fn bench(pool:&Arc<Pool>, distinct:u32, factor:u32, local:bool, repeats:u32)->Result<()> {
    let n=distinct.checked_mul(factor).ok_or("row count overflow")?;
    if distinct==0 || n>200_000_000 || factor==0 {return Err("expected 1..200M input candidates".into());}
    if repeats==0 {return Err("zero stream portions".into());}
    let table_slots=(2*distinct).next_power_of_two();
    let module=unsafe{hd_kernels::from_module(cuda_host::load_all_ptx_bundles_merged(pool.context())?)?};
    let input=unsafe{Buffer::uninitialized(pool.clone(),n as usize)?};
    let table=unsafe{Buffer::<u64>::uninitialized(pool.clone(),table_slots as usize)?};
    let output=unsafe{Buffer::uninitialized(pool.clone(),distinct as usize)?};
    let counters=unsafe{Buffer::<u32>::uninitialized(pool.clone(),3)?};
    let stream=pool.stream(); let config=||LaunchConfig1D::new(852,256,0);
    module.hd_generate(stream,&module.prepare_hd_generate(config())?,input.ptr(),n,distinct,factor,local)?;
    let kernel=module.prepare_hd_insert(config())?;
    let (mut times,mut post)=([0.;2],[0.;2]);
    for (i,prefilter) in [false,true].into_iter().enumerate() {
        // Include hash/counter clear and first-winner warp-compacted writes.
        times[i]=time(pool,||Ok(()),||{
            unsafe {
                cuda_core::simt::memory::memset_d8_async(table.cu_deviceptr(),0,table.len()*8,stream.cu_stream())?;
                cuda_core::simt::memory::memset_d8_async(counters.cu_deviceptr(),0,12,stream.cu_stream())?;
            }
            for _ in 0..repeats {
                module.hd_insert(stream,&kernel,input.ptr(),n,table.ptr(),table_slots-1,output.ptr(),distinct,counters.ptr(),prefilter)?;
            }
            Ok(())
        })?;
        assert_eq!(counters.to_host()?,[distinct,0,1]);
        check(&output,distinct as usize,false)?;
        post[i]=sort(pool,&output,distinct as usize,None)?;
    }
    let sorted=if repeats==1 {sort(pool,&input,n as usize,Some(distinct as usize))?}
        else {portion_sort(pool,&input,distinct,repeats)?};
    let rows=u64::from(n)*u64::from(repeats);
    println!("{distinct}\t{}\t{}\t{rows}\t{}\t{:.4}\t{:.4}\t{:.4}\t{:.4}\t{sorted:.4}\t{:.3}\t{:.3}\tPASS",u64::from(factor)*u64::from(repeats),if local{"adjacent"}else{"scattered"},8*u64::from(table_slots),times[0],times[1],post[0],post[1],rows as f64/times[0]/1e6,rows as f64/times[1]/1e6);
    Ok(())
}
// A physical tile repeated many times makes a stream larger than VRAM. This
// is a controlled working-set experiment, not a DOOP candidate-order replay.
// Sort each portion and remove overlap with the accumulated sorted set. Never
// assume that distinct portions are disjoint or that later portions add zero.
fn portion_sort(pool:&Arc<Pool>,input:&Buffer<u64>,distinct:u32,repeats:u32)->Result<f64> {
    use gpu_datalog_runtime::{DeviceRadixRun, RadixRunParams};
    let params=RadixRunParams{ratio:4,radix_bits:0,merge_tile_rows:32,block_threads:256};
    let state=std::cell::RefCell::new(None);
    let mut first=None;
    let ms=time(pool,||{
        *state.borrow_mut()=Some(DeviceRadixRun::<u64,u64,()>::new_set(pool.clone(),params,HD_BITS)?);
        Ok(())
    },||{
        let mut slot=state.borrow_mut();
        let set=slot.as_mut().unwrap();
        for part in 0..repeats {
            let new=unsafe{set.fresh(input,input.len())?};
            if part==0 {first=Some(new.clone());}
            unsafe {set.append_sorted(new)?;}
        }
        Ok(())
    })?;
    let first=first.unwrap();
    assert_eq!(first.len(),distinct as usize);
    assert_eq!(state.into_inner().unwrap().len(),distinct as usize);
    check(first.buffer(),distinct as usize,true)?;
    Ok(ms)
}
fn overflow(pool:&Arc<Pool>)->Result<()> {
    let module=unsafe{hd_kernels::from_module(cuda_host::load_all_ptx_bundles_merged(pool.context())?)?};
    let stream=pool.stream(); let cfg=||LaunchConfig1D::new(8,256,0);
    let (u,n)=(1024,8192);
    let input=unsafe{Buffer::uninitialized(pool.clone(),n)?};
    let table=Buffer::zeroed(pool.clone(),64)?;
    let partial=unsafe{Buffer::<u64>::uninitialized(pool.clone(),32)?};
    let counters=Buffer::<u32>::zeroed(pool.clone(),3)?;
    module.hd_generate(stream,&module.prepare_hd_generate(cfg())?,input.ptr(),n as u32,u as u32,8,false)?;
    module.hd_insert(stream,&module.prepare_hd_insert(cfg())?,input.ptr(),n as u32,table.ptr(),63,partial.ptr(),32,counters.ptr(),true)?;
    let state=counters.to_host()?;
    if state[1]==0 {return Err("undersized hash terminal did not flag overflow".into());}
    // No partial hash results escape: replay the original bag with sort/unique.
    sort(pool,&input,n,Some(u))?;
    println!("overflow: 64 slots / 32 output rows; discarded partial output; fallback produced 1024 exact keys PASS");
    Ok(())
}
// Historical-count backtest. Estimates on successful dispatches only: not an
// engine timing. Calibration inputs are the bench/stream TSV outputs.
fn policy(census:&str, profiles:&[String]) -> Result<()> {
    use std::collections::{HashMap,VecDeque};
    struct Sample { u:f64, n:f64, slots:f64, hash:f64, post:f64, sort:f64 }
    let mut samples=Vec::new();
    for path in profiles {
        for line in BufReader::new(File::open(path)?).lines() {
            let line=line?; let v:Vec<_>=line.split('\t').collect();
            if v[0]=="unique" {continue;}
            if v.len()!=13 || v[12]!="PASS" {return Err("invalid timing TSV".into());}
            if v[2]=="scattered" {samples.push(Sample {u:v[0].parse()?, n:v[3].parse()?,
                slots:v[4].parse::<f64>()?/8.,hash:v[5].parse()?,post:v[7].parse()?,sort:v[9].parse()?});}
        }
    }
    if samples.is_empty() {return Err("no scattered calibration samples".into());}
    let mut history:HashMap<(String,String),VecDeque<(u64,u64)>>=HashMap::new();
    let (mut good,mut bad,mut c_good,mut u_good,mut c_bad,mut empty)=(0,0,0,0,0,0);
    let (mut hash_ms,mut post_ms,mut sort_ms)=(0.,0.,0.);
    for line in BufReader::new(File::open(census)?).lines().skip(1) {
        let line=line?; let v:Vec<_>=line.split('\t').collect();
        if v.len()!=8 {return Err("invalid census TSV".into());}
        let c:u64=v[4].parse()?; let u:u64=v[5].parse()?;
        let history=history.entry((v[0].into(),v[3].into())).or_default();
        let pc=history.iter().map(|x|x.0).sum::<u64>();
        let pu=history.iter().map(|x|x.1).sum::<u64>();
        let max_u=history.iter().map(|x|x.1).max().unwrap_or(0);
        if history.len()==8 {history.pop_front();} history.push_back((c,u));
        let slots=(4*max_u).max(200_000).next_power_of_two();
        let needed=if slots*8<=128<<20 {4} else {16};
        if pc<800_000 || pu==0 || pc<needed*pu || slots*8>2<<30 {continue;}
        if c==0 {empty+=1;continue;}
        if v[7]!="8" {continue;}
        if u>slots/2 {bad+=1;c_bad+=c;continue;}
        good+=1; c_good+=c; u_good+=u;
        let (n,u,s)=(c as f64,u as f64,slots as f64);
        // Hash lookup locality follows the allocated capacity, not just U.
        // Sort streams are chosen by size and duplication, independent of table.
        let nearest=|score:&dyn Fn(&Sample)->f64| samples.iter().min_by(|a,b|score(a).total_cmp(&score(b))).unwrap();
        let h=nearest(&|p|4.*(s/p.slots).log2().abs()+((n/u)/(p.n/p.u)).log2().abs());
        let b=nearest(&|p|2.*(n/p.n).log2().abs()+((n/u)/(p.n/p.u)).log2().abs());
        let p=nearest(&|p|(u/p.u).log2().abs());
        hash_ms+=(h.hash*n/h.n).max(s*8./890.6e6+0.0015);
        post_ms+=(p.post*u/p.u).max(0.057);
        sort_ms+=(b.sort*n/b.n).max(0.070);
    }
    println!("nonempty_ok={good} fallback={bad} empty_predicted={empty} candidates_ok={c_good} unique_ok={u_good} candidates_fallback={c_bad} scattered_model_success_only_hash_ms={hash_ms:.1} post_ms={post_ms:.1} base_sort_unique_ms={sort_ms:.1}");
    Ok(())
}
fn main()->Result<()> {
    let args:Vec<_>=std::env::args().skip(1).collect();
    if args.len()==2 && args[0]=="census" {return census(&args[1]);}
    if args.as_slice()==["overflow"] {return overflow(&gpu_datalog_runtime::thread_pool()?);}
    if args.len()>=3 && args[0]=="policy" {return policy(&args[1],&args[2..]);}
    let stream=args.first().map(String::as_str)==Some("stream");
    if !(stream && args.len()==5 || args.first().map(String::as_str)==Some("bench") && [1,4].contains(&args.len())) {
        return Err("hashdedup census TRACE | bench [UNIQUE FACTOR scattered|adjacent] | stream UNIQUE TILE_FACTOR scattered|adjacent REPEATS".into());
    }
    let pool=gpu_datalog_runtime::thread_pool()?;
    println!("unique\tfactor\torder\tcandidates\ttable_bytes\thash_ms\tmatch_ms\tpost_hash_sort_ms\tpost_match_sort_ms\tsort_unique_ms\thash_Gins_s\tmatch_Gins_s\toracle");
    if args.len()>=4 {
        if !["adjacent","scattered"].contains(&args[3].as_str()){return Err("unknown order".into());}
        return bench(&pool,args[1].parse()?,args[2].parse()?,args[3]=="adjacent",if stream{args[4].parse()?}else{1});
    }
    for (u,factors) in [(100_000,&[1,2,4,8,16,64,400][..]),(1_000_000,&[1,2,4,8,16,64][..]),(4_000_000,&[1,2,4,8,16][..]),(8_000_000,&[1,2,4,8,16][..]),(100_000_000,&[1,2][..])] {
        for &f in factors {for local in [false,true] {if !(local && f==1) {bench(&pool,u,f,local,1)?;}}}
    }
    Ok(())
}
