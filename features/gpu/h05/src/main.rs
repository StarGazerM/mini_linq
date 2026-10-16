mod data;
mod kernels;
mod dedup;
use data::{Data,Error};
use kernels::{Dimensions,Output};
use cuda_core::{CudaContext,CudaStream,DeviceBuffer,LaunchConfig1D};
use gpu_blocked_run::Run;
use serde_json::{json,Value};
use std::{collections::BTreeMap,fs,path::Path};
fn memory_used()->Result<usize,Error>{
    let mut free=0;let mut total=0;
    let status=unsafe{cuda_core::sys::cuMemGetInfo_v2(&mut free,&mut total)};
    if status!=0{return Err(format!("cuMemGetInfo: {status}").into());}Ok(total-free)
}
fn ptr<T:cuda_core::DeviceCopy>(b:&DeviceBuffer<T>)->*const T{b.cu_deviceptr() as *const T}
fn mut_ptr<T:cuda_core::DeviceCopy>(b:&mut DeviceBuffer<T>)->*mut T{b.cu_deviceptr() as *mut T}
fn median(values:&[f32])->f32{let mut v=values.to_vec();v.sort_by(f32::total_cmp);v[v.len()/2]}
fn minimum(values:&[f32])->f32{values.iter().copied().fold(f32::INFINITY,f32::min)}
fn metric(v:&[f32])->Value{json!({"median":median(v),"min":minimum(v)})}
fn copy_prefix<T:cuda_core::DeviceCopy>(stream:&CudaStream,input:&DeviceBuffer<T>,n:usize)->Result<DeviceBuffer<T>,Error>{
    let out=DeviceBuffer::<T>::zeroed(stream,n)?;
    if n>0 {unsafe{cuda_core::memory::memcpy_dtod_async(out.cu_deviceptr(),input.cu_deviceptr(),n*std::mem::size_of::<T>(),stream.cu_stream())?;}}
    Ok(out)
}
fn main()->Result<(),Error>{
    let args=std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().map(String::as_str)==Some("--synthetic") {data::synthetic(Path::new(args.get(1).ok_or("--synthetic DIR")?))?;return Ok(());}
    let dir=Path::new(args.first().ok_or("usage: gpu-h05 DATA_DIR [A|B|both] [RESULTS_JSON]")?);
    let plan=args.get(1).map(String::as_str).unwrap_or("both");
    if !["A","B","both"].contains(&plan){return Err("plan must be A, B or both".into());}
    let result_path=args.get(2).map(String::as_str).unwrap_or("compare/gpu_h05.results.json");
    let data=Data::load(dir)?;
    let ctx=CudaContext::new(0)?;let stream=ctx.default_stream();
    let module=unsafe{kernels::kernels::load(&ctx)?};
    let baseline_mem=memory_used()?;
    let mut uploaded=BTreeMap::new();
    for (name,col) in &data.columns {uploaded.insert(name.clone(),DeviceBuffer::from_host(&stream,col)?);}
    let dcol=|name:&str|->&DeviceBuffer<i64>{&uploaded[name]};
    let nation_max=*data.col("nation.n_nationkey").iter().max().ok_or("empty nation")?;
    if nation_max<0 || nation_max>1_000_000 {return Err("nation direct-array domain unsupported".into());}
    let mut nr=vec![-1i64;nation_max as usize+1];
    for (&n,&r) in data.col("nation.n_nationkey").iter().zip(data.col("nation.n_regionkey")){if n<0{return Err("negative nation key".into());}nr[n as usize]=r;}
    let nation_region=DeviceBuffer::from_host(&stream,&nr)?;
    let mut asia_regions=0u64;
    for (&r,&name) in data.col("region.r_regionkey").iter().zip(data.col("region.r_name")){if r<0 || r>=64{return Err("region bitmap domain unsupported".into());}if name==data.asia{asia_regions|=1u64<<r;}}
    let line_count=data.col("lineitem.l_orderkey").len();
    // Each lineitem has one PK/FK join path, so this is a strict output bound.
    let mut output_orders=DeviceBuffer::<i64>::zeroed(&stream,line_count)?;
    let mut output_suppliers=DeviceBuffer::<i64>::zeroed(&stream,line_count)?;
    let mut output_nations=DeviceBuffer::<i64>::zeroed(&stream,line_count)?;
    let mut counters=DeviceBuffer::<u32>::zeroed(&stream,4)?;
    let output=Output{orders:mut_ptr(&mut output_orders),suppliers:mut_ptr(&mut output_suppliers),nations:mut_ptr(&mut output_nations),counters:mut_ptr(&mut counters)};
    let timing=Some(cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT);
    let mut results=vec![];
    let launch_a=module.prepare_plan_a(LaunchConfig1D::new(568,256,9216))?;
    let launch_b=module.prepare_plan_b(LaunchConfig1D::new(568,256,33808))?;
    for which in ["A","B"] {
        if plan!="both" && plan!=which{continue;}
        let mut build_samples=vec![];let mut kernel_samples=vec![];let mut dedup_samples=vec![];let mut total_samples=vec![];
        let mut peak_mem=memory_used()?;let mut max_fanout=[0u32;2];let mut answer_count=0;
        for iteration in 0..6 {
            stream.synchronize()?;
            let start=stream.record_event(timing)?;
            let order_count=data.col("orders.o_orderkey").len();
            let mut filter_keys=DeviceBuffer::<i64>::zeroed(&stream,order_count)?;
            let mut filter_customers=DeviceBuffer::<i64>::zeroed(&stream,order_count)?;
            let mut selected=DeviceBuffer::<u32>::zeroed(&stream,1)?;
            let filter_launch=module.prepare_filter_orders(LaunchConfig1D::new(order_count.div_ceil(256).max(1) as u32,256,64))?;
            module.filter_orders(&stream,&filter_launch,dcol("orders.o_orderkey"),dcol("orders.o_custkey"),dcol("orders.o_orderdate"),data.start,data.end,mut_ptr(&mut filter_keys),mut_ptr(&mut filter_customers),mut_ptr(&mut selected))?;
            let count=selected.to_host_vec(&stream)?[0] as usize;
            let filtered_keys=copy_prefix(&stream,&filter_keys,count)?;
            let filtered_customers=copy_prefix(&stream,&filter_customers,count)?;
            let customers=Run::build(&ctx,&stream,dcol("customer.c_custkey"),&[dcol("customer.c_nationkey")])?;
            let suppliers=Run::build(&ctx,&stream,dcol("supplier.s_suppkey"),&[dcol("supplier.s_nationkey")])?;
            let orders=if which=="A" {Run::build(&ctx,&stream,&filtered_keys,&[&filtered_customers])?}else{Run::build(&ctx,&stream,&filtered_customers,&[&filtered_keys])?};
            let lines=if which=="B" {Some(Run::build(&ctx,&stream,dcol("lineitem.l_orderkey"),&[dcol("lineitem.l_suppkey")])?)}else{None};
            peak_mem=peak_mem.max(memory_used()?);
            peak_mem=peak_mem.max(customers.build_peak_mem_used).max(suppliers.build_peak_mem_used).max(orders.build_peak_mem_used);
            if let Some(lines)=&lines {peak_mem=peak_mem.max(lines.build_peak_mem_used);}
            unsafe{cuda_core::memory::memset_d8_async(counters.cu_deviceptr(),0,counters.num_bytes(),stream.cu_stream())?;}
            let dims=Dimensions{orders:orders.view(),customer:customers.view(),supplier:suppliers.view(),order_customer:ptr(&orders.payloads[0]),customer_nation:ptr(&customers.payloads[0]),supplier_nation:ptr(&suppliers.payloads[0]),nation_region:ptr(&nation_region),nation_len:nr.len() as u32,asia_regions};
            let built=stream.record_event(timing)?;
            if which=="A" {
                module.plan_a(&stream,&launch_a,dcol("lineitem.l_orderkey"),dcol("lineitem.l_suppkey"),dims,output)?;
            }else{
                let lines=lines.as_ref().unwrap();
                module.plan_b(&stream,&launch_b,&customers.keys,&customers.payloads[0],dims,orders.view(),ptr(&orders.payloads[0]),lines.view(),ptr(&lines.keys),ptr(&lines.payloads[0]),output)?;
            }
            let executed=stream.record_event(timing)?;
            let counts=counters.to_host_vec(&stream)?;let emitted=counts[0] as usize;
            if emitted>line_count{return Err("output capacity invariant violated".into());}
            max_fanout[0]=max_fanout[0].max(counts[2]);max_fanout[1]=max_fanout[1].max(counts[3]);
            let capacity=emitted.max(1).checked_mul(4).and_then(usize::checked_next_power_of_two).ok_or("dedup capacity overflow")?;
            let (unique,unique_count)=dedup::dedup(&ctx,&stream,&output_orders,&output_suppliers,&output_nations,emitted,capacity)?;
            peak_mem=peak_mem.max(memory_used()?);
            let finished=stream.record_event(timing)?;finished.synchronize()?;
            let b=start.elapsed_ms(&built)?;let k=built.elapsed_ms(&executed)?;let u=executed.elapsed_ms(&finished)?;let total=start.elapsed_ms(&finished)?;
            answer_count=unique_count.to_host_vec(&stream)?[0] as usize;
            let answer=unique.to_host_vec(&stream)?;
            let mut actual=answer.into_iter().filter(|&v|v!=0).map(dedup::decode).collect::<Vec<_>>();
            actual.sort_unstable();
            if actual.len()!=answer_count{return Err("hash table/count disagreement".into());}
            if actual!=data.answer {return Err(format!("Plan {which} exact mismatch: {} vs {} rows; first {:?} / {:?}",actual.len(),data.answer.len(),actual.iter().zip(&data.answer).find(|(a,b)|a!=b),data.answer.first()).into());}
            // Also check the materialized nation column, not only packed keys.
            let eo=copy_prefix(&stream,&output_orders,emitted)?.to_host_vec(&stream)?;
            let es=copy_prefix(&stream,&output_suppliers,emitted)?.to_host_vec(&stream)?;
            let en=copy_prefix(&stream,&output_nations,emitted)?.to_host_vec(&stream)?;
            let mut triples=eo.into_iter().zip(es).zip(en).map(|((o,s),n)|(o,s,n)).collect::<Vec<_>>();triples.sort_unstable();triples.dedup();
            if triples!=data.answer{return Err("materialized nation mismatch".into());}
            println!("sf={} plan={} iteration={} build_ms={b:.6} kernel_ms={k:.6} dedup_ms={u:.6} total_ms={total:.6} answer_count={answer_count} exact=true",data.sf,which,iteration);
            if iteration>0 {build_samples.push(b);kernel_samples.push(k);dedup_samples.push(u);total_samples.push(total);}
        }
        let scan_gbps=if which=="A" {Some(line_count as f64*16.0/(median(&kernel_samples) as f64*1e6))}else{None};
        results.push(json!({
            "sf":data.sf,"plan":which,"build_ms":metric(&build_samples),
            "kernel_ms":metric(&kernel_samples),"dedup_ms":metric(&dedup_samples),
            "total_ms":metric(&total_samples),"answer_count":answer_count,"exact":true,
            "regs":null,"spills":null,"smem":if which=="A"{9216}else{33808},
            "peak_mem_mib":peak_mem.saturating_sub(baseline_mem) as f64/1048576.0,
            "peak_mem_scope":"cuMemGetInfo sampled with build scratch live, after build and dedup",
            "scan_gbps":scan_gbps,"bandwidth_model_gbps":960,
            "max_expansion_rows":max_fanout,
            "max_m_over_t":max_fanout.iter().max().copied().unwrap_or(0) as f64/1024.0,
            "frontier_spill_required":false,
            "expansion_policy":"fixed-depth in-tile loop; no cross-block stealing",
            "build_algorithm":"dense counting sort with stable provenance heapsort",
            "dedup_algorithm":"exact packed-triple u64 CAS set",
            "warmups":1,"runs":5,
            "samples":{"build_ms":build_samples,"kernel_ms":kernel_samples,
                       "dedup_ms":dedup_samples,"total_ms":total_samples}
        }));
    }
    let path=Path::new(result_path);if let Some(parent)=path.parent(){fs::create_dir_all(parent)?;}
    let mut existing:Vec<Value>=if path.exists(){serde_json::from_slice(&fs::read(path)?)?}else{vec![]};
    existing.retain(|r|!results.iter().any(|n|r["sf"]==n["sf"] && r["plan"]==n["plan"]));existing.extend(results);
    fs::write(path,serde_json::to_vec_pretty(&existing)?)?;
    Ok(())
}
