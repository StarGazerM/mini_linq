//! The build script prints real Rust through the unchanged downstream passes;
//! rustc compiles it here, and the tests execute it against an independent oracle.
//!
//! The meaning is `T X = bottom + Set X`, with one abort and an opaque partial
//! Rust guard after the relational join. Other effects and evaluation counts
//! are outside this meaning.
// The current theta query uses host reads; retain the slot for built let reads.
#[allow(unused_macros)]
macro_rules! indexes {
    (probe) => { ::cq_storage::HashIndex<'_, _, _> };
    (semi) => { ::std::collections::HashSet<_> };
    (ordered) => { ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> };
    (trie $inner:ty) => { ::cq_storage::Trie<'_, _, $inner> };
}
pub(crate) use indexes as index_provider;
macro_rules! distinct_state {
    (operator $id:ident; new) => {
        ::std::collections::HashSet::new()
    };
    (operator $id:ident; insert($state:expr, $row:expr)) => {
        $state.insert(Clone::clone($row))
    };
}
include!(concat!(env!("OUT_DIR"), "/query.rs"));

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    fn oracle(left: &[(i32, i32)], right: &[(i32, i32)]) -> BTreeSet<(i32, i32)> {
        left.iter()
            .flat_map(|&(k, x)| {
                right
                    .iter()
                    .filter_map(move |&(l, y)| (k == l && x < y).then_some((x, y)))
            })
            .collect()
    }
    #[test]
    fn theta_join_executes_through_both_unmodified_physical_routes() {
        let left = [(1, 2), (1, 2), (1, 9), (2, 3), (3, 0)];
        let right = [(1, 5), (1, 5), (1, 8), (2, 1), (2, 7), (4, 9)];
        for run in [run_probe, run_merge] {
            for (l, r) in [
                (&left[..], &right[..]),
                (&[][..], &right[..]),
                (&left[..], &[][..]),
            ] {
                let rows = run(l, r);
                let set: BTreeSet<_> = rows.iter().copied().collect();
                assert_eq!(rows.len(), set.len(), "returned rows must be distinct");
                assert_eq!(set, oracle(l, r));
            }
        }
    }
    #[test]
    fn small_inputs_match_the_nested_loop_oracle() {
        let domain = [(0, 0), (0, 1), (1, 0), (1, 2)];
        for l in 0..16 {
            for r in 0..16 {
                let left: Vec<_> = domain
                    .iter()
                    .enumerate()
                    .filter_map(|(i, row)| (l & (1 << i) != 0).then_some(*row))
                    .collect();
                let right: Vec<_> = domain
                    .iter()
                    .enumerate()
                    .filter_map(|(i, row)| (r & (1 << i) != 0).then_some(*row))
                    .collect();
                for run in [run_probe, run_merge] {
                    assert_eq!(
                        run(&left, &right).into_iter().collect::<BTreeSet<_>>(),
                        oracle(&left, &right)
                    );
                }
            }
        }
    }
}
