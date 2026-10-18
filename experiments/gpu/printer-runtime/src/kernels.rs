use cuda_core::DeviceCopy;

#[derive(Clone, Copy)]
#[repr(C)]
pub(crate) struct PackedField {
    pub min: i64,
    pub bit_offset: u64,
    pub width: u32,
    pub padding: u32,
}
// SAFETY: C-layout scalar metadata, with explicit padding initialized by host.
unsafe impl DeviceCopy for PackedField {}

#[cuda_device::cuda_module]
pub(crate) mod runtime_kernels {
    use super::PackedField;
    use crate::{FrontierView, OutputView, TableView, column, flag_error, reserve, write_output};
    use cuda_device::atomic::{AtomicOrdering, DeviceAtomicU64};
    use cuda_device::{DynamicSharedArray, kernel, launch_bounds, launch_contract, thread};

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
    pub unsafe fn membership_insert(table: TableView, key_column: usize, min: i64, bits: *mut u64) {
        let mut row = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * 256;
        while row < table.len {
            let position =
                (unsafe { column(table, key_column, row) } as u64).wrapping_sub(min as u64);
            unsafe { DeviceAtomicU64::from_ptr(bits.add((position / 64) as usize)) }
                .fetch_or(1u64 << (position % 64), AtomicOrdering::Relaxed);
            row += stride;
        }
    }

    /// Only 2*width extrema words cross to the host. Each block reduces its
    /// grid-stride rows before touching global atomics, avoiding rowwise hot spots.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1), dynamic_shared_range = (4096, 4096))]
    pub unsafe fn extrema(table: TableView, width: usize, bounds: *mut u64) {
        let tid = thread::threadIdx_x() as usize;
        let shared = DynamicSharedArray::<u8>::get_raw() as *mut u64;
        let mut col = 0;
        while col < width {
            let mut min = u64::MAX;
            let mut max = 0u64;
            let mut row = thread::index_1d().get() as u64;
            let stride = thread::gridDim_x() as u64 * 256;
            while row < table.len {
                let ordered = unsafe { column(table, col, row) } as u64 ^ (1u64 << 63);
                min = min.min(ordered);
                max = max.max(ordered);
                row += stride;
            }
            unsafe {
                shared.add(tid).write(min);
                shared.add(256 + tid).write(max);
            }
            thread::sync_threads();
            let mut step = 128;
            while step != 0 {
                if tid < step {
                    unsafe {
                        shared
                            .add(tid)
                            .write(shared.add(tid).read().min(shared.add(tid + step).read()));
                        shared.add(256 + tid).write(
                            shared
                                .add(256 + tid)
                                .read()
                                .max(shared.add(256 + tid + step).read()),
                        );
                    }
                }
                thread::sync_threads();
                step >>= 1;
            }
            if tid == 0 {
                unsafe {
                    DeviceAtomicU64::from_ptr(bounds.add(2 * col))
                        .fetch_min(shared.read(), AtomicOrdering::Relaxed);
                    DeviceAtomicU64::from_ptr(bounds.add(2 * col + 1))
                        .fetch_max(shared.add(256).read(), AtomicOrdering::Relaxed);
                }
            }
            thread::sync_threads();
            col += 1;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn pack(
        input: TableView,
        fields: *const PackedField,
        width: usize,
        packed: *mut u64,
        words: usize,
    ) {
        let mut row = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * 256;
        while row < input.len {
            let output = unsafe { packed.add(row as usize * words) };
            let mut col = 0;
            while col < width {
                let field = unsafe { fields.add(col).read() };
                if field.width != 0 {
                    let value =
                        (unsafe { column(input, col, row) } as u64).wrapping_sub(field.min as u64);
                    let word = (field.bit_offset / 64) as usize;
                    let shift = (field.bit_offset % 64) as u32;
                    unsafe {
                        output
                            .add(word)
                            .write(output.add(word).read() | (value << shift));
                    }
                    if shift != 0 && field.width > 64 - shift {
                        unsafe {
                            output
                                .add(word + 1)
                                .write(output.add(word + 1).read() | (value >> (64 - shift)));
                        }
                    }
                }
                col += 1;
            }
            row += stride;
        }
    }

    /// The input packing kernel completes before this launch. A slot publishes
    /// only a row ID, never a partially written key; duplicate comparisons need
    /// neither locks nor waiting for another thread to finish publishing data.
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
        while row < input.len {
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
