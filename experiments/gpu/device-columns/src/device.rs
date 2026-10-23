use cuda_core::DeviceCopy;
use cuda_device::device;

pub const VECTOR_ROWS: usize = 1024;

/// Input element types accepted by the packing kernels. Values widen exactly.
pub const ELEMENT_I64: u32 = 0;
pub const ELEMENT_U32: u32 = 1;
pub const ELEMENT_I32: u32 = 2;

#[device]
#[inline(always)]
pub unsafe fn load_element(input: *const u8, element: u32, row: usize) -> i64 {
    unsafe {
        if element == ELEMENT_U32 {
            (input as *const u32).add(row).read() as i64
        } else if element == ELEMENT_I32 {
            (input as *const i32).add(row).read() as i64
        } else {
            (input as *const i64).add(row).read()
        }
    }
}

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
    /// Null only for native, untransposed Plain storage adopted by its owner.
    pub headers: *const VectorHeader,
    pub data: *const u64,
    /// Capacity; logical length may be smaller when count is nonnull.
    pub len: u64,
    pub count: *const u64,
    pub codec: u32,
}
unsafe impl DeviceCopy for ColumnView {}

/// Borrowed gather/build source, including zero-copy row and packed-key projections.
/// Owners must retain the column storage until all consuming stream work completes.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct SourceView {
    pub column: ColumnView,
    pub stride: u64,
    pub packed: u32,
    pub shift: u32,
    pub mask: u64,
    pub base: i64,
}
unsafe impl DeviceCopy for SourceView {}
impl From<ColumnView> for SourceView {
    fn from(column: ColumnView) -> Self {
        Self {
            column,
            stride: 1,
            packed: 0,
            shift: 0,
            mask: u64::MAX,
            base: 0,
        }
    }
}

#[device]
#[inline(always)]
pub unsafe fn decode_source(source: SourceView, row: u64) -> i64 {
    let value = unsafe { decode(source.column, row * source.stride) };
    if source.packed != 0 {
        let offset = if source.mask == 0 {
            0
        } else {
            ((value as u64) >> source.shift) & source.mask
        };
        source.base.wrapping_add(offset as i64)
    } else {
        value
    }
}

#[device]
#[inline(always)]
pub unsafe fn decode_source_u32(source: SourceView, row: u64) -> u32 {
    if source.packed == 0 {
        unsafe { decode_u32(source.column, row * source.stride) }
    } else {
        unsafe { decode_source(source, row) as u32 }
    }
}

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
    // A vector has only 1024 rows and at most 4096 bits per lane. Keep
    // transposed bit addressing in u32; only the final pointer offset is usize.
    let p = transposed_position(original) as u32;
    let lane = p % 16;
    let bit = (p / 16) * width;
    let word = (bit / 64) * 16 + lane;
    let shift = bit % 64;
    let mut value = unsafe { data.add(word as usize).read() } >> shift;
    if shift + width > 64 {
        value |= unsafe { data.add((word + 16) as usize).read() } << (64 - shift);
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
    if view.headers.is_null() && view.codec == Codec::Plain as u32 {
        return (unsafe { view.data.add(row as usize).read() }) as i64;
    }
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

/// Decode one row of a column whose values all lie in u32. FOR arithmetic is
/// done modulo 2^32, which is exact for in-range values; Delta and native
/// Plain storage use the general path.
#[device]
#[inline(always)]
pub unsafe fn decode_u32(view: ColumnView, row: u64) -> u32 {
    if view.headers.is_null() {
        return unsafe { decode(view, row) } as u32;
    }
    let header = unsafe { view.headers.add((row / 1024) as usize).read() };
    if header.codec == Codec::Delta as u32 {
        return unsafe { decode(view, row) } as u32;
    }
    let data = unsafe { view.data.add(header.offset as usize) };
    (header.base as u32)
        .wrapping_add(unsafe { unpack(data, header.width, (row % 1024) as usize) } as u32)
}

#[device]
#[inline(always)]
unsafe fn decode_native(data: *const u64, row: u64, len: u64) -> i64 {
    if row < len {
        (unsafe { data.add(row as usize).read() }) as i64
    } else {
        0
    }
}

/// Decode one 256-row stripe of a vector without a register array. Every lane
/// in each warp must call convergently; `lane` is the 0..256 thread index and
/// `stripe` is 0..4. Invalid rows are zero, including a partial final warp.
#[device]
#[inline(always)]
pub unsafe fn decode_stripe(view: ColumnView, vector: u64, lane: u32, stripe: usize) -> i64 {
    let len = unsafe { logical_len(view) };
    if vector * 1024 >= len {
        return 0;
    }
    if view.headers.is_null() && view.codec == Codec::Plain as u32 {
        return unsafe {
            decode_native(
                view.data,
                vector * 1024 + lane as u64 + stripe as u64 * 256,
                len,
            )
        };
    }
    let header = unsafe { view.headers.add(vector as usize).read() };
    let data = unsafe { view.data.add(header.offset as usize) };
    unsafe { decode_packed_stripe(header, data, len, vector, lane, stripe) }
}

// At most 63 residuals contribute to a delta prefix. Width <= 26 therefore
// permits an exact u32 scan even when the signed deltas or restart base do not
// fit u32. The frame base is added algebraically after the residual scan.
#[device]
#[inline(always)]
unsafe fn narrow_delta_prefix(
    packed: *const u64,
    width: u32,
    local: usize,
    vector: u64,
    len: u64,
    lane: u32,
) -> u64 {
    let position = local % 64;
    let mut prefix = if position != 0 && vector * 1024 + (local as u64) < len {
        (unsafe { unpack(packed, width, local) }) as u32
    } else {
        0
    };
    let mut distance = 1;
    while distance < 32 {
        let previous = cuda_device::warp::shuffle_up(prefix, distance);
        if lane % 32 >= distance {
            prefix = prefix.wrapping_add(previous);
        }
        distance *= 2;
    }
    if position >= 32 {
        let lower_local = local - 32;
        let mut lower = if lower_local % 64 != 0 && vector * 1024 + (lower_local as u64) < len {
            (unsafe { unpack(packed, width, lower_local) }) as u32
        } else {
            0
        };
        let mut distance = 16;
        while distance != 0 {
            lower = lower.wrapping_add(cuda_device::warp::shuffle_xor(lower, distance));
            distance /= 2;
        }
        prefix = prefix.wrapping_add(lower);
    }
    prefix as u64
}

#[device]
#[inline(always)]
unsafe fn decode_packed_stripe(
    header: VectorHeader,
    data: *const u64,
    len: u64,
    vector: u64,
    lane: u32,
    stripe: usize,
) -> i64 {
    let local = lane as usize + stripe * 256;
    let row = vector * 1024 + local as u64;
    if header.codec == Codec::Delta as u32 {
        let sequence = local / 64;
        let position = local % 64;
        let base = unsafe { data.add(sequence).read() };
        let value = if header.width == 0 {
            base.wrapping_add((header.base as u64).wrapping_mul(position as u64))
        } else if header.width <= 26 {
            base.wrapping_add((header.base as u64).wrapping_mul(position as u64))
                .wrapping_add(unsafe {
                    narrow_delta_prefix(data.add(16), header.width, local, vector, len, lane)
                })
        } else {
            let packed = unsafe { data.add(16) };
            let mut prefix = if row < len && position != 0 {
                unsafe { unpack(packed, header.width, local) }
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
                let mut lower =
                    if previous_local % 64 != 0 && vector * 1024 + (previous_local as u64) < len {
                        unsafe { unpack(packed, header.width, previous_local) }
                    } else {
                        0
                    };
                let mut distance = 16;
                while distance != 0 {
                    lower = lower.wrapping_add(cuda_device::warp::shuffle_xor_u64(lower, distance));
                    distance /= 2;
                }
                prefix = prefix.wrapping_add(lower);
            }
            base.wrapping_add((header.base as u64).wrapping_mul(position as u64))
                .wrapping_add(prefix)
        };
        if row < len {
            value as i64
        } else {
            0
        }
    } else if row < len {
        (header.base as u64).wrapping_add(unsafe { unpack(data, header.width, local) }) as i64
    } else {
        0
    }
}

/// One warp owns a complete 64-row delta sequence, two adjacent rows per
/// thread. Scan the pair sums once, then subtract the second residual for the
/// first row. This avoids decoding/reducing the lower half a second time.
#[device]
#[inline(always)]
unsafe fn decode_pair(
    header: VectorHeader,
    data: *const u64,
    len: u64,
    vector: u64,
    local: usize,
    lane: u32,
) -> (i64, i64) {
    let row = vector * 1024 + local as u64;
    if header.codec != Codec::Delta as u32 {
        let first = if row < len {
            (header.base as u64).wrapping_add(unsafe { unpack(data, header.width, local) }) as i64
        } else {
            0
        };
        let second = if row + 1 < len {
            (header.base as u64).wrapping_add(unsafe { unpack(data, header.width, local + 1) })
                as i64
        } else {
            0
        };
        return (first, second);
    }
    let position = local % 64;
    let base = unsafe { data.add(local / 64).read() };
    let mut first = base.wrapping_add((header.base as u64).wrapping_mul(position as u64));
    let mut second = first.wrapping_add(header.base as u64);
    if header.width != 0 {
        let packed = unsafe { data.add(16) };
        let a = if position != 0 && row < len {
            unsafe { unpack(packed, header.width, local) }
        } else {
            0
        };
        let b = if row + 1 < len {
            unsafe { unpack(packed, header.width, local + 1) }
        } else {
            0
        };
        let prefix = if header.width <= 26 {
            // Exactly <=63*(2^26-1), including the final partial vector.
            let mut sum = (a as u32).wrapping_add(b as u32);
            let mut distance = 1;
            while distance < 32 {
                let previous = cuda_device::warp::shuffle_up(sum, distance);
                if lane % 32 >= distance {
                    sum = sum.wrapping_add(previous);
                }
                distance *= 2;
            }
            sum as u64
        } else {
            let mut sum = a.wrapping_add(b);
            let mut distance = 1;
            while distance < 32 {
                let previous = cuda_device::warp::shuffle_up_u64(sum, distance);
                if lane % 32 >= distance {
                    sum = sum.wrapping_add(previous);
                }
                distance *= 2;
            }
            sum
        };
        first = first.wrapping_add(prefix.wrapping_sub(b));
        second = second.wrapping_add(prefix);
    }
    (
        if row < len { first as i64 } else { 0 },
        if row + 1 < len { second as i64 } else { 0 },
    )
}

/// A 256-thread tile's four paired values, returned in registers. All threads
/// must call convergently with their thread index as lane. Returned rows are
/// `vector*1024 + (lane/32)*128 + (lane%32)*2 + [0, 1, 64, 65]`.
#[device]
#[inline(always)]
pub unsafe fn decode_tile(view: ColumnView, vector: u64, lane: u32) -> [i64; 4] {
    let len = unsafe { logical_len(view) };
    let local = (lane as usize / 32) * 128 + (lane as usize % 32) * 2;
    let row = vector * 1024 + local as u64;
    if vector * 1024 >= len {
        return [0; 4];
    }
    if view.headers.is_null() && view.codec == Codec::Plain as u32 {
        return unsafe {
            [
                decode_native(view.data, row, len),
                decode_native(view.data, row + 1, len),
                decode_native(view.data, row + 64, len),
                decode_native(view.data, row + 65, len),
            ]
        };
    }
    let header = unsafe { view.headers.add(vector as usize).read() };
    let data = unsafe { view.data.add(header.offset as usize) };
    let (a, b) = unsafe { decode_pair(header, data, len, vector, local, lane) };
    let (c, d) = unsafe { decode_pair(header, data, len, vector, local + 64, lane) };
    [a, b, c, d]
}

/// Decode one pair of the tile layout without keeping the other pair live
/// through a consumer's joins. All warp lanes call convergently; `pair` is
/// 0 or 1, selecting offsets [0, 1] or [64, 65] from the lane's first row.
#[device]
#[inline(always)]
pub unsafe fn decode_tile_pair(view: ColumnView, vector: u64, lane: u32, pair: usize) -> [i64; 2] {
    let len = unsafe { logical_len(view) };
    if vector * 1024 >= len {
        return [0; 2];
    }
    let local = (lane as usize / 32) * 128 + (lane as usize % 32) * 2 + pair * 64;
    let row = vector * 1024 + local as u64;
    if view.headers.is_null() && view.codec == Codec::Plain as u32 {
        return unsafe {
            [
                decode_native(view.data, row, len),
                decode_native(view.data, row + 1, len),
            ]
        };
    }
    let header = unsafe { view.headers.add(vector as usize).read() };
    let data = unsafe { view.data.add(header.offset as usize) };
    let (a, b) = unsafe { decode_pair(header, data, len, vector, local, lane) };
    [a, b]
}
