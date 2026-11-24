use crate::{decode, ColumnView};
use cuda_device::thread;

#[cuda_device::cuda_module]
pub mod rows {
    use super::*;
    use cuda_device::{kernel, launch_bounds, launch_contract};

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn hash_rows(
        columns: *const ColumnView,
        arity: usize,
        rows: usize,
        hashes: *mut u32,
        order: *mut u32,
    ) {
        let mut row = thread::index_1d().get();
        let stride = thread::gridDim_x() as usize * 256;
        while row < rows {
            let mut hash = 0x9e37_79b9_7f4a_7c15_u64;
            let mut column = 0;
            while column < arity {
                unsafe {
                    let view = *columns.add(column);
                    // DECODE V: projected-set hashing reconstructs encoded values for admission; P6 retain code-domain keys.
                    hash ^= decode(view, row as u64) as u64;
                }
                // Bijective avalanche per field. Hashes only delimit candidates;
                // exact tuple comparisons below decide equality.
                hash ^= hash >> 30;
                hash = hash.wrapping_mul(0xbf58_476d_1ce4_e5b9);
                hash ^= hash >> 27;
                hash = hash.wrapping_mul(0x94d0_49bb_1331_11eb);
                hash ^= hash >> 31;
                column += 1;
            }
            unsafe {
                *hashes.add(row) = hash as u32;
                *order.add(row) = row as u32;
            }
            row += stride;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn first_occurrences(
        columns: *const ColumnView,
        arity: usize,
        rows: usize,
        hashes: *const u32,
        order: *const u32,
        flags: *mut u32,
    ) {
        let mut slot = thread::index_1d().get();
        let stride = thread::gridDim_x() as usize * 256;
        while slot < rows {
            unsafe {
                let row = *order.add(slot) as usize;
                let hash = *hashes.add(slot);
                let mut first = true;
                // The common distinct-hash case needs no tuple reads or search.
                if slot != 0 && *hashes.add(slot - 1) == hash {
                    let mut low = 0;
                    let mut high = slot;
                    while low < high {
                        let middle = low + (high - low) / 2;
                        if *hashes.add(middle) < hash {
                            low = middle + 1;
                        } else {
                            high = middle;
                        }
                    }
                    // Stable radix order preserves original positions within a
                    // hash run. Check ALL preceding candidates, not just the
                    // adjacent tuple: colliding distinct tuples can interleave.
                    let mut candidate = low;
                    while candidate < slot {
                        let other = *order.add(candidate) as usize;
                        let mut column = 0;
                        while column < arity {
                            let view = *columns.add(column);
                            // DECODE V: exact collision verification reconstructs the current value; P6 retain code-domain equality.
                            let value = decode(view, row as u64);
                            // DECODE V: exact collision verification reconstructs the preceding candidate; P6 retain code-domain equality.
                            let preceding = decode(view, other as u64);
                            if value != preceding {
                                break;
                            }
                            column += 1;
                        }
                        if column == arity {
                            first = false;
                            break;
                        }
                        candidate += 1;
                    }
                }
                // Each permutation entry owns one original-position flag.
                *flags.add(row) = first as u32;
            }
            slot += stride;
        }
    }
}
