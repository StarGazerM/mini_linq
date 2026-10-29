use crate::common::{Gpu, Row};
use metal::{Buffer, BufferRef, CommandBufferRef, ComputePipelineState};

const THREADS: usize = 256;
const SCAN_BLOCK: usize = THREADS * 4;

/// Stable full-width LSD radix sort. Scratch is reused by sequential encodes;
/// simultaneous execution on different command queues requires separate Sorts.
pub struct Sort {
    capacity: usize,
    tile: usize,
    bits: usize,
    bins: usize,
    scratch: Buffer,
    // Level zero is the digit-major tile histogram. Each subsequent level holds
    // the preceding level's block sums, including the terminal one-element sum.
    levels: Vec<Buffer>,
    histogram: ComputePipelineState,
    scan: ComputePipelineState,
    add: ComputePipelineState,
    scatter: ComputePipelineState,
}

impl Sort {
    /// Supported items/thread values are exactly 4 and 8 (1024/2048 rows/tile).
    /// All device allocations and pipeline creation happen here, not in encode.
    pub fn new(gpu: &Gpu, capacity: usize, items: usize) -> Self {
        Self::with_digit_bits(gpu, capacity, items, 4)
    }

    /// Compare four 8-bit passes against eight 4-bit passes.
    pub fn with_digit_bits(gpu: &Gpu, capacity: usize, items: usize, bits: usize) -> Self {
        Self::with_storage(gpu, capacity, items, bits, true)
    }

    /// Shared-source construction diagnostic: only GPU kernels initialize scratch.
    pub fn with_uninitialized_scratch(
        gpu: &Gpu,
        capacity: usize,
        items: usize,
        bits: usize,
    ) -> Self {
        Self::with_storage(gpu, capacity, items, bits, false)
    }

    fn with_storage(gpu: &Gpu, capacity: usize, items: usize, bits: usize, pretouch: bool) -> Self {
        assert!(bits == 4 || bits == 8);
        assert!(capacity <= u32::MAX as usize, "sort capacity must fit u32");
        assert!(matches!(items, 4 | 8), "sort items must be 4 or 8");
        let tile = THREADS * items;
        let scratch = gpu.allocate_buffer(capacity.max(1) * std::mem::size_of::<Row>(), pretouch);
        let bins = 1 << bits;
        let mut length = bins * capacity.div_ceil(tile);
        let mut levels = vec![gpu.allocate_buffer(length.max(2) * 4, pretouch)];
        loop {
            length = length.div_ceil(SCAN_BLOCK).max(1);
            levels.push(gpu.allocate_buffer(length.max(2) * 4, pretouch));
            if length == 1 {
                break;
            }
        }
        let histogram = gpu.pipeline(match (bits, items) {
            (4, 4) => "sort_histogram4",
            (4, 8) => "sort_histogram8",
            (8, 4) => "sort_histogram_byte4",
            (8, 8) => "sort_histogram_byte8",
            _ => unreachable!(),
        });
        let scatter = gpu.pipeline(match (bits, items) {
            (4, 4) => "sort_scatter4",
            (4, 8) => "sort_scatter8",
            (8, 4) => "sort_scatter_byte4",
            (8, 8) => "sort_scatter_byte8",
            _ => unreachable!(),
        });
        let scan = gpu.pipeline("sort_scan_blocks");
        let add = gpu.pipeline("sort_add_block_offsets");
        for pipeline in [&histogram, &scatter, &scan, &add] {
            assert_eq!(
                pipeline.thread_execution_width(),
                32,
                "sort requires 32-lane SIMD groups"
            );
            assert!(pipeline.max_total_threads_per_threadgroup() >= THREADS as u64);
        }
        Self {
            capacity,
            tile,
            bits,
            bins,
            scratch,
            levels,
            histogram,
            scan,
            add,
            scatter,
        }
    }

    /// Enumerate every allocation used by the radix implementation.
    pub fn scratch_buffers(&self) -> impl Iterator<Item = &BufferRef> {
        std::iter::once(self.scratch.as_ref()).chain(self.levels.iter().map(|b| b.as_ref()))
    }

    /// Replace only scratch storage, retaining all compiled pipelines.
    /// The caller must first wait for every command using the old storage.
    pub fn reallocate_scratch(&mut self, gpu: &Gpu, pretouch: bool) {
        self.resize_scratch(gpu, self.capacity, pretouch);
    }

    /// Change capacity between diagnostic sizes without recompiling pipelines.
    /// Existing encodes must have completed before calling this method.
    pub fn resize_scratch(&mut self, gpu: &Gpu, capacity: usize, pretouch: bool) {
        assert!(capacity <= u32::MAX as usize, "sort capacity must fit u32");
        self.capacity = capacity;
        self.scratch = gpu.allocate_buffer(capacity.max(1) * std::mem::size_of::<Row>(), pretouch);
        let mut length = self.bins * capacity.div_ceil(self.tile);
        let mut depth = 0;
        loop {
            let buffer = gpu.allocate_buffer(length.max(2) * 4, pretouch);
            if let Some(slot) = self.levels.get_mut(depth) {
                *slot = buffer;
            } else {
                self.levels.push(buffer);
            }
            depth += 1;
            if depth > 1 && length == 1 {
                break;
            }
            length = length.div_ceil(SCAN_BLOCK).max(1);
        }
        self.levels.truncate(depth);
    }

    /// Input and output must be distinct buffers with room for n rows. Input is
    /// never modified. Encodes every dependency on-GPU, without submitting or
    /// waiting, and always leaves the final result in output, including n == 1.
    pub fn encode(
        &self,
        gpu: &Gpu,
        cb: &CommandBufferRef,
        input: &BufferRef,
        output: &BufferRef,
        n: usize,
    ) {
        self.encode_with_dispatch(
            input,
            output,
            n,
            |pipeline, buffers, constants, groups, threads| {
                gpu.dispatch(cb, pipeline, buffers, constants, groups, threads);
            },
        );
    }

    /// Experiment-only encoder-topology control, using exactly the normal stage
    /// sequence. The callback must preserve dependencies between dispatches.
    /// Input and output must be distinct; the shared source is immutable.
    pub fn encode_with_dispatch(
        &self,
        input: &BufferRef,
        output: &BufferRef,
        n: usize,
        mut dispatch: impl FnMut(&metal::ComputePipelineStateRef, &[&BufferRef], &[u32], usize, usize),
    ) {
        assert!(n <= self.capacity, "sort exceeds allocated capacity");
        if n == 0 {
            return;
        }
        let tiles = n.div_ceil(self.tile);
        // At most four nonterminal levels are needed for u32-sized row counts.
        // A stack array avoids host allocations inside the timed encode path.
        let mut lengths = [0usize; 8];
        lengths[0] = tiles * self.bins;
        let mut depth = 1;
        while lengths[depth - 1] > SCAN_BLOCK {
            lengths[depth] = lengths[depth - 1].div_ceil(SCAN_BLOCK);
            depth += 1;
        }
        for pass in 0..32 / self.bits {
            let source: &BufferRef = if pass == 0 {
                input
            } else if pass % 2 == 1 {
                &self.scratch
            } else {
                output
            };
            let destination: &BufferRef = if pass % 2 == 0 { &self.scratch } else { output };
            let args = [n as u32, tiles as u32, (pass * self.bits) as u32];
            dispatch(
                &self.histogram,
                &[source, &self.levels[0]],
                &args,
                tiles,
                THREADS,
            );
            for level in 0..depth {
                dispatch(
                    &self.scan,
                    &[&self.levels[level], &self.levels[level + 1]],
                    &[lengths[level] as u32],
                    lengths[level].div_ceil(SCAN_BLOCK),
                    THREADS,
                );
            }
            for level in (0..depth - 1).rev() {
                dispatch(
                    &self.add,
                    &[&self.levels[level], &self.levels[level + 1]],
                    &[lengths[level] as u32],
                    lengths[level].div_ceil(SCAN_BLOCK),
                    THREADS,
                );
            }
            dispatch(
                &self.scatter,
                &[source, destination, &self.levels[0]],
                &args,
                tiles,
                THREADS,
            );
        }
    }

    /// Modeled logical device-buffer traffic, NOT hardware DRAM traffic.
    /// Includes key-only histogram loads, row scatter loads/stores, histogram
    /// writes, all scan reads/writes and block sums, propagated block offsets,
    /// and global bin-base loads per tile. Excludes threadgroup traffic.
    pub fn logical_bytes(&self, n: usize) -> u64 {
        assert!(n <= self.capacity, "sort exceeds allocated capacity");
        if n == 0 {
            return 0;
        }
        let tiles = n.div_ceil(self.tile) as u64;
        let mut length = tiles * self.bins as u64;
        let mut bytes = 20 * n as u64 + 4 * length + 4 * self.bins as u64 * tiles;
        loop {
            let blocks = length.div_ceil(SCAN_BLOCK as u64);
            // In-place exclusive scan and one block-sum store per block.
            bytes += 8 * length + 4 * blocks;
            if blocks == 1 {
                break;
            }
            // Propagation reads each parent offset once per child block.
            bytes += 8 * length + 4 * blocks;
            length = blocks;
        }
        bytes * (32 / self.bits) as u64
    }
}
