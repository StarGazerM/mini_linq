use super::*;
use crate::SortedBatch;

impl<W: Word> PackedSet<W> {
    /// Reorder full-key fields on the execution stream. Sort residuals in a
    /// common frame, seal them there, then transfer the origin to the output
    /// frames and fences. No identity-domain row is produced for sorting.
    pub(crate) fn permuted(
        pool: Arc<Pool>, packing: Packing, order: &[usize], ratio: usize,
        input: &[Arc<PackedRun<W>>], sort: impl Fn(&Buffer<W>, usize) -> Result<SortedBatch<W>>,
    ) -> Result<Self> {
        let mut set = Self::new(pool.clone(), packing.clone(), DEFAULT_BLOCK_ROWS, ratio)?;
        let modules = set.modules.clone();
        let codec = &modules.codec;
        let mut destinations = vec![0u32; order.len()];
        for (destination, &source) in order.iter().enumerate() { destinations[source] = destination as u32; }
        let destinations = Buffer::from_host_async(pool.clone(), &destinations)?;
        let fields = Buffer::from_host_async(pool.clone(), &packing.packed_fields().collect::<Vec<_>>())?;
        let origins = unsafe { Buffer::<u64>::uninitialized(pool.clone(), order.len())? };
        for run in input {
            let rows = run.len();
            if let Some(batch) = run.plain_batch() {
                let words = run.packing().permute_device_on(&pool, batch.buffer(), rows, order, 256)?;
                unsafe { set.append_sorted(&sort(&words, rows)?)?; }
                continue;
            }
            let view = run.view();
            let words = unsafe { Buffer::uninitialized(pool.clone(), rows)? };
            codec.common_origins::<W>(pool.stream(), &codec.prepare_common_origins::<W>(LaunchConfig1D::new(view.columns, 32, 0))?, view, origins.ptr())?;
            codec.permute_codes::<W>(pool.stream(), &codec.prepare_permute_codes::<W>(config(rows))?,
                view, origins.ptr(), destinations.ptr(), fields.ptr(), words.ptr())?;
            let sorted = sort(&words, rows)?;
            let result = unsafe { PackedRun::seal_in_frame(sorted.buffer(), rows, &packing, DEFAULT_BLOCK_ROWS, modules.clone(), true)? };
            codec.shift_frames::<W>(pool.stream(), &codec.prepare_shift_frames::<W>(config(result.headers.len()))?,
                result.view(), origins.ptr(), destinations.ptr(), result.frames.ptr().cast(), result.fences.ptr())?;
            unsafe { set.append_reordered(Arc::new(result))?; }
        }
        Ok(set)
    }
}
