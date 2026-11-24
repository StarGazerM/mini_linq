use super::*;
use crate::SortedBatch;
use std::time::Instant;

/// Shared-memory admission ceiling, including four-lane keys and scan scratch.
pub const MAX_SMALL_CANDIDATES: usize = 1024;

impl<W: Word> PackedSet<W> {
    /// Sort, deduplicate and subtract a small arbitrary-order candidate prefix.
    ///
    /// # Safety
    /// The prefix is initialized, has zero unused packing bits, and is ordered
    /// on this set's pool stream. Duplicate candidates are permitted.
    pub unsafe fn fresh_small(&mut self, input: &Buffer<W>, rows: usize) -> Result<SortedBatch<W>> {
        let _fused = cq_gpu_runtime::nvtx::range(c"packed_sort_subtract_fused");
        if rows > MAX_SMALL_CANDIDATES || rows > input.len() || !self.pool.shares_memory(input.pool()) {
            return Err("invalid small admission input".into());
        }
        if rows == 0 {
            return SortedBatch::from_sorted(Buffer::uninitialized(self.pool.clone(), 0)?, 0);
        }
        if self.profile { self.pool.stream().synchronize()?; }
        let start = Instant::now();
        let output = Buffer::uninitialized(self.pool.clone(), rows)?;
        let count = Buffer::<u64>::uninitialized(self.pool.clone(), 1)?;
        let views = Buffer::from_host_async(self.pool.clone(), &self.runs.iter().map(|run| run.view()).collect::<Vec<_>>())?;
        let size = rows.next_power_of_two();
        let threads = if size > 256 { 512 } else { 256 };
        let cfg = cuda_core::simt::LaunchConfig { grid_dim: (1, 1, 1), block_dim: (threads, 1, 1),
            shared_mem_bytes: (size * (std::mem::size_of::<W>() + 4)) as u32 };
        self.modules.small.fresh::<W>(self.pool.stream(), cfg, input.ptr(), rows as u32,
            size as u32, threads, views.ptr(), self.runs.len() as u32, output.ptr(), count.ptr())?;
        let fresh = count.to_host()?[0] as usize;
        if self.profile {
            let elapsed = start.elapsed();
            self.times.subtract += elapsed;
            self.times.fused_small += elapsed;
        }
        self.times.subtract_rows += rows;
        self.times.subtractions += 1;
        SortedBatch::from_sorted(output, fresh)
    }
}

#[cuda_host::cuda_module]
pub(super) mod small_kernels {
    use super::super::{device, PackedView};
    use crate::Word;
    use cuda_device::{kernel, launch_bounds, thread, DynamicSharedArray};

    #[kernel]
    #[launch_bounds(512)]
    pub fn fresh<W: Word>(input: *const W, rows: u32, size: u32, threads: u32, views: *const PackedView<W>,
        runs: u32, output: *mut W, count: *mut u64) {
        let keys = DynamicSharedArray::<W>::get();
        let flags = unsafe { keys.add(size as usize).cast::<u32>() };
        let tid = thread::threadIdx_x();
        unsafe {
            let mut i = tid;
            while i < size {
                let mut key = W::zero();
                if i < rows { key = *input.add(i as usize); }
                else {
                    let mut lane = 0;
                    while lane < W::BITS / 64 { key.set_lane(lane, u64::MAX); lane += 1; }
                }
                *keys.add(i as usize) = key;
                i += threads;
            }
            thread::sync_threads();
            let mut width = 2;
            while width <= size {
                let mut stride = width / 2;
                while stride != 0 {
                    i = tid;
                    while i < size {
                        let other = i ^ stride;
                        if other > i {
                            let a = *keys.add(i as usize);
                            let b = *keys.add(other as usize);
                            let swap = if i & width == 0 { a.compare(&b).is_gt() } else { a.compare(&b).is_lt() };
                            if swap { *keys.add(i as usize) = b; *keys.add(other as usize) = a; }
                        }
                        i += threads;
                    }
                    thread::sync_threads();
                    stride /= 2;
                }
                width *= 2;
            }
            i = tid;
            while i < rows {
                let key = *keys.add(i as usize);
                let mut keep = i == 0 || !key.compare(&*keys.add(i as usize - 1)).is_eq();
                let mut run = 0;
                while keep && run < runs {
                    let view = *views.add(run as usize);
                    keep = !device::contains(view, key);
                    run += 1;
                }
                *flags.add(i as usize) = keep as u32;
                i += threads;
            }
            thread::sync_threads();
            if tid == 0 {
                let mut total = 0;
                i = 0;
                while i < rows {
                    let keep = *flags.add(i as usize) != 0;
                    *flags.add(i as usize) = if keep { total } else { u32::MAX };
                    total += keep as u32;
                    i += 1;
                }
                *count = total as u64;
            }
            thread::sync_threads();
            i = tid;
            while i < rows {
                let pos = *flags.add(i as usize);
                if pos != u32::MAX { *output.add(pos as usize) = *keys.add(i as usize); }
                i += threads;
            }
        }
    }
}
