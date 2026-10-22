//! Stable, device-gated eight-bit LSD sorting of packed u64/u32 pairs.
//!
//! Launch every kernel with 256 threads, on the same stream as plan creation.
//! `radix_clear_histogram` (8 blocks), `radix_histogram` (a bounded nonzero grid),
//! and `radix_prefix_bins` (8 blocks) run once. Then enqueue `radix_init_pass`
//! and persistent `radix_scatter` (bounded nonzero grids) for each possible
//! pass, at most 0..8. The kernels skip unused passes and non-radix plans.
//! Histogram, prefix, and scatter require their exported dynamic shared sizes;
//! clearing and pass initialization require none. No static shared symbols.
//!
//! Scratch: HISTOGRAM_WORDS u32s, ceil(capacity / TILE) * DIGITS u64 lookback
//! words, and one u32 tile counter. The histogram becomes exclusive digit
//! bases in place. Lookback and counter storage are reused between passes.
//! Pair buffers must each accommodate capacity rows and must not alias the
//! other ping-pong buffer or scratch. Only stats.rows entries are accessed.
//! Initial pairs are in original row order. Swap input/output after each
//! enqueued pass; select by ceil((bits - begin_bit) / 8) parity on device.
//! Even passes read the original pair buffer, odd passes read the alternate.
//!
//! Each warp owns 256 consecutive input rows. Match-any ranking preserves
//! lane order within a round, round order within a warp, and then warp order.
//! A shared-memory exchange groups a tile by digit before coalesced output.
//! Monotonically assigned tiles publish their aggregate before looking back;
//! no tile can wait on a predecessor that has not yet been assigned to a CTA.
//! Status occupies the upper 32 bits, leaving all 32 count bits available.

pub(crate) const TILE: usize = 2048;
pub(crate) const THREADS: u32 = 256;
pub(crate) const DIGITS: usize = 256;
pub(crate) const PASSES: usize = 8;
pub(crate) const HISTOGRAM_WORDS: usize = PASSES * DIGITS;
pub(crate) const HISTOGRAM_SHARED_BYTES: u32 = 8192;
pub(crate) const PREFIX_SHARED_BYTES: u32 = 32;
pub(crate) const SCATTER_SHARED_BYTES: u32 = 34824;

#[cuda_device::cuda_module]
pub(crate) mod kernels {
    use crate::{RunStats, SortPlan};
    use cuda_device::atomic::{AtomicOrdering, BlockAtomicU32, DeviceAtomicU32, DeviceAtomicU64};
    use cuda_device::{
        device, kernel, launch_bounds, launch_contract, thread, warp, DynamicSharedArray,
    };

    #[device]
    #[inline(always)]
    unsafe fn radix_enabled(plan: *const SortPlan, stats: *const RunStats) -> bool {
        unsafe {
            (*plan).method == 2
                && (*plan).bits > (*plan).begin_bit
                && (*stats).invalid == 0
                && (*stats).sorted == 0
                && (*stats).rows > 1
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn radix_clear_histogram(
        plan: *const SortPlan,
        stats: *const RunStats,
        histogram: *mut u32,
    ) {
        if !unsafe { radix_enabled(plan, stats) } {
            return;
        }
        let mut bin = thread::index_1d().get();
        let stride = thread::gridDim_x() as usize * 256;
        while bin < 2048 {
            unsafe { histogram.add(bin).write(0) };
            bin += stride;
        }
    }

    /// Read every key once, accumulating all active digit histograms together.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1), dynamic_shared_range = (8192, 8192))]
    pub unsafe fn radix_histogram(
        keys: *const u64,
        plan: *const SortPlan,
        stats: *const RunStats,
        histogram: *mut u32,
    ) {
        if !unsafe { radix_enabled(plan, stats) } {
            return;
        }
        let tid = thread::threadIdx_x() as usize;
        let lane = warp::lane_id();
        let passes = (unsafe { (*plan).bits - (*plan).begin_bit } as usize + 7) / 8;
        let begin_bit = unsafe { (*plan).begin_bit } as usize;
        let bins = passes * 256;
        let shared = DynamicSharedArray::<u64>::get_raw() as *mut u32;
        let mut bin = tid;
        while bin < bins {
            unsafe { shared.add(bin).write(0) };
            bin += 256;
        }
        thread::sync_threads();

        let rows = unsafe { (*stats).rows } as usize;
        let mut chunk = thread::blockIdx_x() as usize * 256;
        let stride = thread::gridDim_x() as usize * 256;
        while chunk < rows {
            let row = chunk + tid;
            let active = row < rows;
            let key = if active {
                unsafe { keys.add(row).read() }
            } else {
                0
            };
            let mut pass = 0;
            while pass < passes {
                let digit = if active {
                    ((key >> (begin_bit + pass * 8)) & 255) as u32
                } else {
                    256
                };
                let matches = warp::match_any_sync(u32::MAX, digit);
                if active && lane == matches.trailing_zeros() {
                    unsafe {
                        BlockAtomicU32::from_ptr(shared.add(pass * 256 + digit as usize))
                            .fetch_add(matches.count_ones(), AtomicOrdering::Relaxed);
                    }
                }
                pass += 1;
            }
            chunk += stride;
        }
        thread::sync_threads();
        bin = tid;
        while bin < bins {
            let count = unsafe { shared.add(bin).read() };
            if count != 0 {
                unsafe {
                    DeviceAtomicU32::from_ptr(histogram.add(bin))
                        .fetch_add(count, AtomicOrdering::Relaxed);
                }
            }
            bin += 256;
        }
    }

    /// One CTA per pass converts its 256 counts to exclusive global bases.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1), dynamic_shared_range = (32, 32))]
    pub unsafe fn radix_prefix_bins(
        plan: *const SortPlan,
        stats: *const RunStats,
        histogram: *mut u32,
    ) {
        let pass = thread::blockIdx_x();
        if !unsafe { radix_enabled(plan, stats) }
            || pass >= 8
            || pass * 8 >= unsafe { (*plan).bits - (*plan).begin_bit }
        {
            return;
        }
        let tid = thread::threadIdx_x();
        let lane = warp::lane_id();
        let warp_id = tid / 32;
        let totals = DynamicSharedArray::<u64>::get_raw() as *mut u32;
        let bin = pass as usize * 256 + tid as usize;
        let count = unsafe { histogram.add(bin).read() };
        let mut inclusive = count;
        let mut distance = 1;
        while distance < 32 {
            let previous = warp::shuffle_up(inclusive, distance);
            if lane >= distance {
                inclusive += previous;
            }
            distance <<= 1;
        }
        if lane == 31 {
            unsafe { totals.add(warp_id as usize).write(inclusive) };
        }
        thread::sync_threads();
        let mut exclusive = inclusive - count;
        let mut previous_warp = 0;
        while previous_warp < warp_id {
            exclusive += unsafe { totals.add(previous_warp as usize).read() };
            previous_warp += 1;
        }
        unsafe { histogram.add(bin).write(exclusive) };
    }

    /// Reset only active tiles; stream order separates consecutive passes.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn radix_init_pass(
        pass: u32,
        plan: *const SortPlan,
        stats: *const RunStats,
        states: *mut u64,
        next_tile: *mut u32,
    ) {
        if !unsafe { radix_enabled(plan, stats) }
            || pass >= 8
            || pass * 8 >= unsafe { (*plan).bits - (*plan).begin_bit }
        {
            return;
        }
        let rows = unsafe { (*stats).rows } as usize;
        let words = ((rows + 2047) / 2048) * 256;
        let mut index = thread::index_1d().get();
        if index == 0 {
            unsafe { DeviceAtomicU32::from_ptr(next_tile).store(0, AtomicOrdering::Relaxed) };
        }
        let stride = thread::gridDim_x() as usize * 256;
        while index < words {
            unsafe {
                DeviceAtomicU64::from_ptr(states.add(index)).store(0, AtomicOrdering::Relaxed)
            };
            index += stride;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1), dynamic_shared_range = (34824, 34824))]
    pub unsafe fn radix_scatter(
        pass: u32,
        keys_in: *const u64,
        values_in: *const u32,
        keys_out: *mut u64,
        values_out: *mut u32,
        plan: *const SortPlan,
        stats: *const RunStats,
        bases: *const u32,
        states: *mut u64,
        next_tile: *mut u32,
    ) {
        if !unsafe { radix_enabled(plan, stats) }
            || pass >= 8
            || pass * 8 >= unsafe { (*plan).bits - (*plan).begin_bit }
        {
            return;
        }
        let tid = thread::threadIdx_x() as usize;
        let lane = warp::lane_id();
        let warp_id = tid / 32;
        let rows = unsafe { (*stats).rows } as usize;
        let shared = DynamicSharedArray::<u64>::get_raw() as *mut u64;
        let exchange_keys = shared;
        let exchange_values = unsafe { shared.add(2048) } as *mut u32;
        let warp_counts = unsafe { exchange_values.add(2048) };
        let local_starts = unsafe { warp_counts.add(2048) };
        let global_offsets = unsafe { local_starts.add(256) };
        let assigned_tile = unsafe { global_offsets.add(256) };
        // A bounded persistent grid makes a disabled pass O(SMs), not O(n).
        // Dynamic assignment also guarantees all lookback predecessors have
        // either completed or are owned by a resident CTA.
        loop {
            if tid == 0 {
                let tile = unsafe {
                    DeviceAtomicU32::from_ptr(next_tile).fetch_add(1, AtomicOrdering::Relaxed)
                };
                unsafe { assigned_tile.write(tile) };
            }
            thread::sync_threads();
            let tile = unsafe { assigned_tile.read() } as usize;
            let tile_start = tile * 2048;
            if tile_start >= rows {
                return;
            }
            let tile_rows = (rows - tile_start).min(2048);
            let shift = unsafe { (*plan).begin_bit } + pass * 8;
            let mut index = tid;
            while index < 2048 {
                unsafe { warp_counts.add(index).write(0) };
                index += 256;
            }
            thread::sync_threads();

            // Eight register-resident items per thread. Warp ownership is contiguous,
            // rather than block-striped, so its count prefix is a stable tile prefix.
            let mut keys = [0u64; 8];
            let mut values = [0u32; 8];
            let mut ranks = [0u32; 8];
            let mut round = 0;
            while round < 8 {
                let local = warp_id * 256 + round * 32 + lane as usize;
                let active = local < tile_rows;
                let key = if active {
                    unsafe { keys_in.add(tile_start + local).read() }
                } else {
                    0
                };
                let value = if active {
                    unsafe { values_in.add(tile_start + local).read() }
                } else {
                    0
                };
                let digit = if active {
                    ((key >> shift) & 255) as u32
                } else {
                    256
                };
                let matches = warp::match_any_sync(u32::MAX, digit);
                let leader = matches.trailing_zeros();
                let mut previous = 0;
                if active && lane == leader {
                    let count = unsafe { warp_counts.add(warp_id * 256 + digit as usize) };
                    previous = unsafe { count.read() };
                    unsafe { count.write(previous + matches.count_ones()) };
                }
                previous = warp::shuffle_sync(u32::MAX, previous, leader);
                keys[round] = key;
                values[round] = value;
                ranks[round] = previous + (matches & warp::lanemask_lt()).count_ones();
                // The next round can choose a different leader for this same bin.
                warp::sync_mask(u32::MAX);
                round += 1;
            }
            thread::sync_threads();

            // Thread d owns digit d: convert its eight warp counts to exclusive
            // warp prefixes, retaining the complete tile aggregate in a register.
            let mut count = 0u32;
            let mut owner_warp = 0;
            while owner_warp < 8 {
                let slot = unsafe { warp_counts.add(owner_warp * 256 + tid) };
                let warp_count = unsafe { slot.read() };
                unsafe { slot.write(count) };
                count += warp_count;
                owner_warp += 1;
            }
            let state = unsafe { DeviceAtomicU64::from_ptr(states.add(tile * 256 + tid)) };
            state.store((1u64 << 32) | count as u64, AtomicOrdering::Release);

            // Compute a local digit base for the shared-memory grouping exchange.
            let mut inclusive = count;
            let mut distance = 1;
            while distance < 32 {
                let previous = warp::shuffle_up(inclusive, distance);
                if lane >= distance {
                    inclusive += previous;
                }
                distance <<= 1;
            }
            if lane == 31 {
                unsafe { global_offsets.add(warp_id).write(inclusive) };
            }
            thread::sync_threads();
            let mut local_base = inclusive - count;
            owner_warp = 0;
            while owner_warp < warp_id {
                local_base += unsafe { global_offsets.add(owner_warp).read() };
                owner_warp += 1;
            }
            unsafe { local_starts.add(tid).write(local_base) };
            // All warp totals must be consumed before global_offsets is repurposed.
            thread::sync_threads();

            let mut preceding = 0u32;
            let mut predecessor = tile;
            while predecessor != 0 {
                predecessor -= 1;
                let prior =
                    unsafe { DeviceAtomicU64::from_ptr(states.add(predecessor * 256 + tid)) };
                let mut word = prior.load(AtomicOrdering::Acquire);
                while word == 0 {
                    word = prior.load(AtomicOrdering::Acquire);
                }
                preceding += word as u32;
                if word >> 32 == 2 {
                    break;
                }
            }
            state.store(
                (2u64 << 32) | (preceding + count) as u64,
                AtomicOrdering::Release,
            );
            let base = unsafe { bases.add(pass as usize * 256 + tid).read() };
            // Keep an actual global digit start, not a possibly negative adjustment.
            unsafe { global_offsets.add(tid).write(base + preceding) };
            thread::sync_threads();

            round = 0;
            while round < 8 {
                let local = warp_id * 256 + round * 32 + lane as usize;
                if local < tile_rows {
                    let key = keys[round];
                    let digit = ((key >> shift) & 255) as usize;
                    let local_base = unsafe { local_starts.add(digit).read() };
                    let warp_base = unsafe { warp_counts.add(warp_id * 256 + digit).read() };
                    let destination = (local_base + warp_base + ranks[round]) as usize;
                    unsafe {
                        exchange_keys.add(destination).write(key);
                        exchange_values.add(destination).write(values[round]);
                    }
                }
                round += 1;
            }
            thread::sync_threads();
            index = tid;
            while index < tile_rows {
                let key = unsafe { exchange_keys.add(index).read() };
                let value = unsafe { exchange_values.add(index).read() };
                let digit = ((key >> shift) & 255) as usize;
                let local_base = unsafe { local_starts.add(digit).read() };
                let global_base = unsafe { global_offsets.add(digit).read() };
                let destination = global_base as usize + index - local_base as usize;
                unsafe {
                    keys_out.add(destination).write(key);
                    values_out.add(destination).write(value);
                }
                index += 256;
            }
            thread::sync_threads();
        }
    }
}
