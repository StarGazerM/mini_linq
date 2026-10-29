use std::collections::{BTreeMap, BTreeSet};
use std::panic::{UnwindSafe, catch_unwind};

fn rejects_duplicate(run: impl FnOnce() + UnwindSafe) {
    let panic = catch_unwind(run).expect_err("duplicate host rows must fail certification");
    let message = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied());
    assert_eq!(
        message,
        Some("host_set E certificate failed: duplicate full host row")
    );
}

#[test]
fn distinct_full_host_rows_execute() {
    for rows in [vec![], vec![(1, 10)], vec![(1, 10), (1, 11), (2, 10)]] {
        let actual: BTreeSet<_> = mini_linq::compile! {
            pipeline { physical = [cq_rewrites::host_sets(E)]; }
            struct CertifiedRows;
            relation E(a:i32, b:i32) = (rows);
            answer(a,b) :- E(a,b);
        }
        .map(|(a, b)| (*a, *b))
        .collect();
        assert_eq!(actual, rows.iter().copied().collect());
    }
}

#[test]
fn full_heading_certificate_does_not_claim_projected_uniqueness() {
    let rows = [(1, 10), (1, 11), (2, 10)];
    let mut actual: Vec<_> = mini_linq::compile! {
        pipeline { physical = [cq_rewrites::host_sets(E)]; }
        struct CertifiedProjection;
        relation E(a:i32, b:i32) = (rows);
        answer(a) :- E(a,b);
    }
    .map(|(a,)| *a)
    .collect();
    actual.sort_unstable();
    assert_eq!(actual, [1, 2]);
}

#[test]
fn duplicates_fail_before_consumption_even_when_filtered_out() {
    rejects_duplicate(|| {
        let rows = [(1, 10), (2, 20), (1, 10)];
        let _unconsumed = mini_linq::compile! {
            pipeline { physical = [cq_rewrites::host_sets(E)]; }
            struct CertifiedFilteredRows;
            relation E(a:i32, b:i32) = (rows);
            answer(a,b) :- E(a,b), where a < 0;
        };
    });
}

#[test]
fn grouped_hosts_are_certified_as_full_rows() {
    let rows = BTreeMap::from([((1,), vec![(10,), (11,)]), ((2,), vec![(10,)])]);
    let actual: BTreeSet<_> = mini_linq::compile! {
        pipeline { physical = [cq_rewrites::host_sets(E)]; }
        struct CertifiedGroupedRows;
        relation E(a:i32, b:i32) = (rows);
        answer(a,b) :- E(a,b);
    }
    .map(|(a, b)| (*a, *b))
    .collect();
    assert_eq!(actual, BTreeSet::from([(1, 10), (1, 11), (2, 10)]));
}

#[test]
fn grouped_hosts_cannot_hide_duplicate_payload_rows() {
    rejects_duplicate(|| {
        let rows = BTreeMap::from([((1,), vec![(10,), (11,), (10,)])]);
        let _unconsumed = mini_linq::compile! {
            pipeline { physical = [cq_rewrites::host_sets(E)]; }
            struct DuplicateGroupedRows;
            relation E(a:i32, b:i32) = (rows);
            answer(a,b) :- E(a,b);
        };
    });
}

#[test]
fn uncertified_duplicate_hosts_keep_default_set_semantics() {
    let rows = [(1, 10), (1, 10), (2, 20)];
    let mut actual: Vec<_> = mini_linq::compile! {
        pipeline {}
        struct UncertifiedRows;
        relation E(a:i32, b:i32) = (rows);
        answer(a,b) :- E(a,b);
    }
    .map(|(a, b)| (*a, *b))
    .collect();
    actual.sort_unstable();
    assert_eq!(actual, [(1, 10), (2, 20)]);
}

#[derive(PartialEq, Eq)]
struct Collision(i32);

impl std::hash::Hash for Collision {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        state.write_u8(0);
    }
}

#[test]
fn certification_compares_values_exactly_without_cloning_columns() {
    let rows = [(Collision(1),), (Collision(2),), (Collision(3),)];
    let mut actual: Vec<_> = mini_linq::compile! {
        pipeline { physical = [cq_rewrites::host_sets(E)]; }
        struct CollidingDistinctRows;
        relation E(a:Collision) = (rows);
        answer(a) :- E(a);
    }
    .map(|(a,)| a.0)
    .collect();
    actual.sort_unstable();
    assert_eq!(actual, [1, 2, 3]);

    rejects_duplicate(|| {
        let rows = [(Collision(1),), (Collision(2),), (Collision(1),)];
        let _unconsumed = mini_linq::compile! {
            pipeline { physical = [cq_rewrites::host_sets(E)]; }
            struct CollidingDuplicateRows;
            relation E(a:Collision) = (rows);
            answer(a) :- E(a);
        };
    });
}

#[test]
fn certification_reuses_the_single_bound_host_leaf() {
    let calls = std::cell::Cell::new(0);
    let rows = [(1, 10), (2, 20)];
    let actual: BTreeSet<_> = mini_linq::compile! {
        pipeline { physical = [cq_rewrites::host_sets(E)]; }
        struct CertifiedLeaf;
        relation E(a:i32, b:i32) = ({ calls.set(calls.get() + 1); &rows });
        answer(a,b) :- E(a,b);
    }
    .map(|(a, b)| (*a, *b))
    .collect();
    assert_eq!(actual, BTreeSet::from([(1, 10), (2, 20)]));
    assert_eq!(calls.get(), 1);
}
