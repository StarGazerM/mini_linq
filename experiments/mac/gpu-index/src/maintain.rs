//! Stable sorted-run maintenance and an exact radix-12 directory.
//!
//! Merge's modeled streaming traffic is `2 * 8 * (na + nb)` bytes: one
//! read and one write per Row. This is logical traffic, not measured DRAM
//! traffic; merge-path binary-search reads are additional (and cache-dependent).
//! Tiled merge also moves rows through threadgroup memory, not counted here.
//! Directory construction performs parallel lower bounds, not another full
//! input scan. Its search reads and 4097 u32 writes are separate from merge.
//! The directory is an exact radix-prefix index, NOT a RadixSpline model.

use crate::common::Gpu;
use metal::{BufferRef, CommandBufferRef, ComputePipelineState, NSRange};

const THREADS: usize = 256;
const DIRECTORY_ENTRIES: usize = 4097;

pub struct Maintain {
    capacity: usize,
    items: usize,
    direct: ComputePipelineState,
    tiled: ComputePipelineState,
    partition: ComputePipelineState,
    partitioned: ComputePipelineState,
    bounds: metal::Buffer,
    directory: ComputePipelineState,
    lookup: ComputePipelineState,
}

impl Maintain {
    /// `capacity` is the maximum combined run length. Items may be 4 or 8;
    /// tiled kernels use respectively 16 or 32 KiB of threadgroup storage.
    pub fn new(gpu: &Gpu, capacity: usize, items: usize) -> Self {
        assert!(capacity <= u32::MAX as usize);
        assert!(items == 4 || items == 8, "merge items must be 4 or 8");
        Self {
            capacity,
            items,
            direct: gpu.pipeline("maintain_merge_direct"),
            tiled: gpu.pipeline(if items == 4 {
                "maintain_merge_tiled_4"
            } else {
                "maintain_merge_tiled_8"
            }),
            partition: gpu.pipeline("maintain_partition"),
            partitioned: gpu.pipeline(if items == 4 {
                "maintain_merge_partitioned_4"
            } else {
                "maintain_merge_partitioned_8"
            }),
            bounds: gpu.buffer((capacity.div_ceil(THREADS * items) + 1) * 4),
            directory: gpu.pipeline("maintain_directory"),
            lookup: gpu.pipeline("maintain_lookup"),
        }
    }

    /// Merge sorted runs, choosing A before B on equal keys. Neither input may
    /// overlap the output. Payloads are copied verbatim, never used as tie keys.
    /// The modeled logical streaming byte count is `16 * (na + nb)`.
    pub fn merge(
        &self,
        gpu: &Gpu,
        cb: &CommandBufferRef,
        a: &BufferRef,
        b: &BufferRef,
        out: &BufferRef,
        na: usize,
        nb: usize,
        tiled: bool,
    ) {
        let n = na.checked_add(nb).expect("merged row count overflow");
        assert!(n <= self.capacity);
        if n == 0 {
            return;
        }
        gpu.dispatch(
            cb,
            if tiled { &self.tiled } else { &self.direct },
            &[a, b, out],
            &[na as u32, nb as u32, self.items as u32],
            n.div_ceil(THREADS * self.items),
            THREADS,
        );
    }

    /// Compute tile boundaries cooperatively in a separate dispatch, then
    /// merge without serial global searches inside each threadgroup.
    pub fn merge_partitioned(
        &self,
        gpu: &Gpu,
        cb: &CommandBufferRef,
        a: &BufferRef,
        b: &BufferRef,
        out: &BufferRef,
        na: usize,
        nb: usize,
    ) {
        let n = na.checked_add(nb).expect("merged row count overflow");
        assert!(n <= self.capacity);
        if n == 0 {
            return;
        }
        let tiles = n.div_ceil(THREADS * self.items);
        let args = [na as u32, nb as u32, self.items as u32];
        gpu.dispatch(
            cb,
            &self.partition,
            &[a, b, &self.bounds],
            &args,
            (tiles + 1).div_ceil(THREADS),
            THREADS,
        );
        gpu.dispatch(
            cb,
            &self.partitioned,
            &[a, b, out, &self.bounds],
            &args,
            tiles,
            THREADS,
        );
    }

    /// Write 4097 u32 offsets: lower_bound(p << 20), p = 0..4096,
    /// followed by n. Missing prefixes therefore have identical adjacent
    /// offsets. The directory must be rebuilt after changing the sorted rows.
    /// Even for n=0, all offsets are initialized; a blit avoids a compute dispatch.
    pub fn directory(
        &self,
        gpu: &Gpu,
        cb: &CommandBufferRef,
        rows: &BufferRef,
        dir: &BufferRef,
        n: usize,
    ) {
        assert!(n <= self.capacity);
        if n == 0 {
            let encoder = cb.new_blit_command_encoder();
            encoder.fill_buffer(dir, NSRange::new(0, (DIRECTORY_ENTRIES * 4) as u64), 0);
            encoder.end_encoding();
            return;
        }
        gpu.dispatch(
            cb,
            &self.directory,
            &[rows, dir],
            &[n as u32],
            DIRECTORY_ENTRIES.div_ceil(THREADS),
            THREADS,
        );
    }

    /// Exact full-multiplicity ranges [begin, end) for u32 query keys using a
    /// directory built for these rows. `out` contains uint2 values; misses are
    /// (n, n). Both binary searches are confined to the exact prefix bucket,
    /// and the upper bound compares keys directly (UINT_MAX never overflows).
    pub fn lookup(
        &self,
        gpu: &Gpu,
        cb: &CommandBufferRef,
        rows: &BufferRef,
        dir: &BufferRef,
        queries: &BufferRef,
        out: &BufferRef,
        n: usize,
        nq: usize,
    ) {
        assert!(n <= self.capacity);
        assert!(nq <= u32::MAX as usize);
        if nq == 0 {
            return;
        }
        if n == 0 {
            let encoder = cb.new_blit_command_encoder();
            encoder.fill_buffer(out, NSRange::new(0, nq as u64 * 8), 0);
            encoder.end_encoding();
            return;
        }
        gpu.dispatch(
            cb,
            &self.lookup,
            &[rows, dir, queries, out],
            &[n as u32, nq as u32],
            nq.div_ceil(THREADS),
            THREADS,
        );
    }
}
