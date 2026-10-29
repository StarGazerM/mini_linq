use cuda_device::thread;
use gpu_device_columns::{decode_source, decode_source_u32, SourceView};

#[cuda_device::cuda_module]
pub mod build {
    use super::*;
    use cuda_device::{kernel, launch_bounds, launch_contract};

    /// Exact numeric source bounds, including strided and packed publications.
    /// Every lane participates, including lanes outside a short final prefix.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn numeric_bounds(input: SourceView, rows: u32, partial: *mut i64) {
        let lane = thread::index_1d().get();
        let stride = thread::gridDim_x() as usize * 256;
        let mut row = lane;
        let mut min = i64::MAX;
        let mut max = i64::MIN;
        while row < rows as usize {
            // DECODE V: bound reduction reconstructs every source value before index planning; P6 reuse recorded encoded-domain bounds.
            let value = unsafe { gpu_device_columns::decode_source(input, row as u64) };
            min = min.min(value);
            max = max.max(value);
            row += stride;
        }
        let mut distance = 16;
        while distance > 0 {
            min = min.min(cuda_device::warp::shuffle_xor_u64(min as u64, distance) as i64);
            max = max.max(cuda_device::warp::shuffle_xor_u64(max as u64, distance) as i64);
            distance /= 2;
        }
        if lane % 32 == 0 {
            unsafe {
                *partial.add((lane / 32) * 2) = min;
                *partial.add((lane / 32) * 2 + 1) = max;
            }
        }
    }

    /// Decode a key through an optional base-row permutation.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn keys_and_rows(
        key: SourceView,
        source: *const u32,
        rows: u32,
        keys: *mut u32,
        order: *mut u32,
    ) {
        let mut i = thread::index_1d().get() as u32;
        let stride = thread::gridDim_x() * 256;
        while i < rows {
            unsafe {
                let row = if source.is_null() {
                    i
                } else {
                    *source.add(i as usize)
                };
                // DECODE V: index key extraction reconstructs values before sorting; P6 sort recorded codes or keep index runs plain.
                *keys.add(i as usize) = decode_source_u32(key, row as u64);
                *order.add(i as usize) = row;
            }
            i = match i.checked_add(stride) {
                Some(next) => next,
                None => break,
            };
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    // DECODE V: multi-key index build expands each column before lexicographic sorting; P6 retain encoded keys.
    pub fn sequence_decode_column(key: SourceView, source: *const u32, rows: u32, out: *mut u32) {
        let mut i = thread::index_1d().get() as u32;
        let stride = thread::gridDim_x() * 256;
        while i < rows {
            unsafe {
                let row = if source.is_null() {
                    i
                } else {
                    *source.add(i as usize)
                };
                // DECODE V: permutation gather reconstructs keys for a later sort; P6 compare encoded fields.
                *out.add(i as usize) = decode_source_u32(key, row as u64);
            }
            i = match i.checked_add(stride) {
                Some(next) => next,
                None => break,
            };
        }
    }

    /// Selection uses decoded integers, including negative signed payloads.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn range_flags(column: SourceView, rows: u32, start: i64, end: i64, flags: *mut u32) {
        let mut row = thread::index_1d().get() as u32;
        let stride = thread::gridDim_x() * 256;
        while row < rows {
            unsafe {
                // DECODE V: range selection decodes before relational comparison; P6 translate bounds into the encoded domain.
                let value = decode_source(column, row as u64);
                *flags.add(row as usize) = (start <= value && value < end) as u32;
            }
            row = match row.checked_add(stride) {
                Some(next) => next,
                None => break,
            };
        }
    }
}
