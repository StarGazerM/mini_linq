#[cuda_device::cuda_module]
pub(crate) mod runtime_kernels {
    use crate::{
        FrontierView, OutputCount, OutputView, TableView, column, column_stripe, flag_error,
        reserve, table_len, write_output,
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
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn validate_output(output: OutputView) {
        if thread::index_1d().get() == 0 {
            let count = unsafe { output.count.read() };
            if count > u32::MAX as u64 {
                unsafe {
                    flag_error(output, 5);
                }
            } else if count > output.capacity {
                unsafe {
                    flag_error(output, 4);
                }
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
        table: TableView, slots: *const u64, width: usize, mask: u64, output: *mut i64,
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
            unsafe { output.add(row as usize).write((hash & mask) as i64); }
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
}
