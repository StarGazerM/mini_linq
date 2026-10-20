//! Exact, opt-in certificates for the entire declared host heading.
//!
//! Sortedness is established on device over every adjacent full-row pair; a
//! sorted, distinct input needs only that pass and two status words. Unsorted
//! input falls back to an exact device hash set containing immutable row IDs.
//! Hash collisions always compare every column. No tuples are downloaded.
use crate::*;

fn duplicate(relation: &str) -> Error {
    failure(
        "CQ_GPU_HOST_SET_DUPLICATE",
        format!("host relation `{relation}` violates host_set: duplicate full row"),
    )
}

fn sorted_certificate(flags: [u32; 2], relation: &str) -> Result<bool> {
    if flags[1] != 0 {
        return Err(duplicate(relation));
    }
    Ok(flags[0] == 0)
}

fn slot_capacity(rows: usize) -> Result<usize> {
    rows.max(1)
        .checked_mul(2)
        .and_then(usize::checked_next_power_of_two)
        .ok_or_else(|| {
            failure(
                "CQ_GPU_HOST_SET_CAPACITY",
                "host-set hash capacity overflow",
            )
        })
}

impl Execution<'_, '_> {
    /// Certify all columns of the bound host before launching any consuming
    /// operator. Index keys, projections and declared sort order are not proof.
    pub fn certify_host_set(&self, input: TableId, relation: &str) -> Result<()> {
        let table = self.table(input)?;
        // Validate every retained ancestor before treating an empty/error input
        // as a set. Only count/status metadata crosses the host boundary.
        record_host_wait(true);
        let rows = gpu_device_columns::checked_count(table)?;
        if rows < 2 {
            return Ok(());
        }
        let runtime = self.runtime;
        let flags = Buffer::<u32>::zeroed(runtime.pool.clone(), 2)?;
        let launch = runtime.kernels.prepare_host_set_adjacent(config(rows, 0))?;
        unsafe {
            runtime.kernels.host_set_adjacent(
                runtime.stream(),
                &launch,
                table.view(),
                table.columns.len(),
                flags.ptr(),
            )?;
        }
        record_host_wait(true);
        let status = flags.to_host()?;
        if sorted_certificate([status[0], status[1]], relation)? {
            return Ok(());
        }
        let capacity = slot_capacity(rows)?;
        let slots = Buffer::<u64>::zeroed(runtime.pool.clone(), capacity)?;
        let launch = runtime.kernels.prepare_host_set_hash(config(rows, 0))?;
        unsafe {
            runtime.kernels.host_set_hash(
                runtime.stream(),
                &launch,
                table.view(),
                table.columns.len(),
                slots.ptr(),
                capacity,
                flags.ptr(),
            )?;
        }
        record_host_wait(true);
        let status = flags.to_host()?;
        if status[1] != 0 {
            return Err(duplicate(relation));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_verified_sorted_distinct_rows_skip_hashing() {
        assert!(sorted_certificate([0, 0], "Edges").unwrap());
        assert!(!sorted_certificate([1, 0], "Edges").unwrap());
        for flags in [[0, 1], [1, 1]] {
            let error = sorted_certificate(flags, "Edges").unwrap_err();
            let error = error.downcast::<RuntimeError>().unwrap();
            assert_eq!(error.code, "CQ_GPU_HOST_SET_DUPLICATE");
            assert_eq!(
                error.detail,
                "host relation `Edges` violates host_set: duplicate full row"
            );
        }
    }

    #[test]
    fn hash_table_has_empty_slots_and_checked_capacity() {
        for rows in [0, 1, 2, 3, 127, 128, 129, 4096] {
            let capacity = slot_capacity(rows).unwrap();
            assert!(capacity.is_power_of_two());
            assert!(capacity >= rows.max(1) * 2);
        }
        let error = slot_capacity(usize::MAX).unwrap_err();
        assert_eq!(
            error.downcast::<RuntimeError>().unwrap().code,
            "CQ_GPU_HOST_SET_CAPACITY"
        );
    }
}
