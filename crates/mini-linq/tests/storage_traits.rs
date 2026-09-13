//! A different layout implements Rust operations; provider and algorithm macro
//! bodies are reused without knowing its payload layout or coordinate types.
#[path = "support/alternate_blocks.rs"]
mod alternate;

use alternate::FlatBlocks;
use cq_storage::BlockStore;
use std::collections::{BTreeSet, HashSet};
use std::num::NonZeroUsize;

fn size(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).unwrap()
}

// Only a lexical substitution is required for this binding's representation.
// Every operation is still expanded by the existing stock block provider.
macro_rules! alternate_blocks {
    (storage $storage:path; $($request:tt)*) => {
        ::mini_linq::block_rows! { storage crate::alternate; $($request)* }
    };
}
pub(crate) use alternate_blocks;

// Independently replace the conservative summary implementation, retaining the
// existing zone recipe and the independently selected operand providers.
macro_rules! alternate_zone {
    (storage $storage:path; $($request:tt)*) => {
        ::mini_linq::zone_semijoin! { storage crate::alternate; $($request)* }
    };
}
pub(crate) use alternate_zone;

macro_rules! mixed_plan {
    ($name:ident, $recipe:path) => {
        ::mini_linq::physical_plan! {
            storage ::cq_storage;
            access ::mini_linq::collect_access;
            materialize ::mini_linq::hash_rows;
            inputs {
                R => crate::alternate_blocks;
                S => ::mini_linq::block_rows;
                T => ::mini_linq::hash_rows;
            }
            results {}
            semijoin $recipe;
            semijoins { s1 => ::mini_linq::zone_semijoin; }
            keys crate::alternate::OrderedKeys;
            key_sets {}
            join ::mini_linq::hash_join;
            joins {}
            struct $name;
            relation R(k:i64,x:i64);
            relation S(k:i64,y:i64);
            relation T(k:i64);
            answer(x,y) :- R(k,x), S(k,y), T(k);
            relational {
                r = rename R {k -> k,x -> x};
                s = rename S {k -> k,y -> y};
                t = rename T {k -> k};
                r1 = semijoin r with t;
                s1 = semijoin s with r1;
                r2 = semijoin r1 with s1;
                joined = natural_join r2 with s1;
                result = project joined keep {x,y};
                output result as answer(x,y);
            }
        }
    };
}

mixed_plan!(MixedZone, crate::alternate_zone);
mixed_plan!(MixedExact, ::mini_linq::exact_semijoin);

#[test]
fn sparse_coordinates_flat_payload_and_stock_blocks_coexist_in_one_query() {
    let rows = |mask: u8| {
        (0..4)
            .filter(|bit| mask & (1 << bit) != 0)
            .map(|bit| [i64::from(bit / 2), i64::from(bit % 2)])
            .collect::<Vec<_>>()
    };
    for r in 0..16 {
        for s in 0..16 {
            for t in 0..4 {
                let (r, s) = (rows(r), rows(s));
                let t: HashSet<_> = (0..2)
                    .filter(|bit| t & (1 << bit) != 0)
                    .map(|bit| [i64::from(bit)])
                    .collect();
                let mut expected = BTreeSet::new();
                for &[k, x] in &r {
                    for &[other_k, y] in &s {
                        if k == other_k && t.contains(&[k]) {
                            expected.insert((x, y));
                        }
                    }
                }
                let expected: Vec<_> = expected.into_iter().collect();
                let r = FlatBlocks::from_records(r, size(1));
                let s = BlockStore::from_rows(s, size(2));
                assert_eq!(MixedZone::run(&r, &s, &t, size(2)), expected);
                assert_eq!(MixedExact::run(&r, &s, &t, size(2)), expected);
            }
        }
    }
}

macro_rules! reduction_plan {
    ($name:ident, $recipe:path) => {
        ::mini_linq::physical_plan! {
            storage ::cq_storage;
            access ::mini_linq::collect_access;
            materialize ::mini_linq::hash_rows;
            inputs { R => crate::alternate_blocks; S => ::mini_linq::hash_rows; }
            results {}
            semijoin $recipe;
            semijoins {}
            keys crate::alternate::OrderedKeys;
            key_sets {}
            join ::mini_linq::hash_join;
            joins {}
            struct $name;
            relation R(k:i64,x:i64);
            relation S(k:i64);
            answer(x) :- R(k,x), S(k);
            relational {
                r = rename R {k -> k,x -> x};
                s = rename S {k -> k};
                reduced = semijoin r with s;
                result = project reduced keep {x};
                output result as answer(x);
            }
        }
    };
}

reduction_plan!(RejectedZone, crate::alternate_zone);
reduction_plan!(RejectedExact, ::mini_linq::exact_semijoin);

#[test]
fn rejected_metadata_never_opens_the_alternate_payload() {
    let r = FlatBlocks::from_records([[1, 10], [2, 20], [3, 30]], size(1));
    let s = HashSet::from([[90]]);
    let (answer, stats) = RejectedZone::run_with_stats(&r, &s, size(2));
    assert!(answer.is_empty());
    assert_eq!(r.payload_opens(), 0);
    let reduced = stats.steps.iter().find(|step| step.0 == "reduced").unwrap();
    assert_eq!(reduced.2.blocks_skipped, 3);
    assert_eq!(reduced.2.exact_probes, 0);
    assert!(RejectedExact::run(&r, &s, size(2)).is_empty());
    assert_eq!(r.payload_opens(), 3);
}

macro_rules! composite_plan {
    ($name:ident, $recipe:path) => {
        ::mini_linq::physical_plan! {
            storage ::cq_storage;
            access ::mini_linq::collect_access;
            materialize ::mini_linq::hash_rows;
            inputs { R => crate::alternate_blocks; S => ::mini_linq::hash_rows; }
            results {}
            semijoin $recipe;
            semijoins {}
            keys crate::alternate::OrderedKeys;
            key_sets {}
            join ::mini_linq::hash_join;
            joins {}
            struct $name;
            relation R(a:i64,b:i64,x:i64);
            relation S(b:i64,a:i64);
            answer(x) :- R(a,b,x), S(b,a);
            relational {
                r = rename R {a -> a,b -> b,x -> x};
                s = rename S {b -> b,a -> a};
                reduced = semijoin r with s;
                result = project reduced keep {x};
                output result as answer(x);
            }
        }
    };
}

composite_plan!(CoordinateEnvelope, ::mini_linq::zone_semijoin);
composite_plan!(CorrelatedEnvelope, crate::alternate_zone);

#[test]
fn a_different_summary_refines_pruning_without_changing_the_zone_recipe() {
    let r = FlatBlocks::from_records([[1, 2, 10], [0, 4, 20], [2, 0, 30]], size(1));
    let s = HashSet::from([[4, 0], [0, 2]]);
    let (coordinate_answer, coordinate) = CoordinateEnvelope::run_with_stats(&r, &s, size(1));
    let coordinate_opens = r.payload_opens();
    let (correlated_answer, correlated) = CorrelatedEnvelope::run_with_stats(&r, &s, size(1));
    let correlated_opens = r.payload_opens() - coordinate_opens;
    assert_eq!(coordinate_answer, vec![(20,), (30,)]);
    assert_eq!(coordinate_answer, correlated_answer);
    let coordinate = coordinate
        .steps
        .iter()
        .find(|step| step.0 == "reduced")
        .unwrap()
        .2;
    let correlated = correlated
        .steps
        .iter()
        .find(|step| step.0 == "reduced")
        .unwrap()
        .2;
    // The stock envelope admits (1,2) inside [0,2] x [0,4], and the residual
    // rejects it. The alternate summary knows no observed whole key fits that
    // singleton block and rejects its metadata before opening the payload.
    assert_eq!((coordinate.blocks_skipped, coordinate.exact_probes), (0, 3));
    assert_eq!((correlated.blocks_skipped, correlated.exact_probes), (1, 2));
    assert_eq!(coordinate_opens - correlated_opens, 1);
}

mini_linq::physical_plan! {
    storage ::cq_storage;
    access ::mini_linq::collect_access;
    materialize ::mini_linq::hash_rows;
    inputs {
        R => crate::alternate_blocks;
        S => ::mini_linq::hash_rows;
        T => ::mini_linq::hash_rows;
    }
    results {}
    semijoin crate::alternate_zone;
    semijoins {}
    keys crate::alternate::OrderedKeys;
    key_sets {}
    join ::mini_linq::hash_join;
    joins {}
    struct Versioned;
    relation R(k:i64,x:i64);
    relation S(k:i64);
    relation T(k:i64);
    answer(x) :- R(k,x), S(k), T(k);
    relational {
        r = rename R {k -> k,x -> x};
        s = rename S {k -> k};
        t = rename T {k -> k};
        r1 = semijoin r with s;
        r2 = semijoin r1 with t;
        r3 = semijoin r2 with s;
        result = project r3 keep {x};
        output result as answer(x);
    }
}

#[test]
fn the_sparse_selection_preserves_reductions_when_a_block_is_reopened() {
    let r = FlatBlocks::from_records([[1, 10], [2, 20], [3, 30], [4, 40]], size(4));
    let s = HashSet::from([[1], [2], [3]]);
    let t = HashSet::from([[2]]);
    let (answer, stats) = Versioned::run_with_stats(&r, &s, &t, size(2));
    assert_eq!(answer, vec![(20,)]);
    for (name, expected) in [("r1", 3), ("r2", 1), ("r3", 1)] {
        let step = stats.steps.iter().find(|step| step.0 == name).unwrap();
        assert_eq!(step.1, expected);
    }
    let final_reduction = stats.steps.iter().find(|step| step.0 == "r3").unwrap();
    assert_eq!(final_reduction.2.exact_probes, 1);
}

mini_linq::physical_plan! {
    storage ::cq_storage;
    access ::mini_linq::collect_access;
    materialize ::mini_linq::hash_rows;
    inputs { R => ::mini_linq::sorted_rows; S => ::mini_linq::hash_rows; }
    results { packed => crate::alternate_blocks; }
    semijoin crate::alternate_zone;
    semijoins {}
    keys crate::alternate::OrderedKeys;
    key_sets {}
    join ::mini_linq::hash_join;
    joins {}
    struct Materialized;
    relation R(k:i64,x:i64);
    relation S(k:i64);
    answer(x) :- R(k,x), S(k);
    relational {
        r = rename R {k -> k,x -> x};
        s = rename S {k -> k};
        packed = project r keep {k,x};
        reduced = semijoin packed with s;
        result = project reduced keep {x};
        output result as answer(x);
    }
}

#[test]
fn materialization_is_available_only_through_the_build_trait() {
    let r = BTreeSet::from([[1, 10], [2, 20], [3, 30]]);
    let s = HashSet::from([[2]]);
    let before = alternate::materialization_count();
    let (answer, stats) = Materialized::run_with_stats(&r, &s, size(1));
    assert_eq!(answer, vec![(20,)]);
    // The identically named inherent constructor deliberately panics.
    assert_eq!(alternate::materialization_count() - before, 1);
    let reduced = stats.steps.iter().find(|step| step.0 == "reduced").unwrap();
    assert_eq!(reduced.2.blocks_skipped, 2);
}

#[test]
fn zero_column_provenance_uses_the_alternate_identity_type() {
    let left = FlatBlocks::<0>::from_records([[]], size(1));
    let right = FlatBlocks::<0>::from_records([[]], size(1));
    let work = alternate::Counters::default();
    let left = alternate_blocks! { storage cq_storage; input (&left) };
    let right = alternate_blocks! { storage cq_storage; input (&right) };
    let own: HashSet<_> =
        alternate_blocks! { storage cq_storage; work (&work); rows (&left) }.collect();
    let own = alternate_blocks! { storage cq_storage; select (&left) (own) };
    assert_eq!(alternate_blocks! { storage cq_storage; len (&own) }, 1);
    let foreign: HashSet<_> =
        alternate_blocks! { storage cq_storage; work (&work); rows (&right) }.collect();
    let rejected = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        alternate_blocks! { storage cq_storage; select (&left) (foreign) }
    }));
    let panic = rejected.err().expect("foreign identity must be rejected");
    let message = panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap();
    assert!(message.contains("selected row belongs to another block store"));
}
