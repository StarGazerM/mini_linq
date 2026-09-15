//! Minimal native macro protocols execute without unrelated configuration.
#[cfg(feature = "scan-rows")]
#[test]
fn scan_discards_metadata_and_observer_without_configuration() {
    use std::cell::Cell;
    macro_rules! rows {
        (rows ($value:expr)) => {
            ($value).iter()
        };
    }
    let values = [[1_i64], [2]];
    let calls = Cell::new(0);
    let result = mini_linq::scan_rows! {
        work (compile_error!("scan must discard the observer"));
        candidates rows { { calls.set(calls.get() + 1); &values } }
            (compile_error!("scan must discard metadata-only syntax"))
    };
    assert_eq!(result.copied().collect::<Vec<_>>(), values);
    assert_eq!(calls.get(), 1);
}

#[cfg(all(feature = "exact-keys", feature = "pull", feature = "stream-pipeline"))]
#[test]
fn key_build_uses_no_membership_trait() {
    use std::collections::HashSet;
    let keys = mini_linq::exact_keys! {
        iterator mini_linq::stream_access;
        keys ::std::collections::HashSet;
        build { out(k) => ([k]) :- Keys(k) => for (k,) in ([(1_i64,), (2,), (1,)]); }
    };
    assert_eq!(keys, HashSet::from([[1], [2]]));
}

#[cfg(feature = "exact-keys")]
#[test]
fn membership_uses_no_build_iterator() {
    let keys = std::collections::HashSet::from([[1_i64], [2]]);
    assert!(mini_linq::exact_keys! { storage cq_storage; contains (&keys) ([2]) });
    assert!(!mini_linq::exact_keys! { storage cq_storage; contains (&keys) ([3]) });
}

#[cfg(feature = "hash-rows")]
#[test]
fn hash_materialization_collects_support_streams_without_storage_options() {
    use std::{cell::Cell, collections::HashSet};
    let calls = Cell::new(0);
    mini_linq::hash_rows! {
        work (compile_error!("materialization does not observe traversal"));
        materialize hash_owner hash_view ({ calls.set(calls.get() + 1); [[2_i64], [1], [2]].into_iter() })
    }
    assert_eq!(hash_view, &HashSet::from([[1], [2]]));
    assert_eq!(calls.get(), 1);
}

#[cfg(feature = "sorted-rows")]
#[test]
fn sorted_materialization_collects_support_streams_without_storage_options() {
    use std::collections::BTreeSet;
    mini_linq::sorted_rows! {
        materialize sorted_owner sorted_view ([[2_i64], [1], [2]].into_iter())
    }
    assert_eq!(sorted_view, &BTreeSet::from([[1], [2]]));
}

#[cfg(feature = "block-rows")]
#[test]
fn block_packing_is_bound_and_evaluates_once_per_construction() {
    use std::{cell::Cell, collections::HashSet};
    let row_calls = Cell::new(0);
    let size_calls = Cell::new(0);
    macro_rules! blocks {
        ($($op:tt)*) => {
            mini_linq::block_rows! {
                storage cq_storage;
                packing ({ size_calls.set(size_calls.get() + 1); std::num::NonZeroUsize::new(1).unwrap() });
                $($op)*
            }
        };
    }
    blocks! {
        materialize owner view ({ row_calls.set(row_calls.get() + 1); [[2_i64], [1], [2]].into_iter() })
    }
    let rows = blocks! { scan (&view) }.copied().collect::<HashSet<_>>();
    assert_eq!(rows, HashSet::from([[1], [2]]));
    assert_eq!((row_calls.get(), size_calls.get()), (1, 1));
    assert_eq!(blocks! { len (&view) }, 2);
    assert_eq!(size_calls.get(), 1);
}

#[cfg(feature = "block-rows")]
#[test]
fn packing_helper_does_not_shadow_direct_or_opaque_caller_expressions() {
    use std::{cell::RefCell, collections::HashSet, num::NonZeroUsize};
    fn __cq_pack(order: &RefCell<Vec<&'static str>>) -> NonZeroUsize {
        order.borrow_mut().push("packing");
        NonZeroUsize::new(1).unwrap()
    }
    macro_rules! caller_packing {
        ($order:expr) => {
            __cq_pack($order)
        };
    }
    let order = RefCell::new(Vec::new());
    mini_linq::block_rows! {
        storage cq_storage;
        packing (__cq_pack(&order));
        materialize direct_owner direct_view ({
            order.borrow_mut().push("rows");
            [[1_i64], [3]]
        })
    }
    mini_linq::block_rows! {
        storage cq_storage;
        packing (caller_packing!(&order));
        materialize opaque_owner opaque_view ({
            order.borrow_mut().push("rows");
            [[1_i64], [3]]
        })
    }
    assert_eq!(*order.borrow(), ["rows", "packing", "rows", "packing"]);
    for view in [&direct_view, &opaque_view] {
        let rows = mini_linq::block_rows! { storage cq_storage; scan (view) };
        assert_eq!(
            rows.copied().collect::<HashSet<_>>(),
            HashSet::from([[1], [3]])
        );
    }
}

#[cfg(all(
    feature = "prune-rows",
    feature = "block-rows",
    feature = "pull",
    feature = "stream-pipeline"
))]
#[test]
fn predicate_helper_does_not_shadow_direct_or_opaque_caller_expressions() {
    use std::cell::Cell;
    fn __cq_bind_predicate(calls: &Cell<usize>) -> impl Fn(&[(i64, i64)]) -> bool {
        calls.set(calls.get() + 1);
        |bounds| bounds[0].0 <= 2
    }
    macro_rules! caller_predicate {
        ($calls:expr) => {
            __cq_bind_predicate($calls)
        };
    }
    macro_rules! blocks {
        ($($op:tt)*) => { mini_linq::block_rows! { storage cq_storage; $($op)* } };
    }
    mini_linq::block_rows! {
        storage cq_storage;
        packing (std::num::NonZeroUsize::new(1).unwrap());
        materialize owner view ([[1_i64], [3]])
    }
    let calls = Cell::new(0);
    let direct = mini_linq::prune_rows! {
        iterator mini_linq::stream_access;
        candidates blocks { &view } (__cq_bind_predicate(&calls))
    };
    assert_eq!(direct.map(|handle| *handle.3).collect::<Vec<_>>(), [[1]]);
    let opaque = mini_linq::prune_rows! {
        iterator mini_linq::stream_access;
        candidates blocks { &view } (caller_predicate!(&calls))
    };
    assert_eq!(opaque.map(|handle| *handle.3).collect::<Vec<_>>(), [[1]]);
    assert_eq!(calls.get(), 2);
}
