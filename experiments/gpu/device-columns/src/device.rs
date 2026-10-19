use cuda_core::DeviceCopy;
use cuda_device::device;

pub const VECTOR_ROWS: usize = 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Codec {
    Plain = 0,
    For = 1,
    Delta = 2,
    Dictionary = 3,
}

/// Offset is in u64 words. Delta vectors prepend sixteen independent bases.
/// The payload is W consecutive 1024-bit words, each with sixteen 64-bit
/// lanes. Every codec uses the same 04261537 unified transposed tuple order.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct VectorHeader {
    pub offset: u64,
    pub base: i64,
    pub width: u32,
    pub codec: u32,
    pub rows: u32,
    pub padding: u32,
}
unsafe impl DeviceCopy for VectorHeader {}

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct ColumnView {
    pub headers: *const VectorHeader,
    pub data: *const u64,
    /// Capacity; logical length may be smaller when count is nonnull.
    pub len: u64,
    pub count: *const u64,
    pub codec: u32,
}
unsafe impl DeviceCopy for ColumnView {}

/// Inverse of the unified transpose in Afroozeh & Boncz, Figure 6:
/// 0,64,...,960,32,96,...,992,...,63,127,...,1023.
#[device]
#[inline(always)]
pub fn transposed_position(original: usize) -> usize {
    let lane = original / 64;
    let q = original % 64;
    let tile = q / 8;
    let reversed = ((tile & 1) << 2) | (tile & 2) | ((tile & 4) >> 2);
    (q % 8) * 128 + reversed * 16 + lane
}

#[device]
#[inline(always)]
pub fn original_position(transposed: usize) -> usize {
    let lane = transposed % 16;
    let slot = transposed / 16;
    let tile = slot % 8;
    let reversed = ((tile & 1) << 2) | (tile & 2) | ((tile & 4) >> 2);
    lane * 64 + reversed * 8 + slot / 8
}

#[device]
#[inline(always)]
pub unsafe fn logical_len(view: ColumnView) -> u64 {
    if view.count.is_null() {
        view.len
    } else {
        unsafe { view.count.read() }.min(view.len)
    }
}

#[device]
#[inline(always)]
pub unsafe fn unpack(data: *const u64, width: u32, original: usize) -> u64 {
    if width == 0 {
        return 0;
    }
    let p = transposed_position(original);
    let lane = p % 16;
    let bit = (p / 16) * width as usize;
    let word = (bit / 64) * 16 + lane;
    let shift = bit % 64;
    let mut value = unsafe { data.add(word).read() } >> shift;
    if shift + width as usize > 64 {
        value |= unsafe { data.add(word + 16).read() } << (64 - shift);
    }
    if width < 64 {
        value &= (1u64 << width) - 1;
    }
    value
}

/// Decode one row without touching an uncompressed global-memory copy.
/// Delta uses independent 64-row lanes. The width-zero case (regular sorted
/// sequences, including all-equal vectors) needs no prefix loop or payload.
#[device]
#[inline(always)]
pub unsafe fn decode(view: ColumnView, row: u64) -> i64 {
    let header = unsafe { view.headers.add((row / 1024) as usize).read() };
    let data = unsafe { view.data.add(header.offset as usize) };
    let local = (row % 1024) as usize;
    if header.codec == Codec::Delta as u32 {
        let lane = local / 64;
        let position = local % 64;
        let mut value = unsafe { data.add(lane).read() };
        if header.width == 0 {
            return value.wrapping_add((header.base as u64).wrapping_mul(position as u64)) as i64;
        }
        let packed = unsafe { data.add(16) };
        let mut i = 1;
        while i <= position {
            value = value
                .wrapping_add(header.base as u64)
                .wrapping_add(unsafe { unpack(packed, header.width, lane * 64 + i) });
            i += 1;
        }
        value as i64
    } else {
        (header.base as u64).wrapping_add(unsafe { unpack(data, header.width, local) }) as i64
    }
}

/// A 256-thread tile's four striped values, returned in registers. All 256
/// threads must call convergently with their thread index as lane. Invalid
/// lanes are zero. Delta prefixes use independent warp scans, never a decoded
/// global buffer: each upper half-lane adds the lower half's warp total.
#[device]
#[inline(always)]
pub unsafe fn decode_tile(view: ColumnView, vector: u64, lane: u32) -> [i64; 4] {
    let mut result = [0i64; 4];
    let len = unsafe { logical_len(view) };
    if vector * 1024 >= len {
        return result;
    }
    let header = unsafe { view.headers.add(vector as usize).read() };
    let data = unsafe { view.data.add(header.offset as usize) };
    let mut i = 0;
    while i < 4 {
        let local = lane as usize + i * 256;
        let row = vector * 1024 + local as u64;
        if header.codec == Codec::Delta as u32 {
            let sequence = local / 64;
            let position = local % 64;
            let base = unsafe { data.add(sequence).read() };
            let value = if header.width == 0 {
                base.wrapping_add((header.base as u64).wrapping_mul(position as u64))
            } else {
                let packed = unsafe { data.add(16) };
                let mut prefix = if row < len && position != 0 {
                    (header.base as u64)
                        .wrapping_add(unsafe { unpack(packed, header.width, local) })
                } else {
                    0
                };
                let mut distance = 1;
                while distance < 32 {
                    let previous = cuda_device::warp::shuffle_up_u64(prefix, distance);
                    if lane % 32 >= distance {
                        prefix = prefix.wrapping_add(previous);
                    }
                    distance *= 2;
                }
                if position >= 32 {
                    let previous_local = sequence * 64 + position - 32;
                    let mut lower = if previous_local % 64 != 0
                        && vector * 1024 + (previous_local as u64) < len
                    {
                        (header.base as u64)
                            .wrapping_add(unsafe { unpack(packed, header.width, previous_local) })
                    } else {
                        0
                    };
                    let mut distance = 16;
                    while distance != 0 {
                        lower =
                            lower.wrapping_add(cuda_device::warp::shuffle_xor_u64(lower, distance));
                        distance /= 2;
                    }
                    prefix = prefix.wrapping_add(lower);
                }
                base.wrapping_add(prefix)
            };
            if row < len {
                result[i] = value as i64;
            }
        } else if row < len {
            result[i] = (header.base as u64)
                .wrapping_add(unsafe { unpack(data, header.width, local) })
                as i64;
        }
        i += 1;
    }
    result
}
