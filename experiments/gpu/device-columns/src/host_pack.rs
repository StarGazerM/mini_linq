//! Host FOR packing into the exact device layout written by the `headers`,
//! `offsets` and `encode` kernels for `EncodedColumn::from_host(.., Codec::For)`:
//! per-1024-row vector headers with word offsets, and sixteen interleaved
//! T=64 lanes of `width` words each in the unified transposed order.
use crate::{original_position, Codec, Result, VectorHeader, VECTOR_ROWS};

/// FOR headers and packed payload words for `values`, identical to the
/// device encoder's output for the same i64 input. Vector groups are packed
/// on all available host threads; each group owns its disjoint payload range.
pub fn pack_for(values: &[i64]) -> (Vec<VectorHeader>, Vec<u64>) {
    let vectors = values.len().div_ceil(VECTOR_ROWS);
    if vectors == 0 {
        return (Vec::new(), Vec::new());
    }
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let per_group = vectors.div_ceil(threads);
    let group_rows = per_group * VECTOR_ROWS;
    let mut headers = vec![VectorHeader::default(); vectors];
    std::thread::scope(|scope| {
        for (chunk, values) in headers.chunks_mut(per_group).zip(values.chunks(group_rows)) {
            scope.spawn(move || {
                for (header, vector) in chunk.iter_mut().zip(values.chunks(VECTOR_ROWS)) {
                    *header = frame(vector);
                }
            });
        }
    });
    let mut words = 0u64;
    for header in &mut headers {
        header.offset = words;
        words += u64::from(header.width) * 16;
    }
    let mut payload = vec![0u64; words as usize];
    std::thread::scope(|scope| {
        let mut rest = payload.as_mut_slice();
        for (chunk, values) in headers.chunks(per_group).zip(values.chunks(group_rows)) {
            let group_words = chunk.iter().map(|h| h.width as usize * 16).sum();
            let (mine, tail) = std::mem::take(&mut rest).split_at_mut(group_words);
            rest = tail;
            scope.spawn(move || {
                let mut at = 0;
                for (header, vector) in chunk.iter().zip(values.chunks(VECTOR_ROWS)) {
                    let width = header.width as usize;
                    pack_vector(vector, header.base, width, &mut mine[at..at + width * 16]);
                    at += width * 16;
                }
            });
        }
    });
    (headers, payload)
}

/// The `headers` kernel's FOR frame: base = minimum, width = bits of the span.
fn frame(vector: &[i64]) -> VectorHeader {
    let (low, high) = vector.iter().fold((i64::MAX, i64::MIN), |(low, high), &v| {
        (low.min(v), high.max(v))
    });
    VectorHeader {
        offset: 0,
        base: low,
        width: 64 - (high as u64).wrapping_sub(low as u64).leading_zeros(),
        codec: Codec::For as u32,
        rows: vector.len() as u32,
        padding: 0,
    }
}

/// The `encode` kernel's FOR branch: lane `l` writes words `l, l+16, ...`;
/// rows past a partial vector's end contribute zero residuals.
fn pack_vector(vector: &[i64], base: i64, width: usize, data: &mut [u64]) {
    if width == 0 {
        return;
    }
    for lane in 0..16 {
        let mut word = 0u64;
        let mut occupied = 0usize;
        let mut index = lane;
        for slot in 0..64 {
            let row = original_position(slot * 16 + lane);
            let residual = vector
                .get(row)
                .map_or(0, |&value| (value as u64).wrapping_sub(base as u64));
            word |= residual << occupied;
            if occupied + width >= 64 {
                data[index] = word;
                index += 16;
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
        }
    }
}

/// Reject headers and a payload length that are not a [`pack_for`] layout for
/// `rows` rows: one FOR header per vector with exact row counts, contiguous
/// word offsets, widths at most 64 and a payload covering every vector.
pub fn check_packed_layout(rows: usize, headers: &[VectorHeader], words: usize) -> Result<()> {
    if headers.len() != rows.div_ceil(VECTOR_ROWS) {
        return Err("packed column header count differs from its rows".into());
    }
    let mut offset = 0u64;
    for (index, header) in headers.iter().enumerate() {
        let expected = (rows - index * VECTOR_ROWS).min(VECTOR_ROWS);
        if header.codec != Codec::For as u32
            || header.offset != offset
            || header.rows as usize != expected
            || header.width > 64
            || header.padding != 0
        {
            return Err(format!("packed column header {index} is not a FOR vector layout").into());
        }
        offset += u64::from(header.width) * 16;
    }
    if offset != words as u64 {
        return Err("packed column payload length differs from its headers".into());
    }
    Ok(())
}
