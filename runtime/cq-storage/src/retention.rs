//! Accounting companions for compiler-owned standard row/set allocations.
//! Custom providers account for their own ownership; borrowed bases are excluded.

/// Keeps a measured ownership allocation live without changing its provider.
pub struct Retention {
    _allocation: crate::counters_memory::Allocation,
}
/// Record a standard Vec's actual capacity and copied tuple occurrences.
/// This performs no allocation or traversal and is a no-op without counters.
#[allow(clippy::ptr_arg)]
pub fn retain_rows<T>(rows: &Vec<T>) -> Retention {
    #[cfg(feature = "counters")]
    crate::counters::record(|c| c.rows_copied += rows.len() as u64);
    Retention {
        _allocation: crate::counters_memory::Allocation::new(
            rows.capacity() * std::mem::size_of::<T>(),
        ),
    }
}
/// Record key cells and control bytes; allocator overhead is not estimated.
pub fn retain_keys<K, S>(keys: &std::collections::HashSet<K, S>) -> Retention {
    Retention {
        _allocation: crate::counters_memory::Allocation::new(
            keys.capacity() * (std::mem::size_of::<K>() + 1),
        ),
    }
}
/// Count a declarative residual test, independently of the chosen read adapter.
#[inline]
pub fn residual(result: bool) -> bool {
    crate::count!(rows_tested);
    result
}
