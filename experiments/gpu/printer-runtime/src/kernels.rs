#[cuda_device::cuda_module]
pub(crate) mod runtime_kernels {
    use crate::{
        FrontierView, FusedRetryMetadata, FusedRetryView, OutputCount, OutputView, RetryMetadata,
        TableView, column, column_stripe, flag_error, reserve, table_len, write_output,
    };
    use cuda_device::atomic::{AtomicOrdering, DeviceAtomicU64};
    use cuda_device::{kernel, launch_bounds, launch_contract, thread};
    use gpu_device_columns::ColumnView;

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn initialize_count(metadata: *mut OutputCount, rows: u64) {
        if thread::index_1d().get() == 0 {
            unsafe {
                (*metadata).rows = rows;
                (*metadata).error = 0;
                (*metadata).padding = 0;
            }
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn initialize_view(views: *mut ColumnView, index: usize, view: ColumnView) {
        if thread::index_1d().get() == 0 {
            unsafe {
                views.add(index).write(view);
            }
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn initialize_pointer(pointers: *mut u64, index: usize, pointer: u64) {
        if thread::index_1d().get() == 0 {
            unsafe {
                pointers.add(index).write(pointer);
            }
        }
    }
    #[inline(always)]
    unsafe fn validated_output_rows(output: OutputView) -> u64 {
        let count = unsafe { output.count.read() };
        // Capacity is checked only after all candidates have been counted, so
        // it cannot mask a fatal error raised during the bounded launch.
        if count > u32::MAX as u64 {
            unsafe {
                flag_error(output, 5);
            }
        } else if count > output.capacity {
            unsafe {
                flag_error(output, 4);
            }
        }
        count
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn validate_output(output: OutputView) {
        if thread::index_1d().get() == 0 {
            unsafe {
                validated_output_rows(output);
            }
        }
    }
    #[inline(always)]
    unsafe fn dependency_error(dependencies: *const u64, count: usize) -> u32 {
        let mut error = 0;
        let mut index = 0;
        while index < count {
            let status = unsafe { dependencies.add(index).read() } as *const OutputCount;
            if error == 0 {
                error = unsafe { (*status).error };
            }
            index += 1;
        }
        error
    }
    /// Check the current output and every retained table/run status together.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn retry_metadata(
        output: OutputView,
        dependencies: *const u64,
        dependency_count: usize,
        metadata: *mut RetryMetadata,
    ) {
        if thread::index_1d().get() == 0 {
            let rows = unsafe { validated_output_rows(output) };
            let dependency_error = unsafe { dependency_error(dependencies, dependency_count) };
            unsafe {
                metadata.write(RetryMetadata {
                    rows,
                    error: output.error.read(),
                    dependency_error,
                });
            }
        }
    }
    /// Capacity is a separate growth flag; fatal arena/output/dependency errors
    /// cannot be hidden by an earlier exhausted ticket or full probe table.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn fused_retry_metadata(
        output: OutputView,
        retry: FusedRetryView,
        dependencies: *const u64,
        dependency_count: usize,
        metadata: *mut FusedRetryMetadata,
    ) {
        if thread::index_1d().get() == 0 {
            let rows = unsafe { output.count.read() };
            let key_rows = unsafe { retry.keys.count.read() };
            let key_error = unsafe { retry.keys.error.read() };
            if rows > u32::MAX as u64 {
                unsafe {
                    flag_error(output, 5);
                }
            }
            if rows > output.capacity || key_rows > retry.keys.capacity {
                unsafe {
                    retry.overflow.write(1);
                }
            }
            #[cfg(feature = "dedup-diagnostics")]
            let mut diagnostics = [0u64; 6];
            #[cfg(feature = "dedup-diagnostics")]
            {
                let mut block = 0;
                while block < 568 {
                    let counters = unsafe { retry.set.counters.add(block * 6) };
                    let mut index = 0;
                    while index < 5 {
                        diagnostics[index] += unsafe { counters.add(index).read() };
                        index += 1;
                    }
                    diagnostics[5] = diagnostics[5].max(unsafe { counters.add(5).read() });
                    block += 1;
                }
            }
            unsafe {
                metadata.write(FusedRetryMetadata {
                    rows,
                    candidates: retry.candidates.read(),
                    distinct: key_rows.min(retry.keys.capacity),
                    error: output.error.read(),
                    key_error: if key_error == 4 && key_rows > retry.keys.capacity {
                        0
                    } else {
                        key_error
                    },
                    dependency_error: dependency_error(dependencies, dependency_count),
                    overflow: retry.overflow.read(),
                    #[cfg(feature = "dedup-diagnostics")]
                    diagnostics,
                });
            }
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn count_u32(table: TableView, count: *mut u32) {
        if thread::index_1d().get() == 0 {
            unsafe {
                count.write(table_len(table) as u32);
            }
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn hash_run_keys(
        table: TableView,
        slots: *const u64,
        width: usize,
        mask: u64,
        output: *mut i64,
    ) {
        let mut row = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * 256;
        let count = unsafe { table_len(table) };
        while row < count {
            let mut hash = crate::HASHED_RUN_SEED;
            let mut index = 0;
            while index < width {
                let slot = unsafe { slots.add(index).read() } as usize;
                hash = crate::hashed_run_step(hash, unsafe { column(table, slot, row) });
                index += 1;
            }
            unsafe {
                output.add(row as usize).write((hash & mask) as i64);
            }
            row += stride;
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn run_status(stats: *const gpu_blocked_run::RunStats, status: *mut OutputCount) {
        if thread::index_1d().get() == 0 && unsafe { (*stats).invalid } != 0 {
            unsafe {
                (*status).error = 7;
            }
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn decode_column(table: TableView, index: usize, output: *mut i64) {
        let lane = thread::threadIdx_x();
        let mut base = thread::blockIdx_x() as u64 * 256;
        let stride = thread::gridDim_x() as u64 * 256;
        let count = unsafe { table_len(table) };
        while base < count {
            let row = base + lane as u64;
            let value = unsafe {
                column_stripe(
                    table,
                    index,
                    row / 1024,
                    lane,
                    ((row % 1024) / 256) as usize,
                )
            };
            if row < count {
                unsafe {
                    output.add(row as usize).write(value);
                }
            }
            base += stride;
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn frontier_initialize(frontier: FrontierView, roots: u64) {
        if thread::index_1d().get() == 0 {
            unsafe {
                frontier.cursor.write(0);
                frontier.remaining.write(roots);
            }
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn membership_extrema_initialize(bounds: *mut u64) {
        if thread::index_1d().get() == 0 {
            unsafe {
                bounds.write(u64::MAX);
                bounds.add(1).write(0);
            }
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn membership_extrema(table: TableView, column_index: usize, bounds: *mut u64) {
        let mut row = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * 256;
        let count = unsafe { table_len(table) };
        let mut low = u64::MAX;
        let mut high = 0u64;
        while row < count {
            // Flipping the sign bit preserves signed order under unsigned atomics.
            let key = (unsafe { column(table, column_index, row) } as u64) ^ (1u64 << 63);
            low = low.min(key);
            high = high.max(key);
            row += stride;
        }
        let mut distance = 16;
        while distance != 0 {
            low = low.min(cuda_device::warp::shuffle_xor_u64(low, distance));
            high = high.max(cuda_device::warp::shuffle_xor_u64(high, distance));
            distance /= 2;
        }
        if thread::threadIdx_x() % 32 == 0 {
            unsafe { DeviceAtomicU64::from_ptr(bounds) }.fetch_min(low, AtomicOrdering::Relaxed);
            unsafe { DeviceAtomicU64::from_ptr(bounds.add(1)) }
                .fetch_max(high, AtomicOrdering::Relaxed);
        }
    }

    // The bitmap allocation is a recorded budget. Bounds remain on device;
    // an unsupported domain marks the relation and yields an empty safe view.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn membership_bounds(
        table: TableView,
        bounds: *mut u64,
        budget: u64,
        status: *mut OutputCount,
        mut min: i64,
        mut max: i64,
        reduced: bool,
    ) {
        if thread::index_1d().get() == 0 {
            let count = unsafe { table_len(table) };
            if reduced {
                min = ((unsafe { bounds.read() }) ^ (1u64 << 63)) as i64;
                max = ((unsafe { bounds.add(1).read() }) ^ (1u64 << 63)) as i64;
            }
            if count == 0 {
                min = 0;
                max = -1;
            } else {
                let range = (max as u64).wrapping_sub(min as u64);
                if range >= budget {
                    unsafe {
                        (*status).error = 6;
                    }
                    min = 0;
                    max = -1;
                }
            }
            unsafe {
                bounds.write(min as u64);
                bounds.add(1).write(max as u64);
            }
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn membership_insert(
        table: TableView,
        key_column: usize,
        bounds: *const u64,
        bits: *mut u64,
    ) {
        let min = unsafe { bounds.read() } as i64;
        let max = unsafe { bounds.add(1).read() } as i64;
        if min > max {
            return;
        }
        let mut row = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * 256;
        let count = unsafe { table_len(table) };
        while row < count {
            let position =
                (unsafe { column(table, key_column, row) } as u64).wrapping_sub(min as u64);
            unsafe { DeviceAtomicU64::from_ptr(bits.add((position / 64) as usize)) }
                .fetch_or(1u64 << (position % 64), AtomicOrdering::Relaxed);
            row += stride;
        }
    }
    /// Maintained exact keys live in slots, not in transient candidate storage.
    /// Busy slots cause a retry after warp reconvergence, never an owner-wait
    /// spin inside divergent control flow.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn persistent_admit(
        input: TableView,
        width: usize,
        keys: *mut u64,
        slots: *mut u64,
        capacity: usize,
        output: OutputView,
        emit: bool,
    ) {
        let lane = thread::threadIdx_x() as u64 % 32;
        let mut base = thread::index_1d().get() as u64 - lane;
        let stride = thread::gridDim_x() as u64 * 256;
        let count = unsafe { table_len(input) };
        while base < count {
            let row = base + lane;
            let mut pending = row < count;
            while cuda_device::warp::ballot_sync(u32::MAX, pending) != 0 {
                if pending {
                    let mut hash = 0x9e37_79b9_7f4a_7c15u64;
                    let mut word = 0;
                    while word < width.max(1) {
                        hash ^= if width == 0 {
                            0
                        } else {
                            unsafe { column(input, word, row) as u64 }
                        };
                        hash = (hash ^ (hash >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
                        hash = (hash ^ (hash >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
                        hash ^= hash >> 31;
                        word += 1;
                    }
                    let mut slot = hash as usize & (capacity - 1);
                    let mut probes = 0;
                    while probes < capacity {
                        let entry = unsafe { DeviceAtomicU64::from_ptr(slots.add(slot)) };
                        let mut state = entry.load(AtomicOrdering::Acquire);
                        if state == 0 {
                            match entry.compare_exchange(
                                0,
                                1,
                                AtomicOrdering::Acquire,
                                AtomicOrdering::Relaxed,
                            ) {
                                Ok(_) => {
                                    let destination = if emit {
                                        unsafe { reserve(output, 1) }
                                    } else {
                                        0
                                    };
                                    let mut col = 0;
                                    while col < width {
                                        let value = unsafe { column(input, col, row) };
                                        unsafe {
                                            keys.add(slot * width + col).write(value as u64);
                                            if emit {
                                                write_output(output, col, destination, value);
                                            }
                                        }
                                        col += 1;
                                    }
                                    entry.store(2, AtomicOrdering::Release);
                                    pending = false;
                                    break;
                                }
                                Err(_) => state = entry.load(AtomicOrdering::Acquire),
                            }
                        }
                        if state == 1 {
                            break;
                        }
                        let mut same = true;
                        let mut col = 0;
                        while col < width {
                            if unsafe {
                                keys.add(slot * width + col).read()
                                    != column(input, col, row) as u64
                            } {
                                same = false;
                                break;
                            }
                            col += 1;
                        }
                        if same {
                            pending = false;
                            break;
                        }
                        slot = (slot + 1) & (capacity - 1);
                        probes += 1;
                    }
                    if probes == capacity {
                        unsafe {
                            flag_error(output, 3);
                        }
                        pending = false;
                    }
                }
            }
            base += stride;
        }
    }

    /// Rehash only on geometric growth. Old keys are unique, so claiming an
    /// empty destination is sufficient; no partially published key is read.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn persistent_rehash(
        old_keys: *const u64,
        old_slots: *const u64,
        old_capacity: usize,
        keys: *mut u64,
        slots: *mut u64,
        capacity: usize,
        width: usize,
    ) {
        let mut old = thread::index_1d().get() as usize;
        let stride = thread::gridDim_x() as usize * 256;
        while old < old_capacity {
            if unsafe { old_slots.add(old).read() } == 2 {
                let mut hash = 0x9e37_79b9_7f4a_7c15u64;
                let mut col = 0;
                while col < width.max(1) {
                    hash ^= if width == 0 {
                        0
                    } else {
                        unsafe { old_keys.add(old * width + col).read() }
                    };
                    hash = (hash ^ (hash >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
                    hash = (hash ^ (hash >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
                    hash ^= hash >> 31;
                    col += 1;
                }
                let mut slot = hash as usize & (capacity - 1);
                loop {
                    let entry = unsafe { DeviceAtomicU64::from_ptr(slots.add(slot)) };
                    if entry.compare_exchange_relaxed(0, 2).is_ok() {
                        let mut col = 0;
                        while col < width {
                            unsafe {
                                keys.add(slot * width + col)
                                    .write(old_keys.add(old * width + col).read());
                            }
                            col += 1;
                        }
                        break;
                    }
                    slot = (slot + 1) & (capacity - 1);
                }
            }
            old += stride;
        }
    }

    /// Copy into a known unpublished tail; no per-row append atomic.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn persistent_copy(input: TableView, width: usize, output: OutputView, offset: u64) {
        let mut row = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * 256;
        while row < unsafe { table_len(input) } {
            let mut col = 0;
            while col < width {
                unsafe {
                    write_output(output, col, offset + row, column(input, col, row));
                }
                col += 1;
            }
            row += stride;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn pack(input: TableView, width: usize, packed: *mut u64, words: usize) {
        let mut row = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * 256;
        let count = unsafe { table_len(input) };
        while row < count {
            if width == 0 {
                unsafe {
                    packed.add(row as usize * words).write(0);
                }
            }
            let mut col = 0;
            while col < width {
                unsafe {
                    packed
                        .add(row as usize * words + col)
                        .write(column(input, col, row) as u64);
                }
                col += 1;
            }
            row += stride;
        }
    }
    #[inline(always)]
    fn active_set_capacity(rows: u64, allocation: usize) -> usize {
        let needed = (rows as usize).saturating_mul(2).max(2);
        let mut capacity = 2usize;
        while capacity < needed && capacity < allocation {
            capacity *= 2;
        }
        capacity.min(allocation)
    }
    /// Only the live-row-sized set can be probed by set_insert.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn clear_set(input: TableView, slots: *mut u64, allocation: usize) {
        let capacity = active_set_capacity(unsafe { table_len(input) }, allocation);
        let mut slot = thread::index_1d().get() as usize;
        let stride = thread::gridDim_x() as usize * 256;
        while slot < capacity {
            unsafe {
                slots.add(slot).write(0);
            }
            slot += stride;
        }
    }
    /// Slots publish immutable row IDs. Compare every word, never a fingerprint.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn set_insert(
        input: TableView,
        width: usize,
        packed: *const u64,
        words: usize,
        slots: *mut u64,
        capacity: usize,
        output: OutputView,
        emit: bool,
    ) {
        let mut row = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * 256;
        let count = unsafe { table_len(input) };
        let capacity = active_set_capacity(count, capacity);
        while row < count {
            let key = unsafe { packed.add(row as usize * words) };
            let mut hash = 0x9e37_79b9_7f4a_7c15u64;
            let mut word = 0;
            while word < words {
                hash ^= unsafe { key.add(word).read() };
                hash = (hash ^ (hash >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
                hash = (hash ^ (hash >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
                hash ^= hash >> 31;
                word += 1;
            }
            let mut slot = hash as usize & (capacity - 1);
            let mut probes = 0;
            let mut finished = false;
            while probes < capacity {
                let entry = unsafe { DeviceAtomicU64::from_ptr(slots.add(slot)) };
                match entry.compare_exchange_relaxed(0, row + 1) {
                    Ok(_) => {
                        if emit {
                            let destination = unsafe { reserve(output, 1) };
                            let mut col = 0;
                            while col < width {
                                unsafe {
                                    write_output(output, col, destination, column(input, col, row));
                                }
                                col += 1;
                            }
                        }
                        finished = true;
                        break;
                    }
                    Err(existing) => {
                        let other = unsafe { packed.add((existing - 1) as usize * words) };
                        let mut same = true;
                        let mut word = 0;
                        while word < words {
                            if unsafe { key.add(word).read() != other.add(word).read() } {
                                same = false;
                                break;
                            }
                            word += 1;
                        }
                        if same {
                            finished = true;
                            break;
                        }
                    }
                }
                slot = (slot + 1) & (capacity - 1);
                probes += 1;
            }
            if !finished {
                unsafe {
                    flag_error(output, 3);
                }
            }
            row += stride;
        }
    }
    /// Append live rows, never allocation padding. Each launch is stream ordered.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn append(input: TableView, width: usize, output: OutputView) {
        let mut row = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * 256;
        while row < unsafe { table_len(input) } {
            let destination = unsafe { reserve(output, 1) };
            let mut col = 0;
            while col < width {
                unsafe {
                    write_output(output, col, destination, column(input, col, row));
                }
                col += 1;
            }
            row += stride;
        }
    }

    /// Probe the same exact multiword set as dedup; preserve the source payload.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn set_exclude(
        source: TableView,
        projected: TableView,
        keys: TableView,
        width: usize,
        key_width: usize,
        packed: *const u64,
        words: usize,
        slots: *const u64,
        allocation: usize,
        output: OutputView,
    ) {
        let mut row = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * 256;
        let capacity = active_set_capacity(unsafe { table_len(keys) }, allocation);
        while row < unsafe { table_len(source) } {
            let mut hash = 0x9e37_79b9_7f4a_7c15u64;
            let mut word = 0;
            while word < words {
                let value = if key_width == 0 {
                    0
                } else {
                    unsafe { column(projected, word, row) as u64 }
                };
                hash ^= value;
                hash = (hash ^ (hash >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
                hash = (hash ^ (hash >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
                hash ^= hash >> 31;
                word += 1;
            }
            let mut slot = hash as usize & (capacity - 1);
            let mut probes = 0;
            let mut found = false;
            while probes < capacity {
                let existing = unsafe { slots.add(slot).read() };
                if existing == 0 {
                    break;
                }
                let other = unsafe { packed.add((existing - 1) as usize * words) };
                let mut same = true;
                let mut word = 0;
                while word < words {
                    let value = if key_width == 0 {
                        0
                    } else {
                        unsafe { column(projected, word, row) as u64 }
                    };
                    if value != unsafe { other.add(word).read() } {
                        same = false;
                        break;
                    }
                    word += 1;
                }
                if same {
                    found = true;
                    break;
                }
                slot = (slot + 1) & (capacity - 1);
                probes += 1;
            }
            if !found {
                let destination = unsafe { reserve(output, 1) };
                let mut col = 0;
                while col < width {
                    unsafe {
                        write_output(output, col, destination, column(source, col, row));
                    }
                    col += 1;
                }
            }
            row += stride;
        }
    }

    /// Fold checked metadata on device; ancestors contribute errors, not rows.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn fold_status(
        input: *const OutputCount,
        capacity: u64,
        include_rows: bool,
        output: *mut OutputCount,
    ) {
        if thread::index_1d().get() == 0 {
            unsafe {
                let status = input.read();
                if status.error != 0 {
                    (*output).error = status.error;
                }
                if status.rows > capacity {
                    (*output).error = 4;
                }
                if include_rows {
                    match (*output).rows.checked_add(status.rows) {
                        Some(rows) => (*output).rows = rows,
                        None => (*output).error = 1,
                    }
                }
            }
        }
    }
    /// Failed fused tickets never publish rows or keys. All in-range tickets
    /// have completed before this stream-ordered suspension finalizer runs.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn fused_clamp(output: OutputView) {
        if thread::index_1d().get() == 0 {
            unsafe {
                output.count.write(output.count.read().min(output.capacity));
            }
        }
    }
    /// Rebuild only at growth boundaries; retained output rows are immutable.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn fused_rehash(output: OutputView, width: usize, set: crate::FusedSetView) {
        let mut row = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * 256;
        while row < unsafe { output.count.read() } {
            let mut hash = crate::HASHED_RUN_SEED;
            let mut column = 0;
            while column < width {
                hash = crate::hashed_run_step(hash, unsafe {
                    (*output.columns.add(column)).add(row as usize).read()
                });
                column += 1;
            }
            let mut slot = hash as usize & (set.capacity - 1);
            loop {
                if unsafe { DeviceAtomicU64::from_ptr(set.slots.add(slot)) }
                    .compare_exchange_relaxed(0, row + 1)
                    .is_ok()
                {
                    break;
                }
                slot = (slot + 1) & (set.capacity - 1);
            }
            row += stride;
        }
    }
}
