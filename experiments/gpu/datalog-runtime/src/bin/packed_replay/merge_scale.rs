use super::*;
use gpu_datalog_runtime::{packed_run::PackedRun, SortedBatch};

pub(super) fn run<W: Word>(spec: &State, expected: &[W], block: u32, denominator: usize) -> Result<Value> {
    if denominator != 2 && denominator != 16 { return Err("merge delta must be 2 or 16 (other/owner = 1/denominator)".into()); }
    let mut owner = Vec::with_capacity(expected.len());
    let mut other = Vec::with_capacity(expected.len() / (denominator + 1));
    for (index, &key) in expected.iter().enumerate() {
        if index % (denominator + 1) == denominator { other.push(key); } else { owner.push(key); }
    }
    let before = tenant()?;
    let frame_policy = super::global::frame_policy()?;
    let mut results = serde_json::Map::new();
    for name in ["engine", "plain", "packed"] {
        let pool = Pool::new(0, 0, 0)?;
        let packing = Packing::new(&spec.fields.iter().map(|f| f.bits).collect::<Vec<_>>())?;
        let evict = unsafe { Buffer::<u64>::uninitialized(pool.clone(), 32 * 1024 * 1024)? };
        let module = unsafe { eviction::load(pool.context())? };
        let cold = || -> Result<()> {
            module.touch(pool.stream(), &module.prepare_touch(cuda_core::LaunchConfig1D::new(65535, 256, 0))?, evict.ptr(), evict.len() as u64)?;
            pool.stream().synchronize()?;
            Ok(())
        };
        let begin = pool.context().new_event(Some(cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT))?;
        let end = pool.context().new_event(Some(cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT))?;
        let (wall_ns, gpu_ns, moved_bytes, output_bytes) = if name == "packed" {
            let a = unsafe { PackedRun::seal(&Buffer::from_host(pool.clone(), &owner)?, owner.len(), &packing, block)? };
            let b = unsafe { PackedRun::seal(&Buffer::from_host(pool.clone(), &other)?, other.len(), &packing, block)? };
            cold()?;
            begin.record(pool.stream())?;
            let (merged, elapsed) = measure(&pool, || unsafe { a.merge_with_frames(&b, frame_policy) })?;
            end.record(pool.stream())?;
            pool.stream().synchronize()?;
            let gpu = (begin.elapsed_ms(&end)? as f64 * 1e6) as u64;
            let bytes = a.allocated_bytes() + b.allocated_bytes() + merged.allocated_bytes();
            // Explicit public oracle export, never an input to the measured merge.
            if merged.unpack()?.to_host()? != expected { return Err("scaled packed merge oracle mismatch".into()); }
            (elapsed, gpu, bytes, merged.allocated_bytes())
        } else if name == "plain" {
            let mut set = PackedSet::new_plain(pool.clone(), packing, block, denominator * 2)?;
            let a = unsafe { SortedBatch::from_sorted(Buffer::from_host(pool.clone(), &owner)?, owner.len())? };
            unsafe { set.append_sorted(&a)?; }
            drop(a);
            let b = unsafe { SortedBatch::from_sorted(Buffer::from_host(pool.clone(), &other)?, other.len())? };
            cold()?;
            begin.record(pool.stream())?;
            let (_, elapsed) = measure(&pool, || unsafe { set.append_sorted(&b) })?;
            end.record(pool.stream())?;
            pool.stream().synchronize()?;
            let gpu = (begin.elapsed_ms(&end)? as f64 * 1e6) as u64;
            if set.runs().len() != 1 || set.to_host()? != expected { return Err("scaled plain merge oracle mismatch".into()); }
            (elapsed, gpu, 2 * expected.len() * std::mem::size_of::<W>(), set.allocated_bytes())
        } else {
            let params = RadixRunParams { ratio: denominator * 2, radix_bits: 0, merge_tile_rows: 32, block_threads: 256 };
            let mut set = DeviceRadixRun::<W, u64, ()>::new_set(pool.clone(), params, packing.bits())?;
            let a = unsafe { SortedBatch::from_sorted(Buffer::from_host(pool.clone(), &owner)?, owner.len())? };
            unsafe { set.append_sorted(a)?; }
            let b = unsafe { SortedBatch::from_sorted(Buffer::from_host(pool.clone(), &other)?, other.len())? };
            cold()?;
            begin.record(pool.stream())?;
            let (_, elapsed) = measure(&pool, || unsafe { set.append_sorted(b) })?;
            end.record(pool.stream())?;
            pool.stream().synchronize()?;
            let gpu = (begin.elapsed_ms(&end)? as f64 * 1e6) as u64;
            if set.run_lengths().len() != 1 || plain_keys(&set)? != expected { return Err("scaled engine merge oracle mismatch".into()); }
            (elapsed, gpu, 2 * expected.len() * std::mem::size_of::<W>(), set.allocated_bytes())
        };
        results.insert(name.into(), json!({"wall_ns":wall_ns, "gpu_ns":gpu_ns,
            "ns_per_row":gpu_ns as f64 / expected.len() as f64,
            "logical_gb_s":(2 * expected.len() * std::mem::size_of::<W>()) as f64 / gpu_ns as f64,
            "resident_gb_s":moved_bytes as f64 / gpu_ns as f64,
            "resident_moved_bytes":moved_bytes, "output_bytes":output_bytes,
            "pool_peak":pool.memory_stats().peak_bytes, "oracle":"exact full sorted output"}));
        gpu_datalog_runtime::radix_run::release_set_scratch();
        gpu_datalog_runtime::packed_run::release_sort_scratch();
    }
    let after = tenant()?;
    Ok(json!({"state":spec.id,"rows":expected.len(),"owner_rows":owner.len(),"other_rows":other.len(),
        "delta_denominator":denominator,"word_bytes":spec.word_bytes,"configurations":results,
        "tenant_before":before,"tenant_after":after,"timing_indicative":foreign(&before)||foreign(&after),
        "protocol":"Real-state distribution replicated with disjoint leading-field offsets; every (denominator+1)th key assigned to other run. Same-process E/P/C; 256 MiB nonconstant device writes evict L2 before each merge. Input construction, eviction and exact host oracle excluded. GPU interval includes host submission/allocation gaps; E/P publication includes plain adoption, C uses presealed inputs. Resident bytes are input+output allocations, not measured DRAM traffic."}))
}

#[cuda_host::cuda_module]
mod eviction {
    use cuda_device::{kernel, launch_bounds, launch_contract, thread};
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn touch(output: *mut u64, rows: u64) {
        let mut i = thread::index_1d().get() as u64;
        while i < rows {
            unsafe { *output.add(i as usize) = i.wrapping_mul(0x9e3779b97f4a7c15); }
            i += thread::gridDim_x() as u64 * 256;
        }
    }
}
