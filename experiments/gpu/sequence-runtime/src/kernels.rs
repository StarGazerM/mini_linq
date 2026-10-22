use cuda_device::thread;
use gpu_device_columns::{decode_source, decode_source_u32, SourceView};

#[cuda_device::cuda_module]
pub mod build {
    use super::*;
    use cuda_device::{kernel, launch_bounds, launch_contract};

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
