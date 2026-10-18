mod data;
mod kernels;
mod dedup;
use data::{Data,Error};
use kernels::{Dimensions,Output};
use cuda_core::{CudaContext,CudaStream,DeviceBuffer,LaunchConfig1D};
use gpu_blocked_run::{read_stats,Buffer,MemoryResource,Run,RunStats};
use rccl_memory::{ContiguousDeviceRange,CudaMemoryResource,PoolMemoryResource,StatisticsMemoryResource};
use serde_json::{json,Value};
use std::{collections::BTreeMap,fs,path::Path};
fn memory_used()->Result<usize,Error>{
    let mut free=0;let mut total=0;
    let status=unsafe{cuda_core::sys::cuMemGetInfo_v2(&mut free,&mut total)};
    if status!=0{return Err(format!("cuMemGetInfo: {status}").into());}Ok(total-free)
}
fn ptr<T:cuda_core::DeviceCopy>(b:&impl ContiguousDeviceRange<T>)->*const T{b.cu_deviceptr() as *const T}
fn mut_ptr<T:cuda_core::DeviceCopy>(b:&mut Buffer<'_,T>)->*mut T{b.cu_deviceptr() as *mut T}
fn median(values:&[f32])->f32{let mut v=values.to_vec();v.sort_by(f32::total_cmp);v[v.len()/2]}
fn minimum(values:&[f32])->f32{values.iter().copied().fold(f32::INFINITY,f32::min)}
fn metric(v:&[f32])->Value{json!({"median":median(v),"min":minimum(v)})}
fn copy_prefix<'a,T:cuda_core::DeviceCopy>(stream:&'a CudaStream,memory:&'a MemoryResource,input:&Buffer<'_,T>,n:usize)->Result<Buffer<'a,T>,Error>{
    let out=Buffer::<T>::zeroed_in(memory,stream,n)?;
    if n>0 {unsafe{cuda_core::memory::memcpy_dtod_async(out.cu_deviceptr(),input.cu_deviceptr(),n*std::mem::size_of::<T>(),stream.cu_stream())?;}}
    Ok(out)
}
// Reserve the sum of every allocation in the larger plan, rather than just
// simultaneous live bytes. Each request includes the pool's 256-byte rounding;
// this also covers fragmentation until all iteration-local buffers are dropped.
// Column lengths were checked against manifest row counts by Data::load.
fn padded_bytes(rows:usize,width:usize)->u128{(rows as u128*width as u128).div_ceil(256)*256}
fn key_domain(keys:&[i64])->(i64,i64){
    let Some((&first,rest))=keys.split_first() else{return (0,-1);};
    rest.iter().fold((first,first),|(min,max),&key|(min.min(key),max.max(key)))
}
fn run_pool_bytes(rows:usize,(min,max):(i64,i64))->Result<u128,Error>{
    if rows==0{return Ok(padded_bytes(1,4)+padded_bytes(1,std::mem::size_of::<RunStats>()));}
    let span=max as i128-min as i128+1;
    if span>gpu_blocked_run::MAX_DOMAIN_KEYS as i128{return Err("dense rank domain exceeds 2^32 key positions".into());}
    let domain=usize::try_from(span)?;let words=domain.div_ceil(64);let unique=rows.min(domain);
    // Device stats, bins, bitmap/prefix, word row offsets, CSR, unique keys,
    // permutation, cursors, sorted keys, fences and one clustered payload.
    let mut bytes=[(1,std::mem::size_of::<RunStats>()),(domain,4),(words,8),(words,4),(words,4),
        (unique+1,4),(unique,8),(rows,4),(domain,4),(rows,8),
        (rows.div_ceil(gpu_blocked_run::BLOCK_ROWS),8),(rows,8)]
        .into_iter().map(|(n,w)|padded_bytes(n,w)).sum::<u128>();
    for mut length in [words,words] {
        loop {length=length.div_ceil(256);bytes+=padded_bytes(length,4);if length==1{break;}}
    }
    Ok(bytes)
}
fn pool_bytes(data:&Data,domains:[(i64,i64);5])->Result<usize,Error>{
    let lines=data.col("lineitem.l_orderkey").len();let orders=data.col("orders.o_orderkey").len();
    let capacity=lines.max(1).checked_mul(4).and_then(usize::checked_next_power_of_two).ok_or("dedup capacity overflow")?;
    // Three persistent outputs plus three exactness-readback copies; two
    // capacity-sized filter columns; counters, dedup and combined run metadata.
    let bytes=6*padded_bytes(lines,8)+2*padded_bytes(orders,8)
        +padded_bytes(4,4)+padded_bytes(1,4)+padded_bytes(2,4)+padded_bytes(capacity,8)
        +padded_bytes(4,std::mem::size_of::<RunStats>())
        +run_pool_bytes(data.col("customer.c_custkey").len(),domains[0])?
        +run_pool_bytes(data.col("supplier.s_suppkey").len(),domains[1])?
        +run_pool_bytes(orders,domains[2])?.max(run_pool_bytes(orders,domains[3])?)
        +run_pool_bytes(lines,domains[4])?;
    Ok(usize::try_from(bytes)?)
}
fn main()->Result<(),Error>{
    let args=std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().map(String::as_str)==Some("--synthetic") {data::synthetic(Path::new(args.get(1).ok_or("--synthetic DIR")?))?;return Ok(());}
    let merge=args.iter().any(|s|s=="--merge");
    let unfiltered=args.iter().any(|s|s=="--unfiltered-orders");
    let args=args.into_iter().filter(|s|s!="--merge" && s!="--unfiltered-orders").collect::<Vec<_>>();
    let dir=Path::new(args.first().ok_or("usage: gpu-h05 DATA_DIR [A|B|both] [RESULTS_JSON] [--merge] [--unfiltered-orders]")?);
    let plan=args.get(1).map(String::as_str).unwrap_or("both");
    if !["A","B","both"].contains(&plan){return Err("plan must be A, B or both".into());}
    let result_path=args.get(2).map(String::as_str).unwrap_or("compare/gpu_h05.results.json");
    if unfiltered && plan!="A" {return Err("--unfiltered-orders requires Plan A".into());}
    let data=Data::load(dir)?;
    // Bounds are computed once from uploaded-column sources, never downloaded
    // from the GPU. Filtered runs retain these conservative full-column bounds.
    let domains=["customer.c_custkey","supplier.s_suppkey","orders.o_orderkey","orders.o_custkey","lineitem.l_orderkey"]
        .map(|name|key_domain(data.col(name)));
    let [customer_domain,supplier_domain,order_domain,order_customer_domain,line_domain]=domains;
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
    let pool_size=pool_bytes(&data,domains)?+data.col("orders.o_orderkey").len()*8+4*data.col("orders.o_orderkey").len().div_ceil(256)+4096;
    // Equal initial/maximum sizes forbid upstream CUDA allocations in warmup
    // and timed iterations. Only input uploads remain cuda_core-owned.
    let memory:MemoryResource=StatisticsMemoryResource::new(PoolMemoryResource::new(CudaMemoryResource::new(ctx.clone()),pool_size,pool_size)?);
    // Each lineitem has one PK/FK join path, so this is a strict output bound.
    let mut output_orders=Buffer::<i64>::zeroed_in(&memory,&stream,line_count)?;
    let mut output_suppliers=Buffer::<i64>::zeroed_in(&memory,&stream,line_count)?;
    let mut output_nations=Buffer::<i64>::zeroed_in(&memory,&stream,line_count)?;
    let mut counters=Buffer::<u32>::zeroed_in(&memory,&stream,4)?;
    let output=Output{orders:mut_ptr(&mut output_orders),suppliers:mut_ptr(&mut output_suppliers),nations:mut_ptr(&mut output_nations),counters:mut_ptr(&mut counters)};
    let timing=Some(cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT);
    let mut results=vec![];
    let launch_a=module.prepare_plan_a(LaunchConfig1D::new(568,256,9216))?;
    let launch_merge=module.prepare_plan_a_merge(LaunchConfig1D::new(568,256,17408))?;
    let launch_b=module.prepare_plan_b(LaunchConfig1D::new(568,256,33808))?;
    for which in ["A","B"] {
        if plan!="both" && plan!=which{continue;}
        let mut build_samples=vec![];let mut kernel_samples=vec![];let mut dedup_samples=vec![];let mut total_samples=vec![];
        let mut allocation_samples=vec![];let mut peak_bytes_samples=vec![];
        let mut sorted_build_samples=vec![];
        let mut order_rows=0u32;
        let mut peak_mem=memory_used()?;let mut max_fanout=[0u32;2];let mut answer_count=0;
        for iteration in 0..6 {
            stream.synchronize()?;
            let persistent_bytes=memory.push_counters().bytes.value;
            let (b,k,u,total,build_stats)={
            let start=stream.record_event(timing)?;
            let order_count=data.col("orders.o_orderkey").len();
            let filter_capacity=if unfiltered {0}else{order_count};
            let mut filter_keys=Buffer::<i64>::zeroed_in(&memory,&stream,filter_capacity)?;
            let mut filter_customers=Buffer::<i64>::zeroed_in(&memory,&stream,filter_capacity)?;
            let mut block_counts=Buffer::<u32>::zeroed_in(&memory,&stream,filter_capacity.div_ceil(256).max(1))?;
            let filter_scan=if unfiltered {vec![]}else{
                let filter_launch=module.prepare_filter_orders(LaunchConfig1D::new(order_count.div_ceil(256).max(1) as u32,256,64))?;
                module.filter_orders(&stream,&filter_launch,dcol("orders.o_orderkey"),dcol("orders.o_custkey"),dcol("orders.o_orderdate"),data.start,data.end,mut_ptr(&mut filter_keys),mut_ptr(&mut filter_customers),mut_ptr(&mut block_counts),false)?;
                let scan=gpu_blocked_run::exclusive_scan(&stream,&memory,&block_counts)?;
                module.filter_orders(&stream,&filter_launch,dcol("orders.o_orderkey"),dcol("orders.o_custkey"),dcol("orders.o_orderdate"),data.start,data.end,mut_ptr(&mut filter_keys),mut_ptr(&mut filter_customers),mut_ptr(&mut block_counts),true)?;
                scan
            };
            let selected=filter_scan.last();
            let customers=Run::build(&ctx,&stream,&memory,dcol("customer.c_custkey"),&[dcol("customer.c_nationkey") as &dyn ContiguousDeviceRange<i64>],customer_domain,None)?;
            let suppliers=Run::build(&ctx,&stream,&memory,dcol("supplier.s_suppkey"),&[dcol("supplier.s_nationkey") as &dyn ContiguousDeviceRange<i64>],supplier_domain,None)?;
            let orders=if unfiltered {
                Run::build(&ctx,&stream,&memory,dcol("orders.o_orderkey"),&[dcol("orders.o_custkey") as &dyn ContiguousDeviceRange<i64>,dcol("orders.o_orderdate") as &dyn ContiguousDeviceRange<i64>],order_domain,None)?
            }else if which=="A" {Run::build(&ctx,&stream,&memory,&filter_keys,&[&filter_customers as &dyn ContiguousDeviceRange<i64>],order_domain,selected)?}else{Run::build(&ctx,&stream,&memory,&filter_customers,&[&filter_keys as &dyn ContiguousDeviceRange<i64>],order_customer_domain,selected)?};
            let lines=if which=="B" {Some(Run::build(&ctx,&stream,&memory,dcol("lineitem.l_orderkey"),&[dcol("lineitem.l_suppkey") as &dyn ContiguousDeviceRange<i64>],line_domain,None)?)}else{None};
            // One combined metadata readback also rejects invalid device row
            // counts/domains before any plan consumes the constructed runs.
            let build_stats=if let Some(lines)=&lines {
                read_stats(&memory,&stream,&[&customers,&suppliers,&orders,lines])?
            }else{
                read_stats(&memory,&stream,&[&customers,&suppliers,&orders])?
            };
            peak_mem=peak_mem.max(memory_used()?);
            unsafe{cuda_core::memory::memset_d8_async(counters.cu_deviceptr(),0,counters.num_bytes(),stream.cu_stream())?;}
            let dims=Dimensions{orders:orders.view(),customer:customers.view(),supplier:suppliers.view(),order_customer:ptr(&orders.payloads[0]),customer_nation:ptr(&customers.payloads[0]),supplier_nation:ptr(&suppliers.payloads[0]),nation_region:ptr(&nation_region),nation_len:nr.len() as u32,asia_regions,
                order_keys:ptr(&orders.keys),order_fences:ptr(&orders.fences),order_rows:build_stats[2].rows,
                order_dates:if unfiltered {ptr(&orders.payloads[1])}else{std::ptr::null()},start:data.start,end:data.end};
            let built=stream.record_event(timing)?;
            if which=="A" {
                if merge {module.plan_a_merge(&stream,&launch_merge,dcol("lineitem.l_orderkey"),dcol("lineitem.l_suppkey"),dims,output)?;}
                else {module.plan_a(&stream,&launch_a,dcol("lineitem.l_orderkey"),dcol("lineitem.l_suppkey"),dims,output)?;}
            }else{
                let lines=lines.as_ref().unwrap();
                module.plan_b(&stream,&launch_b,&customers.keys,&customers.payloads[0],dims,orders.view(),ptr(&orders.payloads[0]),lines.view(),ptr(&lines.keys),ptr(&lines.payloads[0]),output)?;
            }
            let executed=stream.record_event(timing)?;
            let counts=counters.to_host_vec()?;let emitted=counts[0] as usize;
            if emitted>line_count{return Err("output capacity invariant violated".into());}
            max_fanout[0]=max_fanout[0].max(counts[2]);max_fanout[1]=max_fanout[1].max(counts[3]);
            let capacity=emitted.max(1).checked_mul(4).and_then(usize::checked_next_power_of_two).ok_or("dedup capacity overflow")?;
            let (unique,unique_count)=dedup::dedup(&ctx,&stream,&memory,&output_orders,&output_suppliers,&output_nations,&counters,emitted,capacity)?;
            peak_mem=peak_mem.max(memory_used()?);
            let finished=stream.record_event(timing)?;finished.synchronize()?;
            let b=start.elapsed_ms(&built)?;let k=built.elapsed_ms(&executed)?;let u=executed.elapsed_ms(&finished)?;let total=start.elapsed_ms(&finished)?;
            let dedup_metadata=unique_count.to_host_vec()?;
            if dedup_metadata[1]!=0{return Err("dedup hash table full: capacity invariant violated".into());}
            answer_count=dedup_metadata[0] as usize;
            let answer=unique.to_host_vec()?;
            let mut actual=answer.into_iter().filter(|&v|v!=0).map(dedup::decode).collect::<Vec<_>>();
            actual.sort_unstable();
            if actual.len()!=answer_count{return Err("hash table/count disagreement".into());}
            if actual!=data.answer {return Err(format!("Plan {which} exact mismatch: {} vs {} rows; first {:?} / {:?}",actual.len(),data.answer.len(),actual.iter().zip(&data.answer).find(|(a,b)|a!=b),data.answer.first()).into());}
            // Also check the materialized nation column, not only packed keys.
            let eo=copy_prefix(&stream,&memory,&output_orders,emitted)?.to_host_vec()?;
            let es=copy_prefix(&stream,&memory,&output_suppliers,emitted)?.to_host_vec()?;
            let en=copy_prefix(&stream,&memory,&output_nations,emitted)?.to_host_vec()?;
            let mut triples=eo.into_iter().zip(es).zip(en).map(|((o,s),n)|(o,s,n)).collect::<Vec<_>>();triples.sort_unstable();triples.dedup();
            if triples!=data.answer{return Err("materialized nation mismatch".into());}
            (b,k,u,total,build_stats)
            }; // Drop every iteration-local buffer before closing its counters.
            let allocations=memory.pop_counters()?;
            let allocation_count=allocations.allocations.total;
            let peak_bytes=persistent_bytes+allocations.bytes.peak;
            order_rows=build_stats[2].rows;
            // Report the device's decision, including empty sorted runs; do
            // not infer this fast path from host input order or column names.
            let build_names=["customer","supplier",if which=="A" {"orders_by_order"}else{"orders_by_customer"},"lineitem"];
            let sorted_builds=build_names.into_iter().zip(&build_stats)
                .filter_map(|(name,stats)|(stats.sorted==1).then_some(name)).collect::<Vec<_>>();
            println!("sf={} plan={} iteration={} build_ms={b:.6} kernel_ms={k:.6} dedup_ms={u:.6} total_ms={total:.6} answer_count={answer_count} exact=true allocation_count={allocation_count} peak_bytes={peak_bytes} sorted_builds={sorted_builds:?}",data.sf,which,iteration);
            if iteration>0 {build_samples.push(b);kernel_samples.push(k);dedup_samples.push(u);total_samples.push(total);allocation_samples.push(allocation_count);peak_bytes_samples.push(peak_bytes);sorted_build_samples.push(sorted_builds);}
        }
        let scan_gbps=if which=="A" {Some(line_count as f64*16.0/(median(&kernel_samples) as f64*1e6))}else{None};
        results.push(json!({
            "sf":data.sf,"plan":which,"build_ms":metric(&build_samples),
            "kernel_ms":metric(&kernel_samples),"dedup_ms":metric(&dedup_samples),
            "total_ms":metric(&total_samples),"answer_count":answer_count,"exact":true,
            "regs":null,"spills":null,"smem":if which=="A"{if merge {17408}else{9216}}else{33808},
            "probe_mode":if merge {"merge"}else{"point"},"unfiltered_orders":unfiltered,"merge_block_limit":kernels::MERGE_BLOCK_LIMIT,
            "orders_rows":order_rows,"orders_key_customer_bytes":order_rows as u64*16,
            "orders_date_bytes":if unfiltered {order_rows as u64*8}else{0},
            "peak_mem_mib":peak_mem.saturating_sub(baseline_mem) as f64/1048576.0,
            "peak_mem_scope":"cuMemGetInfo sampled after combined build metadata and dedup submission; includes reserved pool",
            "pool_bytes":pool_size,"allocation_scope":"iteration including exactness readback copies; peak bytes includes persistent outputs, excludes input uploads",
            "scan_gbps":scan_gbps,"bandwidth_model_gbps":960,
            "max_expansion_rows":max_fanout,
            "max_m_over_t":max_fanout.iter().max().copied().unwrap_or(0) as f64/1024.0,
            "frontier_spill_required":false,
            "expansion_policy":"fixed-depth in-tile loop; no cross-block stealing",
            "build_algorithm":"device-detected sorted adjacency or bitmap-word prefix counting sort with stable provenance heapsort",
            "sorted_path":"GPU adjacent-key nondecreasing check; device-gated histogram/scatter/heapsort; sorted adjacency builds bitmap and CSR",
            "dedup_algorithm":"exact packed-triple u64 CAS set",
            "warmups":1,"runs":5,
            "samples":{"build_ms":build_samples,"kernel_ms":kernel_samples,
                       "dedup_ms":dedup_samples,"total_ms":total_samples,
                       "allocation_count":allocation_samples,"peak_bytes":peak_bytes_samples,
                       "sorted_builds":sorted_build_samples}
        }));
    }
    let path=Path::new(result_path);if let Some(parent)=path.parent(){fs::create_dir_all(parent)?;}
    let mut existing:Vec<Value>=if path.exists(){serde_json::from_slice(&fs::read(path)?)?}else{vec![]};
    existing.retain(|r|!results.iter().any(|n|r["sf"]==n["sf"] && r["plan"]==n["plan"] && r["probe_mode"]==n["probe_mode"] && r["unfiltered_orders"]==n["unfiltered_orders"]));existing.extend(results);
    fs::write(path,serde_json::to_vec_pretty(&existing)?)?;
    Ok(())
}
