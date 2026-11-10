//! Translate resident source representations directly into scalar order codes.
//!
//! FOR residuals belong to their source frame. A frame is translated to the
//! destination order domain before its residual is added; no logical scalar
//! is reconstructed between the source representation and the packed key.
use gpu_device_columns::{unpack, Codec, SourceView};

/// Formal rule GPU-RadixSourceOrder:
/// Translate a source field into the complete `bits`-wide scalar order domain.
/// Signed domains flip their sign bit, unsigned domains preserve their bits.
/// Packed global frames and vector frames obey the same modular translation
/// law. Delta restart codes accumulate their encoded increments in that domain.
///
/// # Safety
/// The source and selected row are initialized, readable on this stream and
/// retained by their owner. Its descriptors obey the source codec contract;
/// every source value belongs to the recorded scalar domain, `1 <= bits <= 64`.
#[inline(always)]
pub unsafe fn order_code(source: SourceView, row: u64, bits: u32, signed: bool) -> u64 {
    let mask = u64::MAX >> (64 - bits);
    let sign = if signed { 1u64 << (bits - 1) } else { 0 };
    if source.packed != 0 {
        let residual = if source.mask == 0 {
            0
        } else {
            let index = row * source.stride + u64::from(source.shift / 64);
            let shift = source.shift % 64;
            let mut residual = unsafe { *source.column.data.add(index as usize) } >> shift;
            if shift != 0 && source.mask > (u64::MAX >> shift) {
                residual |= unsafe { *source.column.data.add(index as usize + 1) } << (64 - shift);
            }
            residual & source.mask
        };
        let frame = (source.base as u64 & mask) ^ sign;
        return frame.wrapping_add(residual) & mask;
    }
    let row = row * source.stride;
    if source.column.headers.is_null() && source.column.codec == Codec::Plain as u32 {
        return (unsafe { *source.column.data.add(row as usize) } & mask) ^ sign;
    }
    let header = unsafe { *source.column.headers.add((row / 1024) as usize) };
    let data = unsafe { source.column.data.add(header.offset as usize) };
    let local = (row % 1024) as usize;
    if header.codec == Codec::Delta as u32 {
        let lane = local / 64;
        let position = local % 64;
        let restart = (unsafe { *data.add(lane) } & mask) ^ sign;
        let mut code = restart.wrapping_add((header.base as u64).wrapping_mul(position as u64));
        if header.width != 0 {
            let packed = unsafe { data.add(16) };
            let mut index = 1;
            while index <= position {
                code = code.wrapping_add(unsafe { unpack(packed, header.width, lane * 64 + index) });
                index += 1;
            }
        }
        code & mask
    } else {
        let frame = (header.base as u64 & mask) ^ sign;
        frame.wrapping_add(unsafe { unpack(data, header.width, local) }) & mask
    }
}
