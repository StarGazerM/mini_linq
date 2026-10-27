//! Stable full-width CPU radix sorting of private, unpublished row buffers.

use std::mem::MaybeUninit;

use super::Row;

/// Sorts by key alone and returns the number of executed scatter passes (0..=4).
/// `scratch` must contain at least `rows.len()` slots; its initial contents are
/// irrelevant. Equal keys retain their exact source order, including payloads.
///
/// `varying` is the OR of every key XOR the first key, computed during upload.
/// Only varying bytes receive histograms: counting constant bytes creates a
/// needless dependent increment chain on dense keys. At most 8 KiB of counters
/// are used; odd scatter counts require one contiguous copy back to `rows`.
pub(super) fn sort(rows: &mut [Row], scratch: &mut [MaybeUninit<Row>], varying: u32) -> usize {
    let mut shifts = [0u32; 4];
    let mut count = 0;
    for byte in 0..4 {
        if (varying >> (byte * 8)) & 0xff != 0 {
            shifts[count] = byte * 8;
            count += 1;
        }
    }
    match count {
        0 => {}
        1 => sort_digits(rows, scratch, [shifts[0]]),
        2 => sort_digits(rows, scratch, [shifts[0], shifts[1]]),
        3 => sort_digits(rows, scratch, [shifts[0], shifts[1], shifts[2]]),
        4 => sort_digits(rows, scratch, shifts),
        _ => unreachable!(),
    }
    count
}

fn sort_digits<const DIGITS: usize>(
    rows: &mut [Row],
    scratch: &mut [MaybeUninit<Row>],
    shifts: [u32; DIGITS],
) {
    let len = rows.len();
    let scratch = &mut scratch[..len];
    let mut histograms = [[0usize; 256]; DIGITS];
    for row in rows.iter() {
        for digit in 0..DIGITS {
            histograms[digit][((row.key >> shifts[digit]) & 0xff) as usize] += 1;
        }
    }

    for (pass, offsets) in histograms.iter_mut().enumerate() {
        let mut next = 0;
        for count in offsets.iter_mut() {
            let size = *count;
            *count = next;
            next += size;
        }

        let shift = shifts[pass];
        if pass % 2 == 0 {
            scatter(rows, scratch, offsets, shift);
        } else {
            // SAFETY: the previous scatter initialized every scratch slot.
            // Destination Row storage is already initialized and exclusive;
            // treating it as MaybeUninit does not read an invalid value.
            unsafe {
                let source = std::slice::from_raw_parts(scratch.as_ptr().cast::<Row>(), len);
                let target = std::slice::from_raw_parts_mut(
                    rows.as_mut_ptr().cast::<MaybeUninit<Row>>(),
                    len,
                );
                scatter(source, target, offsets, shift);
            }
        }
        // Forward source traversal and incrementing bucket cursors make each
        // pass stable. LSD order therefore sorts all four bytes by key alone.
    }

    if !DIGITS.is_multiple_of(2) {
        // SAFETY: An odd number of executed passes leaves all len sorted rows
        // initialized in scratch by the same partition argument above. Row is
        // Copy and MaybeUninit<Row> has Row's size/alignment. The two exclusive
        // input slices are disjoint and both cover len elements. No reference
        // to uninitialized Row storage is ever constructed.
        unsafe {
            std::ptr::copy_nonoverlapping(scratch.as_ptr().cast::<Row>(), rows.as_mut_ptr(), len);
        }
    }
}

// M3 Max uses 128-byte cache lines. Buffer one line per digit before writing
// contiguous runs: equal-size radix buckets otherwise place 256 write heads at
// power-of-two strides, a particularly bad pattern for CPU caches.
const BUFFER_ROWS: usize = 16;
#[repr(align(128))]
struct WriteBuffers([[MaybeUninit<Row>; BUFFER_ROWS]; 256]);

fn scatter(
    source: &[Row],
    target: &mut [MaybeUninit<Row>],
    offsets: &mut [usize; 256],
    shift: u32,
) {
    let mut buffers = WriteBuffers([[MaybeUninit::uninit(); BUFFER_ROWS]; 256]);
    let mut used = [0usize; 256];
    for &row in source {
        let digit = ((row.key >> shift) & 0xff) as usize;
        buffers.0[digit][used[digit]].write(row);
        used[digit] += 1;
        if used[digit] == BUFFER_ROWS {
            flush(&buffers.0[digit], target, &mut offsets[digit], BUFFER_ROWS);
            used[digit] = 0;
        }
    }
    for digit in 0..256 {
        if used[digit] != 0 {
            flush(&buffers.0[digit], target, &mut offsets[digit], used[digit]);
        }
    }
}

#[inline]
fn flush(
    source: &[MaybeUninit<Row>; BUFFER_ROWS],
    target: &mut [MaybeUninit<Row>],
    offset: &mut usize,
    count: usize,
) {
    // Only the initialized prefix is copied. Exclusive histogram ranges and
    // forward traversal preserve bucket chronology and cover every output row.
    target[*offset..*offset + count].copy_from_slice(&source[..count]);
    *offset += count;
}
