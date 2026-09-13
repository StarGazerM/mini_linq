//! Independent representations, algorithm choices, and selected block access.
use cq_storage::BlockStore;
use std::collections::{BTreeSet, HashSet};
use std::num::NonZeroUsize;

fn size(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).unwrap()
}

// A caller-owned macro can participate without being a compiler-known variant.
macro_rules! caller_sorted {
    ($($request:tt)*) => { ::mini_linq::sorted_rows! { $($request)* } };
}
pub(crate) use caller_sorted;

macro_rules! mixed_exact {
    ($($plan:tt)*) => {
        ::mini_linq::physical_plan! {
            storage ::cq_storage;
            access ::mini_linq::collect_access;
            materialize ::mini_linq::hash_rows;
            inputs {
                R => ::mini_linq::block_rows;
                S => ::mini_linq::hash_rows;
                T => ::mini_linq::sorted_rows;
            }
            results {}
            semijoin ::mini_linq::exact_semijoin;
            semijoins {}
            keys ::std::collections::HashSet;
            key_sets {}
            join ::mini_linq::hash_join;
            joins {}
            $($plan)*
        }
    };
}

macro_rules! mixed_zone {
    ($($plan:tt)*) => {
        ::mini_linq::physical_plan! {
            storage ::cq_storage;
            access ::mini_linq::collect_access;
            materialize ::mini_linq::hash_rows;
            inputs {
                R => ::mini_linq::block_rows;
                S => ::mini_linq::hash_rows;
                T => ::mini_linq::sorted_rows;
            }
            results {}
            semijoin ::mini_linq::exact_semijoin;
            semijoins { r1 => ::mini_linq::zone_semijoin; }
            keys ::std::collections::HashSet;
            key_sets { r1 => ::std::collections::BTreeSet; }
            join ::mini_linq::hash_join;
            joins {}
            $($plan)*
        }
    };
}

macro_rules! mixed_nested {
    ($($plan:tt)*) => {
        ::mini_linq::physical_plan! {
            storage ::cq_storage;
            access ::mini_linq::collect_access;
            materialize ::mini_linq::hash_rows;
            inputs {
                R => ::mini_linq::block_rows;
                S => ::mini_linq::hash_rows;
                T => ::mini_linq::sorted_rows;
            }
            results { live => ::mini_linq::sorted_rows; }
            semijoin ::mini_linq::exact_semijoin;
            semijoins { r1 => ::mini_linq::zone_semijoin; }
            keys ::std::collections::HashSet;
            key_sets { r1 => ::std::collections::BTreeSet; }
            join ::mini_linq::hash_join;
            joins {
                rs => ::mini_linq::nested_loop_join;
                joined => ::mini_linq::nested_loop_join;
            }
            $($plan)*
        }
    };
}

macro_rules! sweeps {
    ($entry:ident, $name:ident) => {
        $entry! {
            struct $name;
            relation R(k:i64,x:i64);
            relation S(z:i64,k:i64);
            relation T(y:i64,z:i64);
            answer(x,y) :- R(k,x), S(z,k), T(y,z);
            relational {
                r = rename R {k -> k,x -> x};
                s = rename S {z -> z,k -> k};
                t = rename T {y -> y,z -> z};
                s1 = semijoin s with t;
                r1 = semijoin r with s1;
                s2 = semijoin s1 with r1;
                t1 = semijoin t with s2;
                rs = natural_join r1 with s2;
                live = project rs keep {x,z};
                joined = natural_join live with t1;
                result = project joined keep {x,y};
                output result as answer(x,y);
            }
        }
    };
}

sweeps!(mixed_exact, MixedExact);
sweeps!(mixed_zone, MixedZone);
sweeps!(mixed_nested, MixedNested);

fn oracle(r: &[[i64; 2]], s: &[[i64; 2]], t: &[[i64; 2]]) -> Vec<(i64, i64)> {
    let mut answer = BTreeSet::new();
    for &[k, x] in r {
        for &[z, sk] in s {
            for &[y, tz] in t {
                if k == sk && z == tz {
                    answer.insert((x, y));
                }
            }
        }
    }
    answer.into_iter().collect()
}

#[test]
fn mixed_representations_and_join_recipes_match_an_independent_oracle() {
    let rows = |mask: u8| {
        (0..4)
            .filter(|bit| mask & (1 << bit) != 0)
            .map(|bit| [i64::from(bit / 2), i64::from(bit % 2)])
            .collect::<Vec<_>>()
    };
    for r in 0..16 {
        for s in 0..16 {
            for t in 0..16 {
                let (r, s, t) = (rows(r), rows(s), rows(t));
                let expected = oracle(&r, &s, &t);
                let r = BlockStore::from_rows(r, size(2));
                let s: HashSet<_> = s.into_iter().collect();
                let t: BTreeSet<_> = t.into_iter().collect();
                assert_eq!(MixedExact::run(&r, &s, &t, size(2)), expected);
                assert_eq!(MixedZone::run(&r, &s, &t, size(2)), expected);
                assert_eq!(MixedNested::run(&r, &s, &t, size(2)), expected);
            }
        }
    }
}

#[test]
fn only_the_chosen_operand_uses_block_access() {
    let r = BlockStore::from_rows(
        [[1, 10], [1, 11], [50, 20], [60, 21], [2, 30], [2, 31]],
        size(2),
    );
    let s = HashSet::from([[100, 1], [200, 2], [900, 90]]);
    let t = BTreeSet::from([[7, 100], [8, 200]]);
    let (answer, zone) = MixedZone::run_with_stats(&r, &s, &t, size(2));
    let (exact_answer, exact) = MixedExact::run_with_stats(&r, &s, &t, size(2));
    let (nested_answer, nested) = MixedNested::run_with_stats(&r, &s, &t, size(2));
    assert_eq!(answer, vec![(10, 7), (11, 7), (30, 8), (31, 8)]);
    assert_eq!(answer, exact_answer);
    assert_eq!(answer, nested_answer);

    let step = |name| zone.steps.iter().find(|(n, _, _)| *n == name).unwrap();
    assert_eq!(step("r1").2.blocks_skipped, 1);
    assert_eq!(step("r1").2.index_rows, 2); // Current s1 is native and pruned.
    assert_eq!(step("r1").2.rows_visited, 6); // Two keys and four retained rows.
    for name in ["s1", "t1", "live", "joined", "result"] {
        let work = step(name).2;
        assert_eq!(work.blocks_considered, 0, "{name}");
        assert_eq!(work.blocks_opened, 0, "{name}");
    }
    // The default hash producer makes the final result independent of blocks.
    assert_eq!(zone.output.blocks_opened, 0);
    assert!(
        zone.steps.iter().map(|s| s.2.rows_visited).sum::<usize>()
            < exact.steps.iter().map(|s| s.2.rows_visited).sum()
    );
    let indexed_join = exact.steps.iter().find(|s| s.0 == "rs").unwrap();
    let loop_join = nested.steps.iter().find(|s| s.0 == "rs").unwrap();
    assert!(indexed_join.2.index_rows > 0);
    assert_eq!(loop_join.2.index_rows, 0);
}

mini_linq::physical_plan! {
    storage ::cq_storage;
    access ::mini_linq::collect_access;
    materialize ::mini_linq::hash_rows;
    inputs {
        R => ::mini_linq::block_rows;
        S => ::mini_linq::hash_rows;
        T => ::mini_linq::sorted_rows;
    }
    results {}
    semijoin ::mini_linq::exact_semijoin;
    semijoins {
        r1 => ::mini_linq::zone_semijoin;
        r2 => ::mini_linq::zone_semijoin;
        r3 => ::mini_linq::zone_semijoin;
    }
    keys ::std::collections::HashSet;
    key_sets {}
    join ::mini_linq::hash_join;
    joins {}
    struct Versioned;
    relation R(k:i64,x:i64);
    relation S(z:i64,k:i64);
    relation T(z:i64);
    answer(x) :- R(k,x), S(z,k), T(z);
    relational {
        r = rename R {k -> k,x -> x};
        s = rename S {z -> z,k -> k};
        t = rename T {z -> z};
        s1 = semijoin s with t;
        r1 = semijoin r with s;
        r2 = semijoin r1 with s1;
        r3 = semijoin r2 with s;
        result = project r3 keep {x};
        output result as answer(x);
    }
}

#[test]
fn later_reductions_use_current_keys_and_cannot_resurrect_selected_rows() {
    let r = BlockStore::from_rows([[1, 10], [2, 20], [3, 30], [50, 40]], size(4));
    let s = HashSet::from([[100, 1], [200, 2], [300, 3]]);
    let t = BTreeSet::from([[100]]);
    let (answer, stats) = Versioned::run_with_stats(&r, &s, &t, size(2));
    assert_eq!(answer, vec![(10,)]);
    let step = |name| stats.steps.iter().find(|(n, _, _)| *n == name).unwrap();
    assert_eq!(step("r1").1, 3);
    assert_eq!(step("r2").1, 1);
    assert_eq!(step("r2").2.index_rows, 1); // Rebuild for narrowed s1, not base s.
    assert_eq!(step("r3").1, 1);
    assert_eq!(step("r3").2.exact_probes, 1);
    // Reopening the same physical block must enumerate only r2's selection.
    assert_eq!(step("r3").2.rows_visited, 1);
}

mini_linq::physical_plan! {
    storage ::cq_storage;
    access ::mini_linq::collect_access;
    materialize ::mini_linq::hash_rows;
    inputs { R => ::mini_linq::sorted_rows; S => ::mini_linq::hash_rows; }
    results { packed => ::mini_linq::block_rows; }
    semijoin ::mini_linq::exact_semijoin;
    semijoins { reduced => ::mini_linq::zone_semijoin; }
    keys ::std::collections::HashSet;
    key_sets { reduced => ::std::collections::BTreeSet; }
    join ::mini_linq::hash_join;
    joins {}
    struct ExplicitPacking;
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
fn only_explicit_materialization_constructs_block_storage() {
    let r = BTreeSet::from([[1, 10], [2, 20], [50, 30], [60, 40]]);
    let s = HashSet::from([[1]]);
    let (answer, stats) = ExplicitPacking::run_with_stats(&r, &s, size(1));
    assert_eq!(answer, vec![(10,)]);
    let reduced = stats.steps.iter().find(|s| s.0 == "reduced").unwrap();
    assert_eq!(reduced.2.blocks_skipped, 3);
    assert_eq!(reduced.2.blocks_opened, 1);
    assert_eq!(stats.output.blocks_opened, 0);
}

macro_rules! native {
    ($($plan:tt)*) => {
        ::mini_linq::physical_plan! {
            storage ::cq_storage;
            access ::mini_linq::collect_access;
            materialize ::mini_linq::hash_rows;
            inputs {
                R => ::mini_linq::hash_rows;
                S => ::mini_linq::hash_rows;
                T => crate::caller_sorted;
            }
            results { live => crate::caller_sorted; }
            semijoin ::mini_linq::exact_semijoin;
            semijoins {}
            keys ::std::collections::BTreeSet;
            key_sets { r1 => ::std::collections::HashSet; }
            join ::mini_linq::nested_loop_join;
            joins { rs => ::mini_linq::hash_join; }
            $($plan)*
        }
    };
}
sweeps!(native, Native);

#[test]
fn native_inputs_and_results_have_no_block_layout_dependency() {
    let r = HashSet::from([[1, 10], [2, 20]]);
    let s = HashSet::from([[100, 1], [200, 2]]);
    let t = BTreeSet::from([[7, 100]]);
    for block_rows in [size(1), size(100)] {
        let (answer, stats) = Native::run_with_stats(&r, &s, &t, block_rows);
        assert_eq!(answer, vec![(10, 7)]);
        for (_, _, work) in &stats.steps {
            assert_eq!(work.blocks_considered, 0);
            assert_eq!(work.blocks_opened, 0);
            assert_eq!(work.blocks_skipped, 0);
        }
        assert_eq!(stats.output.blocks_opened, 0);
    }
}

mini_linq::physical_plan! {
    storage ::cq_storage;
    access ::mini_linq::collect_access;
    materialize ::mini_linq::sorted_rows;
    inputs { R => ::mini_linq::block_rows; S => ::mini_linq::hash_rows; }
    results {}
    semijoin ::mini_linq::zone_semijoin;
    semijoins {}
    keys ::std::collections::HashSet;
    key_sets {}
    join ::mini_linq::hash_join;
    joins {}
    struct Composite;
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

#[test]
fn summary_false_positives_still_require_exact_composite_keys() {
    let r = BlockStore::from_rows([[1, 1, 10], [1, 2, 20], [2, 1, 30], [2, 2, 40]], size(4));
    let s = HashSet::from([[1, 2], [2, 1]]);
    let (answer, stats) = Composite::run_with_stats(&r, &s, size(2));
    assert_eq!(answer, vec![(20,), (30,)]);
    let reduced = stats.steps.iter().find(|s| s.0 == "reduced").unwrap();
    assert_eq!(reduced.2.blocks_skipped, 0);
    assert_eq!(reduced.2.exact_probes, 4);
    assert!(Composite::run(&r, &HashSet::new(), size(2)).is_empty());
}

mini_linq::physical_plan! {
    storage ::cq_storage;
    access ::mini_linq::collect_access;
    materialize ::mini_linq::hash_rows;
    inputs { R => ::mini_linq::hash_rows; S => crate::caller_sorted; }
    results { u => crate::caller_sorted; exists => crate::caller_sorted; }
    semijoin ::mini_linq::exact_semijoin;
    semijoins {}
    keys ::std::collections::BTreeSet;
    key_sets {}
    join ::mini_linq::hash_join;
    joins { ur => ::mini_linq::nested_loop_join; }
    struct NativeUnit;
    relation R(a:i64);
    relation S(b:i64);
    answer(x) :- R(x), S(y);
    relational {
        u = unit;
        r = rename r#R {r#a -> x};
        s = rename S {b -> y};
        ur = natural_join u with r;
        exists = project s keep {};
        reduced = semijoin ur with exists;
        output reduced as answer(x);
    }
}

#[test]
fn zero_column_resources_and_empty_separator_use_native_sets() {
    for left in [HashSet::new(), HashSet::from([[1], [2]])] {
        for right in [BTreeSet::new(), BTreeSet::from([[7]])] {
            let expected: Vec<_> = if right.is_empty() {
                vec![]
            } else {
                left.iter()
                    .map(|row| (row[0],))
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect()
            };
            assert_eq!(NativeUnit::run(&left, &right, size(2)), expected);
        }
    }
}

// Unary S is already an exact full-key set. This caller-owned recipe borrows
// that current binding directly, while retaining the supplied reduction code.
macro_rules! borrow_keys {
    (
        storage $storage:path;
        access $access:path;
        keys $keys:path;
        work ($work:expr);
        $result:ident = project s keep {k}; {
            s(k) => $right_provider:path { $right:expr };
        }
    ) => {
        ($right,)
    };
    (
        storage $storage:path;
        access $access:path;
        keys $keys:path;
        $result:ident = project s keep {k}; {
            s(k) => $right_provider:path { $right:expr };
        }
    ) => {
        ($right,)
    };
    ($($request:tt)*) => {
        ::mini_linq::exact_semijoin! { $($request)* }
    };
}
pub(crate) use borrow_keys;

mini_linq::physical_plan! {
    storage ::cq_storage;
    access ::mini_linq::collect_access;
    materialize ::mini_linq::hash_rows;
    inputs { R => ::mini_linq::hash_rows; S => ::mini_linq::hash_rows; }
    semijoin crate::borrow_keys;
    keys ::std::collections::HashSet;
    join ::mini_linq::hash_join;
    struct BorrowedKeys;
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

#[test]
fn caller_recipe_can_borrow_existing_keys_without_scanning_or_constructing_them() {
    let r = HashSet::from([[1, 10], [2, 20], [3, 30]]);
    let mut s = HashSet::from([[1], [9]]);
    let (answer, stats) = BorrowedKeys::run_with_stats(&r, &s, size(2));
    assert_eq!(answer, vec![(10,)]);
    let reduced = stats.steps.iter().find(|s| s.0 == "reduced").unwrap();
    assert_eq!(reduced.2.index_rows, 0);
    assert_eq!(reduced.2.rows_visited, r.len());
    assert_eq!(reduced.2.exact_probes, r.len());

    s.remove(&[1]);
    s.insert([3]);
    let (answer, stats) = BorrowedKeys::run_with_stats(&r, &s, size(2));
    assert_eq!(answer, vec![(30,)]);
    let reduced = stats.steps.iter().find(|s| s.0 == "reduced").unwrap();
    assert_eq!(reduced.2.index_rows, 0);
    assert_eq!(reduced.2.rows_visited, r.len());
}
