#[cuda_device::cuda_module]
pub(crate) mod runtime_kernels {
    use crate::{
        column, flag_error, reserve, table_len, write_output, FrontierView, OutputCount,
        OutputView, TableView,
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
        let mut row = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * 256;
        let count = unsafe { table_len(table) };
        while row < count {
            unsafe {
                output.add(row as usize).write(column(table, index, row));
            }
            row += stride;
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
    // The bitmap allocation is a recorded budget. Bounds remain on device;
    // an unsupported domain marks the relation and yields an empty safe view.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn membership_bounds(
        table: TableView,
        column_index: usize,
        bounds: *mut u64,
        budget: u64,
        status: *mut OutputCount,
    ) {
        if thread::index_1d().get() == 0 {
            let count = unsafe { table_len(table) };
            let mut min = i64::MAX;
            let mut max = i64::MIN;
            let mut row = 0;
            while row < count {
                let key = unsafe { column(table, column_index, row) };
                min = min.min(key);
                max = max.max(key);
                row += 1;
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
    ) {
        let mut row = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * 256;
        let count = unsafe { table_len(input) };
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
                        let destination = unsafe { reserve(output, 1) };
                        let mut col = 0;
                        while col < width {
                            unsafe {
                                write_output(output, col, destination, column(input, col, row));
                            }
                            col += 1;
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
}
