use super::*;
use std::time::Duration;

// RCCL validates the *exact* partition length, even when the pool allocation
// is larger. The guard remains locked until both merge kernels are enqueued.
struct PartitionPrefix<'a> { buffer: &'a mut Buffer<usize>, len: usize }
unsafe impl cuda_host::KernelSliceArg for PartitionPrefix<'_> {
    type Elem = usize;
    fn cu_deviceptr(&self) -> u64 { self.buffer.cu_deviceptr() }
    fn len(&self) -> usize { self.len }
}
unsafe impl cuda_host::KernelSliceArgMut for PartitionPrefix<'_> {}
impl rccl_cub::WorkspaceBuffer<usize> for PartitionPrefix<'_> {
    type Range = Self;
    fn range_mut(&mut self) -> &mut Self { self }
}


pub(super) struct MergeEvents(pub(super) [cuda_core::CudaEvent; 5]);
impl MergeEvents {
    pub(super) fn new(pool: &Pool) -> Result<Self> {
        let timing = Some(cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT);
        Ok(Self([pool.context().new_event(timing)?, pool.context().new_event(timing)?,
            pool.context().new_event(timing)?, pool.context().new_event(timing)?, pool.context().new_event(timing)?]))
    }
    pub(super) fn elapsed(&self, plain: bool) -> Result<(Duration, Duration, Duration)> {
        Ok((if plain { Duration::ZERO } else { Duration::from_secs_f64(self.0[0].elapsed_ms(&self.0[1])? as f64 / 1000.0) },
            if plain { Duration::ZERO } else { Duration::from_secs_f64(self.0[1].elapsed_ms(&self.0[2])? as f64 / 1000.0) },
            Duration::from_secs_f64(self.0[3].elapsed_ms(&self.0[4])? as f64 / 1000.0)))
    }
}

impl<W: Word> PackedRun<W> {
    /// Merge disjoint key sets without whole-run unpacked scratch.
    ///
    /// # Safety
    /// The two sorted unique inputs must have no key in common, as at admission.
    pub unsafe fn merge(&self, other: &Self) -> Result<Self> {
        self.merge_with_events(other, None, SMALL_RUN_ROWS, MergeFrames::Metadata)
    }
    /// Explicit replay policy; normal callers use `merge`.
    ///
    /// # Safety
    /// Both inputs are sorted unique and their key sets are disjoint.
    pub unsafe fn merge_with_frames(&self, other: &Self, policy: MergeFrames) -> Result<Self> {
        self.merge_with_events(other, None, SMALL_RUN_ROWS, policy)
    }
    pub(super) unsafe fn merge_with_events(&self, other: &Self, events: Option<&MergeEvents>, plain_limit: usize, policy: MergeFrames) -> Result<Self> {
        let _merge = cq_gpu_runtime::nvtx::range(c"packed_merge");
        if self.packing != other.packing || self.block_rows != other.block_rows || !self.pool.shares_memory(&other.pool) {
            return Err("packed merge layout or pool mismatch".into());
        }
        let rows = self.rows.checked_add(other.rows).ok_or("packed merge size overflow")?;
        crate::phases::admission_scope!(c"admit/packed-merge", rows);
        if rows <= plain_limit {
            if let (Some(a), Some(b)) = (&self.plain, &other.plain) {
                let module = &self.modules.runs;
                if let Some(e) = events { e.0[3].record(self.pool.stream())?; }
                let batch = if rows <= 131_072 {
                    // For a few tiles, one launch avoids RCCL's partition pass.
                    let output = Buffer::uninitialized(self.pool.clone(), rows)?;
                    module.merge_wide::<W>(self.pool.stream(), &module.prepare_merge_wide::<W>(config(rows.div_ceil(8)))?,
                        a.view(), b.view(), output.ptr(), 8)?;
                    crate::SortedBatch::from_sorted(output, rows)?
                } else if W::BITS <= 128 && Arc::ptr_eq(&self.pool, &self.modules._pool) {
                    let launch = self.modules.rccl.on(self.pool.stream())?;
                    let count = rccl_cub::thrust_integration::device_merge_partition_count::<W>(a.len(), b.len())?;
                    let mut cache = self.modules.partitions.lock().map_err(|_| "poisoned merge partition storage")?;
                    if cache.as_ref().is_none_or(|p| p.len() < count) {
                        *cache = Some(Buffer::uninitialized(self.pool.clone(), count)?);
                    }
                    let prefix = PartitionPrefix { buffer: cache.as_mut().unwrap(), len: count };
                    let mut workspace = rccl_cub::DeviceMergeWorkspace::__from_partitions(prefix);
                    let mut out = super::sort::OutputBuffer::<W>::new(self.pool.clone(), rows)?;
                    rccl_cub::device_merge_keys(&launch, a, b, &mut workspace, &mut out, |x: &W, y: &W| x.compare(y))?;
                    crate::SortedBatch::from_sorted(out.buffer, rows)?
                } else {
                    let params = crate::RadixRunParams { ratio: 4, radix_bits: 0, merge_tile_rows: 32, block_threads: 256 };
                    crate::runs::SetOps { pool: &self.pool, module, params, active_bits: self.packing.bits() }.merge(a, b)?
                };
                if let Some(e) = events { e.0[4].record(self.pool.stream())?; }
                return Self::from_plain(batch, self.packing.clone(), self.block_rows, self.modules.clone());
            }
        }
        self.merge_leaves(other, events, policy)
    }
}

pub(super) mod merge_kernels {
    use super::super::{device, PackedView};
    use crate::Word;

    /// Assemble one output u64 from a contiguous sorted tile (global or shared).
    /// Each payload word has one writer, with no searches or row-sized slots.
    pub(crate) unsafe fn payload_word<W: Word>(out: PackedView<W>, block: u64, shared: *const W, index: u64) -> u64 {
        let h = *out.headers.add(block as usize);
        if h.encoding == 0 { return (*shared.add((index / (W::BITS / 64) as u64) as usize)).lane((index % (W::BITS / 64) as u64) as u32); }
        let begin = index * 64;
        let end = (begin + 64).min(h.rows as u64 * h.bits as u64);
        let mut row = begin / h.bits as u64;
        let mut value = 0;
        while row * (h.bits as u64) < end {
            let word = *shared.add(row as usize);
            let mut field_end = (row + 1) * h.bits as u64;
            let mut col = 0;
            while col < out.columns {
                let frame = *out.frames.add((block * out.columns as u64 + col as u64) as usize);
                let field_start = field_end - frame.width() as u64;
                if field_end > begin && field_start < end && frame.width() != 0 {
                    let field = *out.fields.add(col as usize);
                    let residual = word.extract(field.shift, field.bits) - frame.base;
                    let start = begin.max(field_start);
                    let stop = end.min(field_end);
                    let width = (stop - start) as u32;
                    let fragment = (residual >> (start - field_start)) & (u64::MAX >> (64 - width));
                    value |= fragment << (start - begin);
                }
                field_end = field_start; col += 1;
            }
            row += 1;
        }
        value
    }
    /// The fence alone is maintained in the run-wide identity frame.
    pub(crate) unsafe fn write_fence<W: Word>(source: PackedView<W>, index: u64, out: PackedView<W>, block: u64) -> bool {
        if !source.plain.is_null() { *(out.fences as *mut W).add(block as usize) = *source.plain.add(index as usize); return false; }
        let source_block = device::row_block(source, index);
        if index == (*source.headers.add(source_block as usize)).first {
            *(out.fences as *mut W).add(block as usize) = *source.fences.add(source_block as usize); return false;
        }
        let mut cursor = device::source_codes(source, index, out.fields, out.columns);
        let mut fence = W::zero();
        // DECODE I: one output fence is rebased to the fixed run-wide identity frame; avoiding this requires changing the fence representation.
        while let Some((code, frame, field)) = cursor.next() { fence.insert(field.shift, field.bits, code + frame.base); }
        *(out.fences as *mut W).add(block as usize) = fence;
        (*source.headers.add(source_block as usize)).encoding != 0
    }
}
