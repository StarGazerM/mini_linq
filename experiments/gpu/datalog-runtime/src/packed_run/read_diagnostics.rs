//! Opt-in lower-bound stage experiments; not an alternate production search.
use super::*;

/// Minimal one-CTA launch with the same output-allocation path as batched reads.
/// This is a launch floor, not an estimate of large-grid scheduling overhead.
pub fn launch_floor(pool: &Arc<Pool>) -> Result<Buffer<u64>> {
    let output = unsafe { Buffer::uninitialized(pool.clone(), 1)? };
    let module = unsafe { diagnostic_kernels::load(pool.context())? };
    module.empty(pool.stream(), &module.prepare_empty(LaunchConfig1D::new(1, 256, 0))?, output.ptr())?;
    Ok(output)
}

/// Ten-launch CUDA-event intervals, with descriptors and output staged before timing.
/// Stages stop after fences, fences+header/frames, lower bound, and lower bound+code checksum.
pub fn stages<W: Word>(state: &PackedSet<W>, probes: &Buffer<W>) -> Result<[u64; 4]> {
    let pool = probes.pool();
    if !pool.shares_memory(&state.pool) { return Err("diagnostic pool mismatch".into()); }
    if probes.is_empty() { return Ok([0; 4]); }
    let views = state.read_views()?;
    let output = unsafe { Buffer::<u64>::uninitialized(pool.clone(), probes.len())? };
    let module = unsafe { diagnostic_kernels::load(pool.context())? };
    let prepared = module.prepare_probe::<W>(config(probes.len()))?;
    let timing = Some(cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT);
    let start = pool.context().new_event(timing)?;
    let end = pool.context().new_event(timing)?;
    let mut times = [0; 4];
    for stage in 0..4 {
        module.probe::<W>(pool.stream(), &prepared, views.ptr(), state.runs().len() as u32, probes.ptr(), probes.len() as u64, stage, output.ptr())?;
        start.record(pool.stream())?;
        for _ in 0..10 { module.probe::<W>(pool.stream(), &prepared, views.ptr(), state.runs().len() as u32, probes.ptr(), probes.len() as u64, stage, output.ptr())?; }
        end.record(pool.stream())?;
        pool.stream().synchronize()?;
        times[stage as usize] = (start.elapsed_ms(&end)? as f64 * 1e6) as u64;
    }
    Ok(times)
}

#[cuda_host::cuda_module]
mod diagnostic_kernels {
    use super::PackedView;
    use crate::{packed_run::device, Word};
    use cuda_device::{kernel, launch_bounds, launch_contract, thread};
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn empty(output: *mut u64) {
        if thread::threadIdx_x() == 0 { unsafe { *output = 0; } }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn probe<W: Word>(views: *const PackedView<W>, count: u32, keys: *const W, rows: u64, stage: u32, output: *mut u64) {
        let mut i = thread::index_1d().get() as u64;
        while i < rows {
            unsafe {
                let key = *keys.add(i as usize);
                let mut sum = 0u64;
                let mut run = 0;
                while run < count {
                    let view = *views.add(run as usize);
                    if stage >= 2 {
                        let index = device::lower_bound(view, key);
                        sum = sum.wrapping_add(index);
                        if stage == 3 && index < view.rows {
                            sum = sum.wrapping_add(super::read_support::checksum_row(view, index, W::BITS));
                        }
                    } else if view.plain.is_null() {
                        let mut lo = 0;
                        let mut hi = view.blocks;
                        while lo < hi {
                            let mid = (lo + hi) / 2;
                            if (*view.fences.add(mid as usize)).compare(&key).is_lt() { lo = mid + 1; } else { hi = mid; }
                        }
                        let block = lo.saturating_sub(1);
                        sum = sum.wrapping_add(block);
                        if stage == 1 && view.rows != 0 {
                            let header = *view.headers.add(block as usize);
                            sum = sum.wrapping_add(header.offset + header.bits as u64 + header.encoding as u64);
                            let mut col = 0;
                            while col < view.columns {
                                let frame = *view.frames.add((block * view.columns as u64 + col as u64) as usize);
                                sum = sum.wrapping_add(frame.base).wrapping_add(frame.width() as u64);
                                col += 1;
                            }
                        }
                    }
                    run += 1;
                }
                *output.add(i as usize) = sum;
            }
            i += thread::gridDim_x() as u64 * 256;
        }
    }
}
