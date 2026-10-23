#[cuda_device::cuda_module]
pub(crate) mod codecs {
    use crate::{
        decode_source, load_element, original_position, Codec, ColumnView, SourceView,
        VectorHeader, ELEMENT_I64,
    };
    use cuda_device::warp;
    use cuda_device::{device, kernel, launch_bounds, launch_contract, thread, DynamicSharedArray};

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn headers(
        input: *const u8,
        element: u32,
        capacity: u64,
        count: *const u64,
        codec: u32,
        output: *mut VectorHeader,
    ) {
        let len = if count.is_null() {
            capacity
        } else {
            unsafe { count.read() }.min(capacity)
        };
        // One warp cooperates on each vector; scalar per-vector scans leave
        // almost all memory parallelism idle for small resident results.
        let lane = thread::threadIdx_x() as usize % 32;
        let mut vector = thread::index_1d().get() as u64 / 32;
        let vectors = len.div_ceil(1024);
        let stride = thread::gridDim_x() as u64 * 8;
        while vector < vectors {
            let begin = vector * 1024;
            let rows = len.saturating_sub(begin).min(1024) as usize;
            let mut low = i64::MAX;
            let mut high = i64::MIN;
            if codec != Codec::Plain as u32 {
                let mut i = lane;
                while i < rows {
                    if codec != Codec::Delta as u32 || i % 64 != 0 {
                        let value = unsafe { load_element(input, element, begin as usize + i) };
                        let value = if codec == Codec::Delta as u32 {
                            value.wrapping_sub(unsafe {
                                load_element(input, element, begin as usize + i - 1)
                            })
                        } else {
                            value
                        };
                        low = low.min(value);
                        high = high.max(value);
                    }
                    i += 32;
                }
                let mut distance = 16;
                while distance != 0 {
                    low = low.min(cuda_device::warp::shuffle_xor_u64(low as u64, distance) as i64);
                    high =
                        high.max(cuda_device::warp::shuffle_xor_u64(high as u64, distance) as i64);
                    distance /= 2;
                }
            }
            if low > high {
                low = 0;
                high = 0;
            }
            let width = if codec == Codec::Plain as u32 {
                64
            } else {
                64 - (high as u64).wrapping_sub(low as u64).leading_zeros()
            };
            if lane == 0 {
                unsafe {
                    output.add(vector as usize).write(VectorHeader {
                        offset: 0,
                        base: if codec == Codec::Plain as u32 { 0 } else { low },
                        width,
                        codec,
                        rows: rows as u32,
                        padding: 0,
                    });
                }
            }
            vector += stride;
        }
    }

    // Hierarchical exclusive scan. Each block scans at most 256 vector sizes;
    // all levels remain device-resident, including the final packed extent.
    #[device]
    fn scan_words(value: u64) -> (u64, u64) {
        let lane = thread::threadIdx_x() as usize;
        let shared = DynamicSharedArray::<u8>::get_raw() as *mut u64;
        unsafe {
            shared.add(lane).write(value);
        }
        thread::sync_threads();
        let mut sum = value;
        let mut distance = 1;
        while distance < 256 {
            let before = if lane >= distance {
                unsafe { shared.add(lane - distance).read() }
            } else {
                0
            };
            thread::sync_threads();
            sum += before;
            unsafe {
                shared.add(lane).write(sum);
            }
            thread::sync_threads();
            distance *= 2;
        }
        let total = unsafe { shared.add(255).read() };
        thread::sync_threads();
        (sum - value, total)
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1), dynamic_shared_range = (2048, 2048))]
    pub unsafe fn offsets(
        headers: *mut VectorHeader,
        capacity: u64,
        count: *const u64,
        totals: *mut u64,
    ) {
        let len = if count.is_null() {
            capacity
        } else {
            unsafe { count.read() }.min(capacity)
        };
        let live_vectors = len.div_ceil(1024) as usize;
        let blocks = (capacity.div_ceil(1024) as usize).div_ceil(256).max(1);
        let mut block = thread::blockIdx_x() as usize;
        let lane = thread::threadIdx_x() as usize;
        while block < blocks {
            let i = block * 256 + lane;
            let words = if i < live_vectors {
                let h = unsafe { headers.add(i).read() };
                h.width as u64 * 16
                    + if h.codec == Codec::Delta as u32 {
                        16
                    } else {
                        0
                    }
            } else {
                0
            };
            let (prefix, total) = scan_words(words);
            if i < live_vectors {
                unsafe {
                    (*headers.add(i)).offset = prefix;
                }
            }
            if lane == 0 {
                unsafe {
                    totals.add(block).write(total);
                }
            }
            block += thread::gridDim_x() as usize;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1), dynamic_shared_range = (2048, 2048))]
    pub unsafe fn scan_offsets(values: *mut u64, len: usize, totals: *mut u64) {
        let mut block = thread::blockIdx_x() as usize;
        let lane = thread::threadIdx_x() as usize;
        while block < len.div_ceil(256) {
            let i = block * 256 + lane;
            let value = if i < len {
                unsafe { values.add(i).read() }
            } else {
                0
            };
            let (prefix, total) = scan_words(value);
            if i < len {
                unsafe {
                    values.add(i).write(prefix);
                }
            }
            if lane == 0 {
                unsafe {
                    totals.add(block).write(total);
                }
            }
            block += thread::gridDim_x() as usize;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn add_offsets(values: *mut u64, len: usize, prefixes: *const u64) {
        let mut i = thread::index_1d().get();
        while i < len {
            unsafe {
                *values.add(i) += prefixes.add(i / 256).read();
            }
            i += thread::gridDim_x() as usize * 256;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn finish_offsets(
        headers: *mut VectorHeader,
        capacity: u64,
        count: *const u64,
        prefixes: *const u64,
    ) {
        let len = if count.is_null() {
            capacity
        } else {
            unsafe { count.read() }.min(capacity)
        };
        let vectors = len.div_ceil(1024) as usize;
        let mut i = thread::index_1d().get();
        while i < vectors {
            unsafe {
                (*headers.add(i)).offset += prefixes.add(i / 256).read();
            }
            i += thread::gridDim_x() as usize * 256;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn encode(
        input: *const u8,
        element: u32,
        headers: *const VectorHeader,
        capacity: u64,
        count: *const u64,
        output: *mut u64,
    ) {
        let len = if count.is_null() {
            capacity
        } else {
            unsafe { count.read() }.min(capacity)
        };
        let vectors = len.div_ceil(1024) as usize;
        // Sixteen independent T=64 lanes per vector. Each lane owns all its
        // packed words: no bitwise atomics or cross-lane carry is required.
        let lane = thread::threadIdx_x() as usize % 16;
        let mut vector = thread::blockIdx_x() as usize * 16 + thread::threadIdx_x() as usize / 16;
        let stride = thread::gridDim_x() as usize * 16;
        while vector < vectors {
            let h = unsafe { headers.add(vector).read() };
            let is_delta = h.codec == Codec::Delta as u32;
            let data = unsafe { output.add(h.offset as usize) };
            if is_delta {
                let row = lane * 64;
                let base = if row < h.rows as usize {
                    unsafe { load_element(input, element, vector * 1024 + row) }
                } else {
                    0
                };
                unsafe {
                    data.add(lane).write(base as u64);
                }
            }
            let data = unsafe { data.add(if is_delta { 16 } else { 0 }) };
            let width = h.width as usize;
            if width != 0 {
                let mut word = 0u64;
                let mut occupied = 0usize;
                let mut word_index = lane;
                let mut slot = 0;
                while slot < 64 {
                    let row = original_position(slot * 16 + lane);
                    let mut residual = 0u64;
                    if row < h.rows as usize && (!is_delta || row % 64 != 0) {
                        let value = unsafe { load_element(input, element, vector * 1024 + row) };
                        let value = if is_delta {
                            value.wrapping_sub(unsafe {
                                load_element(input, element, vector * 1024 + row - 1)
                            })
                        } else {
                            value
                        };
                        residual = (value as u64).wrapping_sub(h.base as u64);
                    }
                    word |= residual << occupied;
                    if occupied + width >= 64 {
                        unsafe {
                            data.add(word_index).write(word);
                        }
                        word_index += 16;
                        let consumed = 64 - occupied;
                        word = if consumed < width {
                            residual >> consumed
                        } else {
                            0
                        };
                        occupied = occupied + width - 64;
                    } else {
                        occupied += width;
                    }
                    slot += 1;
                }
            }
            vector += stride;
        }
    }

    /// Gather `source[order[i]]` into index order and FOR-pack it in one pass.
    /// One warp per 1024-row vector stages its native-width values in shared memory,
    /// reduces the vector's extrema, writes its header, and packs it into a
    /// fixed slot of `slot_width` 1024-bit words at `vector * slot_width * 16`.
    /// The vector's own width never exceeds the slot because the caller's
    /// bounds enclose every value. `element` selects u32, i32, or i64 lanes.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1), dynamic_shared_range = (32768, 65536))]
    pub unsafe fn gather_encode(
        source: SourceView,
        order: *const u32,
        rows: u64,
        slot_width: u32,
        element: u32,
        headers: *mut VectorHeader,
        output: *mut u64,
    ) {
        let warp_index = thread::threadIdx_x() as usize / 32;
        let lane = thread::threadIdx_x() as usize % 32;
        let element_bytes = if element == ELEMENT_I64 { 8 } else { 4 };
        let staged = unsafe {
            DynamicSharedArray::<u8, 16>::get_raw().add(warp_index * 1024 * element_bytes)
        };
        let vectors = rows.div_ceil(1024);
        let mut vector = thread::blockIdx_x() as u64 * 8 + warp_index as u64;
        let stride = thread::gridDim_x() as u64 * 8;
        while vector < vectors {
            let begin = vector * 1024;
            let count = (rows - begin).min(1024) as usize;
            let mut low = i64::MAX;
            let mut high = i64::MIN;
            let mut i = lane;
            while i < count {
                let gathered =
                    unsafe { decode_source(source, order.add(begin as usize + i).read() as u64) };
                unsafe {
                    if element == ELEMENT_I64 {
                        (staged as *mut i64).add(i).write(gathered);
                    } else {
                        (staged as *mut u32).add(i).write(gathered as u32);
                    }
                }
                low = low.min(gathered);
                high = high.max(gathered);
                i += 32;
            }
            let mut distance = 16;
            while distance != 0 {
                low = low.min(warp::shuffle_xor_u64(low as u64, distance) as i64);
                high = high.max(warp::shuffle_xor_u64(high as u64, distance) as i64);
                distance /= 2;
            }
            warp::sync_mask(u32::MAX);
            let width = 64 - (high as u64).wrapping_sub(low as u64).leading_zeros();
            let offset = vector * slot_width as u64 * 16;
            if lane == 0 {
                unsafe {
                    headers.add(vector as usize).write(VectorHeader {
                        offset,
                        base: low,
                        width,
                        codec: Codec::For as u32,
                        rows: count as u32,
                        padding: 0,
                    });
                }
            }
            // Sixteen independent T=64 lanes, as in `encode`.
            if lane < 16 && width != 0 {
                let data = unsafe { output.add(offset as usize) };
                let width = width as usize;
                let mut word = 0u64;
                let mut occupied = 0usize;
                let mut word_index = lane;
                let mut slot = 0;
                while slot < 64 {
                    let row = original_position(slot * 16 + lane);
                    let residual = if row < count {
                        let value = unsafe { load_element(staged, element, row) };
                        (value as u64).wrapping_sub(low as u64)
                    } else {
                        0
                    };
                    word |= residual << occupied;
                    if occupied + width >= 64 {
                        unsafe { data.add(word_index).write(word) };
                        word_index += 16;
                        let consumed = 64 - occupied;
                        word = if consumed < width {
                            residual >> consumed
                        } else {
                            0
                        };
                        occupied = occupied + width - 64;
                    } else {
                        occupied += width;
                    }
                    slot += 1;
                }
            }
            warp::sync_mask(u32::MAX);
            vector += stride;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn fastlanes_decode_column(view: ColumnView, output: *mut i64) {
        let len = unsafe { crate::logical_len(view) };
        let mut vector = thread::blockIdx_x() as u64;
        let lane = thread::threadIdx_x();
        while vector * 1024 < len {
            let [a, b, c, d] = unsafe { crate::decode_tile(view, vector, lane) };
            let row = vector * 1024 + (lane as u64 / 32) * 128 + (lane as u64 % 32) * 2;
            if row < len {
                unsafe { output.add(row as usize).write(a) };
            }
            if row + 1 < len {
                unsafe { output.add(row as usize + 1).write(b) };
            }
            if row + 64 < len {
                unsafe { output.add(row as usize + 64).write(c) };
            }
            if row + 65 < len {
                unsafe { output.add(row as usize + 65).write(d) };
            }
            vector += thread::gridDim_x() as u64;
        }
    }
}
