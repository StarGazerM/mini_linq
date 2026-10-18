mod common;
use common::{median, memory, Edges};
use cuda_core::{CudaContext, DeviceBuffer, LaunchConfig1D};
use gpu_blocked_run::{Buffer, MemoryResource};
use gpu_triangles::{ptr, Error, Graph, Triple, Workspace};
use rccl_memory::{CudaMemoryResource, PoolMemoryResource};
use serde_json::json;

fn main() -> Result<(), Error> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let path = args
        .first()
        .ok_or("gpu-triangles EDGES EXPECTED_COUNT RESULTS_JSON [pool_capacity=64] [K=8]")?;
    let expected: u64 = args
        .get(1)
        .ok_or("missing expected DuckDB count")?
        .parse()?;
    let result_path = args.get(2).ok_or("missing result path")?;
    let capacity = args.get(3).map(|s| s.parse()).transpose()?.unwrap_or(64u32);
    let threshold = args.get(4).map(|s| s.parse()).transpose()?.unwrap_or(8u32);
    let edges = Edges::load(path)?;
    let wedge_start = std::time::Instant::now();
    let wedges = edges.wedges();
    let wedge_count_ms = wedge_start.elapsed().as_secs_f64() * 1000.0;
    println!(
        "input={} distinct={} wedges={} preparation_ms={}",
        edges.input_rows,
        edges.sources.len(),
        wedges,
        edges.preparation_ms
    );
    let ctx = CudaContext::new(0)?;
    let stream = ctx.default_stream();
    let _module = unsafe { gpu_triangles::kernels::load(&ctx)? };
    let (baseline_free, total_mem) = memory()?;
    let keys = DeviceBuffer::from_host(&stream, &edges.sources)?;
    let neighbors = DeviceBuffer::from_host(&stream, &edges.targets)?;
    let reserved = edges.pool_bytes(capacity as usize * gpu_triangles::SLOT_WORDS * 4);
    let memory_pool = MemoryResource::new(PoolMemoryResource::new(
        CudaMemoryResource::new(ctx.clone()),
        reserved,
        reserved,
    )?);
    let workspace = Workspace::new(&memory_pool, &stream, capacity)?;
    let timing = Some(cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT);
    let mut results = Vec::new();
    let mut minimum_free = memory()?.0;
    for k in [0, threshold] {
        let mut builds = vec![];
        let mut kernels = vec![];
        let mut totals = vec![];
        let mut steals = vec![];
        let mut publications = vec![];
        let mut full = vec![];
        let mut maximum = 0;
        for iteration in 0..6 {
            stream.synchronize()?;
            let begin = stream.record_event(timing)?;
            let graph = Graph::build(
                &ctx,
                &stream,
                &memory_pool,
                &keys,
                &neighbors,
                edges.domain(),
            )?;
            let built = stream.record_event(timing)?;
            workspace.launch(&stream, graph.view(), k, 568, core::ptr::null_mut(), 0, 0)?;
            let finished = stream.record_event(timing)?;
            finished.synchronize()?;
            let result = workspace.result()?;
            if result.count != expected {
                return Err(format!("triangle mismatch {} != {expected}", result.count).into());
            }
            if result.remaining != 0 {
                return Err("unfinished tile owners".into());
            }
            minimum_free = minimum_free.min(memory()?.0);
            let build = begin.elapsed_ms(&built)?;
            let kernel = built.elapsed_ms(&finished)?;
            let total = begin.elapsed_ms(&finished)?;
            println!("K={k} iteration={iteration} count={} build_ms={build:.6} kernel_ms={kernel:.6} total_ms={total:.6} steals={} published={} pool_full={} max_M={}",result.count,result.steals,result.published,result.pool_full,result.max_expansion);
            maximum = maximum.max(result.max_expansion);
            if iteration > 0 {
                builds.push(build);
                kernels.push(kernel);
                totals.push(total);
                steals.push(result.steals);
                publications.push(result.published);
                full.push(result.pool_full);
            }
        }
        results.push(json!({"mode":if k==0{"pool_off"}else{"pool_on"},"k":k,"pool_capacity":capacity,"grid_blocks":568,"tile_items":1024,"descriptor_bytes":gpu_triangles::DESCRIPTOR_WORDS*4,"answer_count":expected,"exact":true,"build_ms":builds,"kernel_ms":kernels,"total_ms":totals,"build_median_ms":median(&builds),"kernel_median_ms":median(&kernels),"total_median_ms":median(&totals),"steals":steals,"publications":publications,"pool_full":full,"max_m_over_t":maximum as f64/1024.0,"max_expansion":maximum}));
    }
    // BFS needs every wedge simultaneously, before the filter launch. Capacity
    // rejection is recorded explicitly rather than pretending to execute it.
    let demand = wedges
        .checked_mul(std::mem::size_of::<Triple>() as u64)
        .ok_or("wedge byte overflow")?;
    let free = memory()?.0;
    let bfs = if demand as u128 + reserved as u128 + (64u128 << 20) > free as u128 {
        json!({"status":"oom_preflight","wedges":wedges,"materialized_bytes":demand,"compact_u32_triple_bytes":wedges*12,"available_bytes":free,"note":"full i64 triples cannot fit; no allocation attempted"})
    } else {
        let bfs_reserved = reserved + usize::try_from(demand)? + (1 << 20);
        let bfs_pool = MemoryResource::new(PoolMemoryResource::new(
            CudaMemoryResource::new(ctx.clone()),
            bfs_reserved,
            bfs_reserved,
        )?);
        let out = Buffer::<Triple>::zeroed_in(&bfs_pool, &stream, usize::try_from(wedges)?)?;
        let answer = Buffer::<u64>::zeroed_in(&bfs_pool, &stream, 1)?;
        let bfs_workspace = Workspace::new(&bfs_pool, &stream, 0)?;
        let module = unsafe { gpu_triangles::kernels::load(&ctx)? };
        let launch = module.prepare_filter_wedges(LaunchConfig1D::new(
            wedges.div_ceil(256).max(1).try_into()?,
            256,
            32,
        ))?;
        let mut samples = vec![];
        let mut bfs_minimum_free = memory()?.0;
        for iteration in 0..6 {
            let graph = Graph::build(&ctx, &stream, &bfs_pool, &keys, &neighbors, edges.domain())?;
            let start = stream.record_event(timing)?;
            bfs_workspace.launch(&stream, graph.view(), 0, 568, ptr(&out), wedges, 1)?;
            unsafe {
                cuda_core::memory::memset_d8_async(ptr(&answer) as u64, 0, 8, stream.cu_stream())?;
                module.filter_wedges(
                    &stream,
                    &launch,
                    graph.view(),
                    ptr(&out),
                    wedges,
                    ptr(&answer),
                )?;
            }
            let stop = stream.record_event(timing)?;
            stop.synchronize()?;
            let count = answer.to_host_vec()?[0];
            if count != expected || bfs_workspace.result()?.count != wedges {
                return Err("BFS reference mismatch".into());
            }
            let ms = start.elapsed_ms(&stop)?;
            if iteration > 0 {
                samples.push(ms);
            }
            bfs_minimum_free = bfs_minimum_free.min(memory()?.0);
            println!("BFS iteration={iteration} kernel_ms={ms:.6} count={count}");
        }
        json!({"status":"complete","exact":true,"answer_count":expected,"wedges":wedges,"materialized_bytes":demand,"compact_u32_triple_bytes":wedges*12,"kernel_ms":samples,"kernel_median_ms":median(&samples),"peak_device_bytes":baseline_free.saturating_sub(bfs_minimum_free),"includes_retained_lazy_pool":reserved})
    };
    let result = json!({"input":path,"semantics":"directed distinct edges; no symmetrization or vertex inequality guards","input_rows":edges.input_rows,"distinct_edges":edges.sources.len(),"source_domain":edges.domain(),"directory":if gpu_triangles::dense_domain(edges.domain(),edges.sources.len()){"rank"}else{"fences_binary"},"host_canonicalization_ms":edges.preparation_ms,"host_wedge_demand_count_ms":wedge_count_ms,"warmups":1,"runs":5,"expected_count":expected,"timing_scope":"CUDA events: GPU run build and query; ingestion/sort/dedup/upload excluded","allocation_scope":"fixed RCCL pool primed before warmup; no cuMemAlloc/Free in timed regions","pool_reserved_bytes":reserved,"peak_device_bytes":baseline_free.saturating_sub(minimum_free),"peak_device_scope":"cuMemGetInfo sampled with fixed pool live, relative to post-module/pre-upload baseline; includes pool reserve","device_total_bytes":total_mem,"loadavg":std::fs::read_to_string("/proc/loadavg")?,"gpu":results,"bfs":bfs});
    std::fs::write(result_path, serde_json::to_string_pretty(&result)? + "\n")?;
    Ok(())
}
