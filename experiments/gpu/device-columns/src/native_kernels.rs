#[cuda_device::cuda_module]
pub(crate) mod native_load {
    use crate::native::{Facts, Input, Summary, ELEMENT_ENCODED};
    use crate::{decode, load_element, logical_len, original_position, Codec, VectorHeader};
    use cuda_device::{device, kernel, launch_bounds, launch_contract, thread, warp, DynamicSharedArray};

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn domain_bounds(members: *const Facts, count: usize, result: *mut Facts) {
        if thread::index_1d().get() == 0 {
            let mut min = i64::MAX;
            let mut max = i64::MIN;
            let mut empty = 1;
            let mut index = 0;
            while index < count {
                let member = unsafe { members.add(index).read() };
                if member.invalid == 0 {
                    min = min.min(member.min);
                    max = max.max(member.max);
                    empty = 0;
                }
                index += 1;
            }
            unsafe { result.write(Facts { min, max, codec: empty, invalid: 0 }) };
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn domain_rebase(headers: *mut VectorHeader, count: usize, minimum: i64) {
        let mut index = thread::index_1d().get();
        let stride = thread::gridDim_x() as usize * 256;
        while index < count {
            let header = unsafe { &mut *headers.add(index) };
            header.base = header.base.wrapping_sub(minimum);
            index += stride;
        }
    }

    #[device]
    #[inline(always)]
    unsafe fn valid(source: Input, positions: *const u32, row: usize) -> bool {
        source.element != ELEMENT_ENCODED
            || ((unsafe { positions.add(row).read() }) as u64) < unsafe { logical_len(source.column) }
    }

    #[device]
    #[inline(always)]
    unsafe fn value(source: Input, positions: *const u32, row: usize) -> i64 {
        let index = if positions.is_null() { row } else { (unsafe { positions.add(row).read() }) as usize };
        if source.element == ELEMENT_ENCODED {
            if index as u64 >= unsafe { logical_len(source.column) } {
                // Summarization reports invalid positions before packing. Do
                // not dereference them, including as a delta predecessor.
                return 0;
            }
            // DECODE I: selected source inversion feeds bounds and immediate resident re-encoding, not a logical consumer.
            unsafe { decode(source.column, index as u64) }
        } else {
            unsafe { load_element(source.column.data.cast::<u8>().add(index * source.stride), source.element, 0) }
        }
    }

    // Each warp reads a logical vector, preserving the existing codec policy's
    // vector and 64-row lane boundaries even when positions permute the input.
    // Native output is written in this same pass rather than widened twice.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn native_summarize(
        source: Input,
        positions: *const u32,
        rows: usize,
        detect_delta: u32,
        native: *mut i64,
        summaries: *mut Summary,
    ) {
        let lane = thread::threadIdx_x() as usize % 32;
        let mut vector = thread::index_1d().get() / 32;
        let stride = thread::gridDim_x() as usize * 8;
        while vector < rows.div_ceil(1024) {
            let begin = vector * 1024;
            let live = (rows - begin).min(1024);
            let mut low = i64::MAX;
            let mut high = i64::MIN;
            let mut dlow = i64::MAX;
            let mut dhigh = i64::MIN;
            let mut descending = 0u64;
            let mut invalid = 0u64;
            let mut i = lane;
            while i < live {
                invalid |= (!unsafe { valid(source, positions, begin + i) }) as u64;
                let current = unsafe { value(source, positions, begin + i) };
                low = low.min(current);
                high = high.max(current);
                if !native.is_null() {
                    unsafe { native.add(begin + i).write(current) };
                }
                if detect_delta != 0 && i != 0 {
                    let previous = unsafe { value(source, positions, begin + i - 1) };
                    descending += (current < previous) as u64;
                    if i % 64 != 0 {
                        let delta = current.wrapping_sub(previous);
                        dlow = dlow.min(delta);
                        dhigh = dhigh.max(delta);
                    }
                }
                i += 32;
            }
            let mut distance = 16;
            while distance != 0 {
                low = low.min(warp::shuffle_xor_u64(low as u64, distance) as i64);
                high = high.max(warp::shuffle_xor_u64(high as u64, distance) as i64);
                invalid |= warp::shuffle_xor_u64(invalid, distance);
                if detect_delta != 0 {
                    dlow = dlow.min(warp::shuffle_xor_u64(dlow as u64, distance) as i64);
                    dhigh = dhigh.max(warp::shuffle_xor_u64(dhigh as u64, distance) as i64);
                    descending += warp::shuffle_xor_u64(descending, distance);
                }
                distance /= 2;
            }
            if lane == 0 {
                unsafe {
                    summaries.add(vector).write(Summary {
                        min: low, max: high, delta_min: dlow, delta_max: dhigh, descending, invalid,
                    });
                }
            }
            vector += stride;
        }
    }

    #[device]
    #[inline(always)]
    fn width(low: i64, high: i64) -> u32 {
        if low > high { 0 } else { 64 - (high as u64).wrapping_sub(low as u64).leading_zeros() }
    }

    // One CTA reduces compact vector summaries, not source values. All 256
    // threads participate; only forty-eight u64 shared slots are needed. Signed
    // comparisons preserve the complete i64 domain (including both extrema).
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1), dynamic_shared_range = (384, 384))]
    pub unsafe fn native_reduce(
        summaries: *const Summary,
        vectors: usize,
        rows: usize,
        default_codec: u32,
        detect_delta: u32,
        facts: *mut Facts,
    ) {
        let tid = thread::threadIdx_x() as usize;
        let lane = tid % 32;
        let warp_id = tid / 32;
        let mut low = i64::MAX;
        let mut high = i64::MIN;
        let mut for_words = 0u64;
        let mut delta_words = 0u64;
        let mut descending = 0u64;
        let mut invalid = 0u64;
        let mut i = tid;
        while i < vectors {
            let summary = unsafe { summaries.add(i).read() };
            low = low.min(summary.min);
            high = high.max(summary.max);
            invalid |= summary.invalid;
            if detect_delta != 0 {
                for_words += width(summary.min, summary.max) as u64 * 16;
                delta_words += width(summary.delta_min, summary.delta_max) as u64 * 16 + 16;
                descending += summary.descending;
            }
            i += 256;
        }
        let mut distance = 16;
        while distance != 0 {
            low = low.min(warp::shuffle_xor_u64(low as u64, distance) as i64);
            high = high.max(warp::shuffle_xor_u64(high as u64, distance) as i64);
            for_words += warp::shuffle_xor_u64(for_words, distance);
            delta_words += warp::shuffle_xor_u64(delta_words, distance);
            descending += warp::shuffle_xor_u64(descending, distance);
            invalid |= warp::shuffle_xor_u64(invalid, distance);
            distance /= 2;
        }
        let shared = DynamicSharedArray::<u8>::get_raw() as *mut u64;
        if lane == 0 {
            unsafe {
                shared.add(warp_id * 6).write(low as u64);
                shared.add(warp_id * 6 + 1).write(high as u64);
                shared.add(warp_id * 6 + 2).write(for_words);
                shared.add(warp_id * 6 + 3).write(delta_words);
                shared.add(warp_id * 6 + 4).write(descending);
                shared.add(warp_id * 6 + 5).write(invalid);
            }
        }
        thread::sync_threads();
        if warp_id == 0 {
            low = i64::MAX;
            high = i64::MIN;
            for_words = 0;
            delta_words = 0;
            descending = 0;
            invalid = 0;
            if lane < 8 {
                unsafe {
                    low = shared.add(lane * 6).read() as i64;
                    high = shared.add(lane * 6 + 1).read() as i64;
                    for_words = shared.add(lane * 6 + 2).read();
                    delta_words = shared.add(lane * 6 + 3).read();
                    descending = shared.add(lane * 6 + 4).read();
                    invalid = shared.add(lane * 6 + 5).read();
                }
            }
            let mut distance = 16;
            while distance != 0 {
                low = low.min(warp::shuffle_xor_u64(low as u64, distance) as i64);
                high = high.max(warp::shuffle_xor_u64(high as u64, distance) as i64);
                for_words += warp::shuffle_xor_u64(for_words, distance);
                delta_words += warp::shuffle_xor_u64(delta_words, distance);
                descending += warp::shuffle_xor_u64(descending, distance);
                invalid |= warp::shuffle_xor_u64(invalid, distance);
                distance /= 2;
            }
            if lane == 0 {
                let codec = if detect_delta != 0 && descending <= rows as u64 / 16 && delta_words < for_words {
                    Codec::Delta as u32
                } else {
                    default_codec
                };
                unsafe { facts.write(Facts { min: low, max: high, codec, invalid: invalid as u32 }) };
            }
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn native_headers(summaries: *const Summary, facts: *const Facts, rows: usize, output: *mut VectorHeader) {
        let codec = unsafe { (*facts).codec };
        let mut vector = thread::index_1d().get();
        while vector < rows.div_ceil(1024) {
            let summary = unsafe { summaries.add(vector).read() };
            let (base, bits) = if codec == Codec::Plain as u32 {
                (0, 64)
            } else if codec == Codec::Delta as u32 {
                (if summary.delta_min > summary.delta_max { 0 } else { summary.delta_min }, width(summary.delta_min, summary.delta_max))
            } else {
                (summary.min, width(summary.min, summary.max))
            };
            unsafe {
                output.add(vector).write(VectorHeader {
                    offset: 0, base, width: bits, codec,
                    rows: (rows - vector * 1024).min(1024) as u32, padding: 0,
                });
            }
            vector += thread::gridDim_x() as usize * 256;
        }
    }

    // The resident layout and padded residuals match codecs::encode exactly;
    // only the input accessor differs, reading selected rows directly.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn native_encode(
        source: Input,
        positions: *const u32,
        headers: *const VectorHeader,
        vectors: usize,
        output: *mut u64,
    ) {
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
                    unsafe { value(source, positions, vector * 1024 + row) }
                } else {
                    0
                };
                unsafe { data.add(lane).write(base as u64) };
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
                        let current = unsafe { value(source, positions, vector * 1024 + row) };
                        let current = if is_delta {
                            current.wrapping_sub(unsafe { value(source, positions, vector * 1024 + row - 1) })
                        } else {
                            current
                        };
                        residual = (current as u64).wrapping_sub(h.base as u64);
                    }
                    word |= residual << occupied;
                    if occupied + width >= 64 {
                        unsafe { data.add(word_index).write(word) };
                        word_index += 16;
                        let consumed = 64 - occupied;
                        word = if consumed < width { residual >> consumed } else { 0 };
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
}
