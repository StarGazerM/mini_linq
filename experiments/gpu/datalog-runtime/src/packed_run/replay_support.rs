//! Batched lookup adapters for the replay's rule-read proxy.
use super::*;
use crate::{DeviceRadixRun, RadixKeysView};

pub fn plain_ranges<W: Word>(state: &DeviceRadixRun<W, u64, ()>, low: &Buffer<W>, high: &Buffer<W>) -> Result<Buffer<[u64; 2]>> {
    let _lookup = cq_gpu_runtime::nvtx::range(c"engine_ranges");
    let pool = state.pool();
    if low.len() != high.len() || !pool.shares_memory(low.pool()) || !pool.shares_memory(high.pool()) { return Err("lookup pool/shape mismatch".into()); }
    let output = unsafe { Buffer::uninitialized(pool.clone(), low.len())? };
    if !low.is_empty() {
        let snapshot = state.snapshot()?;
        let module = unsafe { lookup_kernels::load(pool.context())? };
        module.plain::<W>(pool.stream(), &module.prepare_plain::<W>(config(low.len()))?, snapshot.keys_view(), low.ptr(), high.ptr(), low.len() as u64, output.ptr())?;
    }
    Ok(output)
}
pub fn packed_ranges<W: Word>(state: &PackedSet<W>, low: &Buffer<W>, high: &Buffer<W>) -> Result<Buffer<[u64; 2]>> {
    let _lookup = cq_gpu_runtime::nvtx::range(c"packed_ranges");
    let pool = low.pool();
    if low.len() != high.len() || !pool.shares_memory(high.pool()) || !pool.shares_memory(&state.pool) { return Err("lookup pool/shape mismatch".into()); }
    let output = unsafe { Buffer::uninitialized(pool.clone(), low.len())? };
    if !low.is_empty() {
        let views = state.read_views()?;
        let module = unsafe { lookup_kernels::load(pool.context())? };
        if state.runs().iter().all(|run| run.plain.is_some()) {
            module.storage_plain::<W>(pool.stream(), &module.prepare_storage_plain::<W>(config(low.len()))?, views.ptr(), state.runs().len() as u32, low.ptr(), high.ptr(), low.len() as u64, output.ptr())?;
        } else {
            module.packed::<W>(pool.stream(), &module.prepare_packed::<W>(config(low.len()))?, views.ptr(), state.runs().len() as u32, low.ptr(), high.ptr(), low.len() as u64, output.ptr())?;
        }
    }
    Ok(output)
}
#[cuda_host::cuda_module]
mod lookup_kernels {
    use super::{PackedView, RadixKeysView};
    use crate::{radix_run::device as plain, packed_run::device as packed, Word};
    use cuda_device::{kernel, launch_bounds, launch_contract, thread};
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn plain<W: Word>(view: RadixKeysView<W, u64>, low: *const W, high: *const W, rows: u64, output: *mut [u64; 2]) {
        let mut i = thread::index_1d().get() as u64;
        while i < rows {
            unsafe {
                let mut begin = 0;
                let mut end = 0;
                let mut run = 0;
                while run < view.run_count {
                    let (a, b) = plain::interval(plain::run(view, run), *low.add(i as usize), *high.add(i as usize));
                    begin += a; end += b; run += 1;
                }
                *output.add(i as usize) = [begin, end];
            }
            i += thread::gridDim_x() as u64 * 256;
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn packed<W: Word>(views: *const PackedView<W>, count: u32, low: *const W, high: *const W, rows: u64, output: *mut [u64; 2]) {
        ranges::<W, false>(views, count, low, high, rows, output);
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn storage_plain<W: Word>(views: *const PackedView<W>, count: u32, low: *const W, high: *const W, rows: u64, output: *mut [u64; 2]) {
        ranges::<W, true>(views, count, low, high, rows, output);
    }
    #[cuda_device::device]
    fn ranges<W: Word, const PLAIN: bool>(views: *const PackedView<W>, count: u32, low: *const W, high: *const W, rows: u64, output: *mut [u64; 2]) {
        let mut i = thread::index_1d().get() as u64;
        while i < rows {
            unsafe {
                let mut begin = 0;
                let mut end = 0;
                let mut run = 0;
                while run < count {
                    let (a, b) = packed::read_range::<W, PLAIN>(*views.add(run as usize), *low.add(i as usize), *high.add(i as usize));
                    begin += a; end += b; run += 1;
                }
                *output.add(i as usize) = [begin, end];
            }
            i += thread::gridDim_x() as u64 * 256;
        }
    }
}
