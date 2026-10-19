#[cuda_device::cuda_module]
pub(crate) mod codecs {
    use crate::{original_position, Codec, ColumnView, VectorHeader};
    use cuda_device::{kernel, launch_bounds, launch_contract, thread};

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn headers(
        input: *const i64,
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
        let mut vector = thread::index_1d().get() as u64;
        let vectors = capacity.div_ceil(1024);
        let stride = thread::gridDim_x() as u64 * 256;
        while vector < vectors {
            let begin = vector * 1024;
            let rows = len.saturating_sub(begin).min(1024) as usize;
            let mut low = i64::MAX;
            let mut high = i64::MIN;
            let mut i = 0;
            while i < rows {
                let value = unsafe { input.add(begin as usize + i).read() };
                if codec != Codec::Delta as u32 || i % 64 != 0 {
                    let value = if codec == Codec::Delta as u32 {
                        value.wrapping_sub(unsafe { input.add(begin as usize + i - 1).read() })
                    } else {
                        value
                    };
                    low = low.min(value);
                    high = high.max(value);
                }
                i += 1;
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
            vector += stride;
        }
    }

    // Only the loader reads total back; device-result encoders reserve their
    // conservative capacity and leave the actual packed extent on the device.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn offsets(headers: *mut VectorHeader, vectors: usize, total: *mut u64) {
        if thread::index_1d().get() == 0 {
            let mut offset = 0;
            let mut i = 0;
            while i < vectors {
                let mut h = unsafe { headers.add(i).read() };
                h.offset = offset;
                offset += h.width as u64 * 16
                    + if h.codec == Codec::Delta as u32 {
                        16
                    } else {
                        0
                    };
                unsafe {
                    headers.add(i).write(h);
                }
                i += 1;
            }
            unsafe {
                total.write(offset);
            }
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn encode(
        input: *const i64,
        headers: *const VectorHeader,
        vectors: usize,
        output: *mut u64,
    ) {
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
                    unsafe { input.add(vector * 1024 + row).read() }
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
                        let value = unsafe { input.add(vector * 1024 + row).read() };
                        let value = if is_delta {
                            value.wrapping_sub(unsafe { input.add(vector * 1024 + row - 1).read() })
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

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn fastlanes_decode_column(view: ColumnView, output: *mut i64) {
        let len = unsafe { crate::logical_len(view) };
        let mut vector = thread::blockIdx_x() as u64;
        let lane = thread::threadIdx_x();
        while vector * 1024 < len {
            let values = unsafe { crate::decode_tile(view, vector, lane) };
            let mut i = 0;
            while i < 4 {
                let row = vector * 1024 + lane as u64 + i as u64 * 256;
                if row < len {
                    unsafe {
                        output.add(row as usize).write(values[i]);
                    }
                }
                i += 1;
            }
            vector += thread::gridDim_x() as u64;
        }
    }
}
