//! Rule-like range scans and membership batches used by the replay.
use super::*;
use crate::{DeviceRadixRun, RadixKeysView};

mod balanced;
/// Additive, order-independent checksum of non-prefix lanes, shared with the host oracle.
pub fn checksum<W: Word>(word: W, suffix_bits: u32) -> u64 {
    let mut sum = 0u64;
    let mut lane = 0;
    while lane < W::BITS / 64 {
        let bits = suffix_bits.saturating_sub(lane * 64).min(64);
        if bits != 0 { sum = sum.wrapping_add((word.lane(lane) & (u64::MAX >> (64 - bits))).wrapping_mul(0x9e3779b97f4a7c15u64.wrapping_add(lane as u64 * 2))); }
        lane += 1;
    }
    sum
}

/// Contribution of one field to the existing lane-weighted checksum.
#[inline(always)]
fn field_checksum(code: u64, shift: u32, bits: u32, field_bits: u32) -> u64 {
    if bits == 0 { return 0; }
    let code = if bits < field_bits { code & (u64::MAX >> (64 - bits)) } else { code };
    let weight = 0x9e3779b97f4a7c15u64.wrapping_add((shift / 64) as u64 * 2);
    let offset = shift % 64;
    let mut sum = (code << offset).wrapping_mul(weight);
    if bits > 64 - offset { sum = sum.wrapping_add((code >> (64 - offset)).wrapping_mul(weight.wrapping_add(2))); }
    sum
}

/// Carries across a checksum lane and truncation at the demanded suffix are
/// corrected algebraically. No value field or full key is reconstructed.
#[inline(always)]
fn checksum_carry(code: u64, base: u64, shift: u32, bits: u32, field_bits: u32) -> u64 {
    if bits == 0 { return 0; }
    let split = 64 - shift % 64;
    let weight = 0x9e3779b97f4a7c15u64.wrapping_add((shift / 64) as u64 * 2);
    let mut correction = 0u64;
    if bits > split {
        let mask = u64::MAX >> (64 - split);
        if (code & mask) > mask - (base & mask) { correction = weight.wrapping_add(2); }
    }
    if bits < field_bits {
        let mask = u64::MAX >> (64 - bits);
        if (code & mask) > mask - (base & mask) {
            let overflow = if bits < split { (1u64 << (bits + shift % 64)).wrapping_mul(weight) }
                else if bits == split { 0 }
                else { (1u64 << (bits - split)).wrapping_mul(weight.wrapping_add(2)) };
            correction = correction.wrapping_sub(overflow);
        }
    }
    correction
}

/// Full-key publication checksum has no suffix truncation state.
#[inline(always)]
unsafe fn checksum_full<W: Word>(view: PackedView<W>, index: u64) -> u64 {
    let mut cursor = device::codes(view, index);
    let mut sum = 0u64;
    while let Some((code, frame, field)) = cursor.next() {
        let shift = field.shift % 64;
        let weight = 0x9e3779b97f4a7c15u64.wrapping_add((field.shift / 64) as u64 * 2);
        sum = sum.wrapping_add((code << shift).wrapping_mul(weight))
            .wrapping_add((frame.base << shift).wrapping_mul(weight));
        if W::BITS > 64 && field.bits > 64 - shift {
            let split = 64 - shift;
            let mask = u64::MAX >> (64 - split);
            let upper = weight.wrapping_add(2);
            sum = sum.wrapping_add((code >> split).wrapping_mul(upper))
                .wrapping_add((frame.base >> split).wrapping_mul(upper));
            if (code & mask) > mask - (frame.base & mask) { sum = sum.wrapping_add(upper); }
        }
    }
    sum
}

/// General exact oracle for partial and cross-lane fields. Linear scan
/// specializations omit this correction state entirely.
#[inline(always)]
unsafe fn partial_column_sum(payload: *const u64, mut bit: u64, frame: Frame, field: PackedField,
    bits: u32, count: u32, step: u64) -> u64 {
    let mut sum = field_checksum(frame.base, field.shift, bits, field.bits).wrapping_mul(count as u64);
    let mut row = 0;
    while row < count {
        let code = device::read_code(payload, bit, frame.width());
        sum = sum.wrapping_add(field_checksum(code, field.shift, bits, field.bits))
            .wrapping_add(checksum_carry(code, frame.base, field.shift, bits, field.bits));
        bit += step;
        row += 1;
    }
    sum
}

#[inline(always)]
unsafe fn sum_codes<const NARROW: bool>(payload: *const u64, mut bit: u64, width: u32, count: u32, step: u64) -> u64 {
    if width == 0 { return 0; }
    let mut sum = 0u64;
    let mut row = 0;
    while row < count {
        let code = if NARROW { device::read_bits32(payload.cast::<u32>(), bit, width) as u64 }
            else { device::read_bits(payload, bit, width) };
        sum = sum.wrapping_add(code);
        bit += step;
        row += 1;
    }
    sum
}

/// Schema-selected whole-field consumer; every demanded field fits one lane.
#[inline(always)]
unsafe fn checksum_linear_row<W: Word>(view: PackedView<W>, index: u64, suffix_bits: u32) -> u64 {
    if !view.plain.is_null() { return checksum(*view.plain.add(index as usize), suffix_bits); }
    let mut cursor = device::codes(view, index);
    let mut sum = 0u64;
    while let Some((code, frame, field)) = cursor.next() {
        if field.shift < suffix_bits {
            let shift = field.shift % 64;
            let weight = 0x9e3779b97f4a7c15u64.wrapping_add((field.shift / 64) as u64 * 2);
            sum = sum.wrapping_add((code << shift).wrapping_mul(weight))
                .wrapping_add((frame.base << shift).wrapping_mul(weight));
        }
    }
    sum
}

fn checksum_layout<W: Word>(state: &PackedSet<W>, suffix_bits: u32) -> (bool, bool) {
    let mut linear = true;
    let mut narrow = true;
    if let Some(run) = state.runs().first() {
        for field in run.packing().packed_fields() {
            let bits = field.bits.min(suffix_bits.saturating_sub(field.shift));
            if bits != 0 {
                linear &= bits == field.bits && bits <= 64 - field.shift % 64;
                narrow &= field.bits <= 32;
            }
        }
    }
    (linear, narrow)
}

/// Enumeration consumer: checksum codes plus their frame contributions.
#[inline(always)]
pub unsafe fn checksum_row<W: Word>(view: PackedView<W>, index: u64, suffix_bits: u32) -> u64 {
    if !view.plain.is_null() { return checksum(*view.plain.add(index as usize), suffix_bits); }
    if suffix_bits == W::BITS { return checksum_full(view, index); }
    let mut cursor = device::codes(view, index);
    let mut sum = 0u64;
    while let Some((code, frame, field)) = cursor.next() {
        let bits = field.bits.min(suffix_bits.saturating_sub(field.shift));
        let shift = field.shift % 64;
        if bits == field.bits && bits <= 64 - shift {
            let weight = 0x9e3779b97f4a7c15u64.wrapping_add((field.shift / 64) as u64 * 2);
            sum = sum.wrapping_add((code << shift).wrapping_mul(weight))
                .wrapping_add((frame.base << shift).wrapping_mul(weight));
        } else {
            sum = sum.wrapping_add(field_checksum(code, field.shift, bits, field.bits))
                .wrapping_add(field_checksum(frame.base, field.shift, bits, field.bits))
                .wrapping_add(checksum_carry(code, frame.base, field.shift, bits, field.bits));
        }
    }
    sum
}

// Sparse fanout needs more than one CTA per probe: partition each interval
// into contiguous pieces so its longest range can use many SMs.
fn scan_config(rows: usize, probes: usize) -> (LaunchConfig1D, u32, u32) {
    let sparse = probes <= 1024 && rows / probes >= 32;
    let shift = if sparse { 8 } else { 5 };
    let partitions = if sparse { if probes <= 64 { 32 } else { 8 } } else { 1 };
    (LaunchConfig1D::new((probes * partitions as usize).div_ceil(256 >> shift).min(65535) as u32, 256, 64), shift, partitions)
}

pub fn plain_read<W: Word>(state: &DeviceRadixRun<W, u64, ()>, low: &Buffer<W>, high: &Buffer<W>, suffix_bits: u32, membership: bool) -> Result<Buffer<[u64; 3]>> {
    let _read = cq_gpu_runtime::nvtx::range(if membership { c"engine_membership" } else { c"engine_scan" });
    let pool = state.pool();
    if low.len() != high.len() || !pool.shares_memory(low.pool()) || !pool.shares_memory(high.pool()) { return Err("read pool/shape mismatch".into()); }
    let output = unsafe { Buffer::uninitialized(pool.clone(), low.len())? };
    if !low.is_empty() {
        let snapshot = state.snapshot()?;
        let module = unsafe { read_kernels::load(pool.context())? };
        if !membership && state.len() / low.len() >= 2 {
            let (launch, shift, partitions) = scan_config(state.len(), low.len());
            let partial = if partitions > 1 { Some(unsafe { Buffer::<[u64; 3]>::uninitialized(pool.clone(), low.len() * partitions as usize)? }) } else { None };
            let target = partial.as_ref().unwrap_or(&output);
            module.plain_scan::<W>(pool.stream(), &module.prepare_plain_scan::<W>(launch)?, snapshot.keys_view(), low.ptr(), high.ptr(), low.len() as u64, suffix_bits, shift, partitions, target.ptr())?;
            if let Some(partial) = partial {
                module.finish_scan(pool.stream(), &module.prepare_finish_scan(config(low.len()))?, partial.ptr(), low.len() as u64, partitions, output.ptr())?;
            }
        } else {
            module.plain::<W>(pool.stream(), &module.prepare_plain::<W>(config(low.len()))?, snapshot.keys_view(), low.ptr(), high.ptr(), low.len() as u64, suffix_bits, membership, output.ptr())?;
        }
    }
    Ok(output)
}

pub fn packed_read<W: Word>(state: &PackedSet<W>, low: &Buffer<W>, high: &Buffer<W>, suffix_bits: u32, membership: bool) -> Result<Buffer<[u64; 3]>> {
    let _read = cq_gpu_runtime::nvtx::range(if membership { c"packed_membership" } else { c"packed_scan" });
    let pool = low.pool();
    if low.len() != high.len() || !pool.shares_memory(high.pool()) || !pool.shares_memory(&state.pool) { return Err("read pool/shape mismatch".into()); }
    let plain_storage = state.runs().iter().all(|run| run.plain.is_some());
    // Amortize the range-prefix launches by fanout, not a probe-count cutoff.
    // Plain decoding is cheaper, so it needs more rows to repay scheduling.
    let balanced_fanout = if plain_storage { 128 } else { 8 };
    if !membership && !low.is_empty() && state.len() / low.len() >= balanced_fanout {
        return balanced::read(state, low, high, suffix_bits, plain_storage);
    }
    let output = unsafe { Buffer::uninitialized(pool.clone(), low.len())? };
    if !low.is_empty() {
        let views = state.read_views()?;
        let module = unsafe { read_kernels::load(pool.context())? };
        if membership {
            if plain_storage {
                module.storage_plain_contains::<W>(pool.stream(), &module.prepare_storage_plain_contains::<W>(config(low.len()))?, views.ptr(), state.runs().len() as u32, low.ptr(), low.len() as u64, output.ptr())?;
            } else {
                module.packed_contains::<W>(pool.stream(), &module.prepare_packed_contains::<W>(config(low.len()))?, views.ptr(), state.runs().len() as u32, low.ptr(), low.len() as u64, output.ptr())?;
            }
        } else if state.len() / low.len() >= 2 {
            let (launch, shift, partitions) = scan_config(state.len(), low.len());
            let partial = if partitions > 1 { Some(unsafe { Buffer::<[u64; 3]>::uninitialized(pool.clone(), low.len() * partitions as usize)? }) } else { None };
            let target = partial.as_ref().unwrap_or(&output);
            if plain_storage {
                module.storage_plain_scan::<W>(pool.stream(), &module.prepare_storage_plain_scan::<W>(launch)?, views.ptr(), state.runs().len() as u32, low.ptr(), high.ptr(), low.len() as u64, suffix_bits, shift, partitions, target.ptr())?;
            } else {
                module.packed_scan::<W>(pool.stream(), &module.prepare_packed_scan::<W>(launch)?, views.ptr(), state.runs().len() as u32, low.ptr(), high.ptr(), low.len() as u64, suffix_bits, shift, partitions, target.ptr())?;
            }
            if let Some(partial) = partial {
                module.finish_scan(pool.stream(), &module.prepare_finish_scan(config(low.len()))?, partial.ptr(), low.len() as u64, partitions, output.ptr())?;
            }
        } else {
            if plain_storage {
                module.storage_plain_serial_scan::<W>(pool.stream(), &module.prepare_storage_plain_serial_scan::<W>(config(low.len()))?, views.ptr(), state.runs().len() as u32, low.ptr(), high.ptr(), low.len() as u64, suffix_bits, output.ptr())?;
            } else {
                module.packed_serial_scan::<W>(pool.stream(), &module.prepare_packed_serial_scan::<W>(config(low.len()))?, views.ptr(), state.runs().len() as u32, low.ptr(), high.ptr(), low.len() as u64, suffix_bits, output.ptr())?;
            }
        }
    }
    Ok(output)
}

#[cuda_host::cuda_module]
mod read_kernels {
    use super::{PackedView, RadixKeysView, checksum};
    use crate::{radix_run::device as plain, packed_run::device as packed, Word};
    use cuda_device::{kernel, launch_bounds, launch_contract, thread};
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn finish_scan(partial: *const [u64; 3], rows: u64, partitions: u32, output: *mut [u64; 3]) {
        let mut i = thread::index_1d().get() as u64;
        while i < rows {
            unsafe {
                let base = i * partitions as u64;
                let mut result = *partial.add(base as usize);
                let mut part = 1;
                while part < partitions {
                    result[2] = result[2].wrapping_add((*partial.add((base + part as u64) as usize))[2]);
                    part += 1;
                }
                *output.add(i as usize) = result;
            }
            i += thread::gridDim_x() as u64 * 256;
        }
    }
    #[inline(always)]
    unsafe fn scan_reduce(mut sum: u64, shift: u32) -> u64 {
        let tid = thread::threadIdx_x();
        let mut distance = 16;
        while distance != 0 { sum = sum.wrapping_add(cuda_device::warp::shuffle_xor_u64(sum, distance)); distance /= 2; }
        if shift == 8 {
            let scratch = cuda_device::DynamicSharedArray::<u64>::get();
            if tid & 31 == 0 { *scratch.add((tid / 32) as usize) = sum; }
            thread::sync_threads();
            if tid == 0 {
                sum = 0;
                let mut warp = 0;
                while warp < 8 { sum = sum.wrapping_add(*scratch.add(warp)); warp += 1; }
            }
            thread::sync_threads();
        }
        sum
    }
    #[inline(always)]
    pub(super) unsafe fn scan_sum<W: Word, const PLAIN: bool, const LINEAR: bool, const NARROW: bool>(view: PackedView<W>, begin: u64, end: u64, lane_index: u32, stride_shift: u32, suffix_bits: u32) -> u64 {
        if begin >= end { return 0; }
        let stride = 1u64 << stride_shift;
        let mut sum = 0u64;
        if PLAIN || !view.plain.is_null() {
            let mut index = begin + lane_index as u64;
            while index < end { sum = sum.wrapping_add(checksum(*view.plain.add(index as usize), suffix_bits)); index += stride; }
            return sum;
        }
        // All lanes walk the same leaves. Restart lane numbering at each
        // boundary; only the reduced sum, not its lane ownership, is observable.
        let mut next_block = packed::row_block(view, begin);
        let mut local = (begin - (*view.headers.add(next_block as usize)).first) as u32;
        let mut left = end - begin;
        while left != 0 {
            let block = next_block;
            let header = *view.headers.add(block as usize);
            let rows = left.min((header.rows - local) as u64) as u32;
            let first = local;
            left -= rows as u64;
            local = 0;
            next_block += 1;
            if lane_index >= rows { continue; }
            let start = first + lane_index;
            let count = 1 + ((rows - 1 - lane_index) >> stride_shift);
            if header.encoding == 0 {
                let mut row = start as u64;
                let mut n = 0;
                while n < count {
                    let mut lane = 0;
                    while lane < W::BITS / 64 {
                        let bits = suffix_bits.saturating_sub(lane * 64).min(64);
                        if bits != 0 {
                            let code = *view.payload.add((header.offset + row * (W::BITS / 64) as u64 + lane as u64) as usize);
                            sum = sum.wrapping_add((code & (u64::MAX >> (64 - bits)))
                                .wrapping_mul(0x9e3779b97f4a7c15u64.wrapping_add(lane as u64 * 2)));
                        }
                        lane += 1;
                    }
                    row += stride;
                    n += 1;
                }
            } else {
                // Advance one bit cursor per column; load frames once per block.
                let mut remaining = header.bits;
                let mut col = 0;
                while col < view.columns {
                    let frame = *view.frames.add((block * view.columns as u64 + col as u64) as usize);
                    let field = *view.fields.add(col as usize);
                    remaining -= frame.width();
                    let bits = if LINEAR { if field.shift < suffix_bits { field.bits } else { 0 } }
                        else { field.bits.min(suffix_bits.saturating_sub(field.shift)) };
                    if bits != 0 {
                        let bit = header.offset * 64 + start as u64 * header.bits as u64 + remaining as u64;
                        let step = stride * header.bits as u64;
                        let shift = field.shift % 64;
                        if LINEAR || (bits == field.bits && bits <= 64 - shift) {
                            let codes = if NARROW || (!LINEAR && frame.width() <= 32) { super::sum_codes::<true>(view.payload, bit, frame.width(), count, step) }
                                else { super::sum_codes::<false>(view.payload, bit, frame.width(), count, step) };
                            let weight = 0x9e3779b97f4a7c15u64.wrapping_add((field.shift / 64) as u64 * 2);
                            sum = sum.wrapping_add((codes << shift).wrapping_mul(weight))
                                .wrapping_add((frame.base << shift).wrapping_mul(weight).wrapping_mul(count as u64));
                        } else {
                            sum = sum.wrapping_add(super::partial_column_sum(view.payload, bit, frame, field, bits, count, step));
                        }
                    }
                    col += 1;
                }
            }
        }
        sum
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn plain<W: Word>(view: RadixKeysView<W, u64>, low: *const W, high: *const W, rows: u64, suffix_bits: u32, membership: bool, output: *mut [u64; 3]) {
        let mut i = thread::index_1d().get() as u64;
        while i < rows {
            unsafe {
                let key = *low.add(i as usize);
                let mut result = [0u64; 3];
                if membership { result[0] = plain::contains(view, key) as u64; }
                else {
                    let mut run = 0;
                    while run < view.run_count {
                        let current = plain::run(view, run);
                        let (a, b) = plain::interval(current, key, *high.add(i as usize));
                        result[0] += a; result[1] += b;
                        let mut row = a;
                        while row < b { result[2] = result[2].wrapping_add(checksum(*current.keys.add(row as usize), suffix_bits)); row += 1; }
                        run += 1;
                    }
                }
                *output.add(i as usize) = result;
            }
            i += thread::gridDim_x() as u64 * 256;
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn packed_serial_scan<W: Word>(views: *const PackedView<W>, count: u32, low: *const W, high: *const W, rows: u64, suffix_bits: u32, output: *mut [u64; 3]) {
        serial_scan::<W, false>(views, count, low, high, rows, suffix_bits, output);
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn storage_plain_serial_scan<W: Word>(views: *const PackedView<W>, count: u32, low: *const W, high: *const W, rows: u64, suffix_bits: u32, output: *mut [u64; 3]) {
        serial_scan::<W, true>(views, count, low, high, rows, suffix_bits, output);
    }
    #[cuda_device::device]
    fn serial_scan<W: Word, const PLAIN: bool>(views: *const PackedView<W>, count: u32, low: *const W, high: *const W, rows: u64, suffix_bits: u32, output: *mut [u64; 3]) {
        let mut i = thread::index_1d().get() as u64;
        while i < rows {
            unsafe {
                let key = *low.add(i as usize);
                let mut result = [0u64; 3];
                let mut run = 0;
                while run < count {
                    let current = *views.add(run as usize);
                    let (a, b) = packed::read_range::<W, PLAIN>(current, key, *high.add(i as usize));
                    result[0] += a; result[1] += b;
                    result[2] = result[2].wrapping_add(scan_sum::<W, PLAIN, false, false>(current, a, b, 0, 0, suffix_bits));
                    run += 1;
                }
                *output.add(i as usize) = result;
            }
            i += thread::gridDim_x() as u64 * 256;
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn packed_contains<W: Word>(views: *const PackedView<W>, count: u32, keys: *const W, rows: u64, output: *mut [u64; 3]) {
        contains::<W, false>(views, count, keys, rows, output);
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn storage_plain_contains<W: Word>(views: *const PackedView<W>, count: u32, keys: *const W, rows: u64, output: *mut [u64; 3]) {
        contains::<W, true>(views, count, keys, rows, output);
    }
    #[cuda_device::device]
    fn contains<W: Word, const PLAIN: bool>(views: *const PackedView<W>, count: u32, keys: *const W, rows: u64, output: *mut [u64; 3]) {
        let mut i = thread::index_1d().get() as u64;
        while i < rows {
            unsafe {
                let key = *keys.add(i as usize);
                let mut found = 0;
                let mut run = 0;
                while run < count {
                    let current = *views.add(run as usize);
                    let hit = if PLAIN {
                        let a = packed::plain_bound(current, key, false);
                        a < current.rows && (*current.plain.add(a as usize)).compare(&key).is_eq()
                    } else { packed::contains(current, key) };
                    if hit { found = 1; break; }
                    run += 1;
                }
                *output.add(i as usize) = [found, 0, 0];
            }
            i += thread::gridDim_x() as u64 * 256;
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1), dynamic_shared_range = (64, 64))]
    pub fn plain_scan<W: Word>(view: RadixKeysView<W, u64>, low: *const W, high: *const W, rows: u64, suffix_bits: u32, shift: u32, partitions: u32, output: *mut [u64; 3]) {
        let lane = thread::threadIdx_x() & 31;
        let group_lane = thread::threadIdx_x() & ((1 << shift) - 1);
        let mut work = (thread::index_1d().get() as u64) >> shift;
        while work < rows * partitions as u64 {
            let i = work >> partitions.trailing_zeros();
            let part = work & (partitions as u64 - 1);
            unsafe {
                let mut result = [0u64; 3];
                let mut run = 0;
                while run < view.run_count {
                    let current = plain::run(view, run);
                    let (mut a, mut b) = if lane == 0 { plain::interval(current, *low.add(i as usize), *high.add(i as usize)) } else { (0, 0) };
                    a = cuda_device::warp::shuffle_up_u64(a, lane);
                    b = cuda_device::warp::shuffle_up_u64(b, lane);
                    result[0] += a; result[1] += b;
                    let length = b.saturating_sub(a);
                    let start = a + ((length * part) >> partitions.trailing_zeros());
                    let stop = a + ((length * (part + 1)) >> partitions.trailing_zeros());
                    let mut row = start + group_lane as u64;
                    while row < stop { result[2] = result[2].wrapping_add(checksum(*current.keys.add(row as usize), suffix_bits)); row += 1 << shift; }
                    run += 1;
                }
                result[2] = scan_reduce(result[2], shift);
                if group_lane == 0 { *output.add(work as usize) = result; }
            }
            work += (thread::gridDim_x() as u64 * 256) >> shift;
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1), dynamic_shared_range = (64, 64))]
    pub fn packed_scan<W: Word>(views: *const PackedView<W>, count: u32, low: *const W, high: *const W, rows: u64, suffix_bits: u32, shift: u32, partitions: u32, output: *mut [u64; 3]) {
        scan::<W, false>(views, count, low, high, rows, suffix_bits, shift, partitions, output);
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1), dynamic_shared_range = (64, 64))]
    pub fn storage_plain_scan<W: Word>(views: *const PackedView<W>, count: u32, low: *const W, high: *const W, rows: u64, suffix_bits: u32, shift: u32, partitions: u32, output: *mut [u64; 3]) {
        scan::<W, true>(views, count, low, high, rows, suffix_bits, shift, partitions, output);
    }
    #[cuda_device::device]
    fn scan<W: Word, const PLAIN: bool>(views: *const PackedView<W>, count: u32, low: *const W, high: *const W, rows: u64, suffix_bits: u32, shift: u32, partitions: u32, output: *mut [u64; 3]) {
        let lane = thread::threadIdx_x() & 31;
        let group_lane = thread::threadIdx_x() & ((1 << shift) - 1);
        let mut work = (thread::index_1d().get() as u64) >> shift;
        while work < rows * partitions as u64 {
            let i = work >> partitions.trailing_zeros();
            let part = work & (partitions as u64 - 1);
            unsafe {
                let mut result = [0u64; 3];
                let mut run = 0;
                while run < count {
                    let current = *views.add(run as usize);
                    let (mut a, mut b) = if lane == 0 { packed::read_range::<W, PLAIN>(current, *low.add(i as usize), *high.add(i as usize)) } else { (0, 0) };
                    a = cuda_device::warp::shuffle_up_u64(a, lane);
                    b = cuda_device::warp::shuffle_up_u64(b, lane);
                    result[0] += a; result[1] += b;
                    let length = b.saturating_sub(a);
                    let start = a + ((length * part) >> partitions.trailing_zeros());
                    let stop = a + ((length * (part + 1)) >> partitions.trailing_zeros());
                    result[2] = result[2].wrapping_add(scan_sum::<W, PLAIN, false, false>(current, start, stop, group_lane, shift, suffix_bits));
                    run += 1;
                }
                result[2] = scan_reduce(result[2], shift);
                if group_lane == 0 { *output.add(work as usize) = result; }
            }
            work += (thread::gridDim_x() as u64 * 256) >> shift;
        }
    }
}
