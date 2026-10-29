//! Positional carriers fetch logical columns and stop at value boundaries.
use std::collections::BTreeSet;

#[test]
fn represent_pipeline_fetches_filtered_columns() {
    let rows = [(1_i32, 10_i32), (2, 20), (3, 30)];
    let actual: BTreeSet<_> = mini_linq::compile! {
        pipeline { represent = ::mini_linq::passes::represent(positions); }
        struct RepresentScan;
        relation R(k:i32,v:i32) = (rows);
        answer(v) :- R(k,v), where k >= 2;
    }
    .map(|(v,)| *v)
    .collect();
    assert_eq!(actual, BTreeSet::from([20, 30]));
}

#[test]
fn positions_feed_probe_keys_and_owned_bind_outputs() {
    let left = [(1_i32,), (2,), (3,)];
    let right = [(2_i32, 20_i32), (3, 30)];
    let actual: BTreeSet<_> = mini_linq::compile! {
        pipeline { represent = ::mini_linq::passes::represent(positions); }
        struct RepresentProbe;
        relation L(k:i32) = (left);
        relation R(k:i32,v:i32) = (right);
        answer(k,n) :- L(k), where k >= 2, R(k,v), let n: i32 = *v + 1;
    }
    .map(|(k, n)| (*k, n))
    .collect();
    assert_eq!(actual, BTreeSet::from([(2, 21), (3, 31)]));
}

#[test]
fn positions_fetch_for_opaque_guards_and_computed_heads() {
    fn keep(k: &i32) -> bool {
        *k != 2
    }
    let rows = [(1_i32,), (2,), (3,)];
    let actual: BTreeSet<_> = mini_linq::compile! {
        pipeline { represent = ::mini_linq::passes::represent(positions); }
        struct RepresentGuard;
        relation R(k:i32) = (rows);
        relation answer(value:i32);
        answer(*k + 10) :- R(k), if keep(k);
    }
    .map(|(k,)| k)
    .collect();
    assert_eq!(actual, BTreeSet::from([11, 13]));
}
