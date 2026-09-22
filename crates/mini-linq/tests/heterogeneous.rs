//! Independent representations, algorithm choices, and selected block access.
use cq_storage::BlockStore;
use std::collections::{BTreeSet, HashSet};
use std::num::NonZeroUsize;

fn size(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).unwrap()
}

// Each binding owns its complete operation protocol; the physical host forwards
// only semantic operands and, for observed execution, its explicit work argument.
macro_rules! bound_block {
    ($($op:tt)*) => { ::mini_linq::array_block_rows! { storage ::cq_storage; packing (crate::size(1)); $($op)* } };
}
macro_rules! bound_scan {
    (work ($work:expr); $($op:tt)*) => { ::mini_linq::scan_candidates! { $($op)* } };
    ($($op:tt)*) => { ::mini_linq::scan_candidates! { $($op)* } };
}
macro_rules! bound_prune {
    ($($op:tt)*) => { ::mini_linq::prune_block_candidates! { access_iter ::mini_linq::access_to_support_iter; $($op)* } };
}
macro_rules! bound_exact {
    ($($op:tt)*) => { ::mini_linq::exact_keys! { storage ::cq_storage; access_iter ::mini_linq::access_to_support_iter; keys ::std::collections::HashSet; $($op)* } };
}
macro_rules! bound_zone {
    ($($op:tt)*) => { ::mini_linq::summarized_exact_keys! { storage ::cq_storage; access_iter ::mini_linq::access_to_support_iter; keys ::std::collections::HashSet; $($op)* } };
}
macro_rules! zone_sorted {
    ($($op:tt)*) => { ::mini_linq::summarized_exact_keys! { storage ::cq_storage; access_iter ::mini_linq::access_to_support_iter; keys ::std::collections::BTreeSet; $($op)* } };
}
macro_rules! bound_hash {
    ($($op:tt)*) => { ::mini_linq::hash_pairs! { $($op)* } };
}
macro_rules! bound_nested {
    ($($op:tt)*) => { ::mini_linq::nested_pairs! { $($op)* } };
}
pub(crate) use {
    bound_block, bound_exact, bound_hash, bound_nested, bound_prune, bound_scan, bound_zone,
    zone_sorted,
};

// A caller-owned macro can participate without being a compiler-known variant.
macro_rules! caller_sorted {
    ($($request:tt)*) => { ::mini_linq::array_sorted_rows! { $($request)* } };
}
pub(crate) use caller_sorted;

macro_rules! mixed_exact {
    ($($plan:tt)*) => {
        ::mini_linq::observed_configured_program! {
            counter_storage ::cq_storage;
            access ::mini_linq::access_to_support_iter;

            result_rows ::mini_linq::array_hash_rows;
            inputs {
                R => crate::bound_block;
                S => ::mini_linq::array_hash_rows;
                T => ::mini_linq::array_sorted_rows;
            }
            result_bindings {}
            candidates crate::bound_scan;
            candidate_bindings {  }
            keys crate::bound_exact;
            key_sets {  }

            pairs crate::bound_hash;
            pair_bindings {}
            $($plan)*
        }
    };
}

macro_rules! mixed_zone {
    ($($plan:tt)*) => {
        ::mini_linq::observed_configured_program! {
            counter_storage ::cq_storage;
            access ::mini_linq::access_to_support_iter;

            result_rows ::mini_linq::array_hash_rows;
            inputs {
                R => crate::bound_block;
                S => ::mini_linq::array_hash_rows;
                T => ::mini_linq::array_sorted_rows;
            }
            result_bindings {}
            candidates crate::bound_scan;
            candidate_bindings { r1 => crate::bound_prune; }
            keys crate::bound_exact;
            key_sets { r1 => crate::zone_sorted; }

            pairs crate::bound_hash;
            pair_bindings {}
            $($plan)*
        }
    };
}

macro_rules! mixed_nested {
    ($($plan:tt)*) => {
        ::mini_linq::observed_configured_program! {
            counter_storage ::cq_storage;
            access ::mini_linq::access_to_support_iter;

            result_rows ::mini_linq::array_hash_rows;
            inputs {
                R => crate::bound_block;
                S => ::mini_linq::array_hash_rows;
                T => ::mini_linq::array_sorted_rows;
            }
            result_bindings { live => ::mini_linq::array_sorted_rows; }
            candidates crate::bound_scan;
            candidate_bindings { r1 => crate::bound_prune; }
            keys crate::bound_exact;
            key_sets { r1 => crate::zone_sorted; }

            pairs crate::bound_hash;
            pair_bindings {
                rs => crate::bound_nested;
                joined => crate::bound_nested;
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
                assert_eq!(MixedExact::run(&r, &s, &t).0, expected);
                assert_eq!(MixedZone::run(&r, &s, &t).0, expected);
                assert_eq!(MixedNested::run(&r, &s, &t).0, expected);
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
    let (answer, zone) = MixedZone::run(&r, &s, &t);
    let (exact_answer, exact) = MixedExact::run(&r, &s, &t);
    let (nested_answer, nested) = MixedNested::run(&r, &s, &t);
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

mini_linq::observed_configured_program! {
    counter_storage ::cq_storage;
    access ::mini_linq::access_to_support_iter;

    result_rows ::mini_linq::array_hash_rows;
    inputs {
        R => crate::bound_block;
        S => ::mini_linq::array_hash_rows;
        T => ::mini_linq::array_sorted_rows;
    }
    result_bindings {}
    candidates crate::bound_scan;
    candidate_bindings { r1 => crate::bound_prune; r2 => crate::bound_prune; r3 => crate::bound_prune; }
    keys crate::bound_exact;
    key_sets { r1 => crate::bound_zone; r2 => crate::bound_zone; r3 => crate::bound_zone; }

    pairs crate::bound_hash;
    pair_bindings {}
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
    let (answer, stats) = Versioned::run(&r, &s, &t);
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

mini_linq::observed_configured_program! {
    counter_storage ::cq_storage;
    access ::mini_linq::access_to_support_iter;

    result_rows ::mini_linq::array_hash_rows;
    inputs { R => ::mini_linq::array_sorted_rows; S => ::mini_linq::array_hash_rows; }
    result_bindings { packed => crate::bound_block; }
    candidates crate::bound_scan;
    candidate_bindings { reduced => crate::bound_prune; }
    keys crate::bound_exact;
    key_sets { reduced => crate::bound_zone; }

    pairs crate::bound_hash;
    pair_bindings {}
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
    let (answer, stats) = ExplicitPacking::run(&r, &s);
    assert_eq!(answer, vec![(10,)]);
    let reduced = stats.steps.iter().find(|s| s.0 == "reduced").unwrap();
    assert_eq!(reduced.2.blocks_skipped, 3);
    assert_eq!(reduced.2.blocks_opened, 1);
    assert_eq!(stats.output.blocks_opened, 0);
}

macro_rules! native {
    ($($plan:tt)*) => {
        ::mini_linq::observed_configured_program! {
            counter_storage ::cq_storage;
            access ::mini_linq::access_to_support_iter;

            result_rows ::mini_linq::array_hash_rows;
            inputs {
                R => ::mini_linq::array_hash_rows;
                S => ::mini_linq::array_hash_rows;
                T => crate::caller_sorted;
            }
            result_bindings { live => crate::caller_sorted; }
            candidates crate::bound_scan;
            candidate_bindings {  }
            keys crate::bound_exact;
            key_sets {  }

            pairs crate::bound_nested;
            pair_bindings { rs => crate::bound_hash; }
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
    let (answer, stats) = Native::run(&r, &s, &t);
    assert_eq!(answer, vec![(10, 7)]);
    for (_, _, work) in &stats.steps {
        assert_eq!(work.blocks_considered, 0);
        assert_eq!(work.blocks_opened, 0);
        assert_eq!(work.blocks_skipped, 0);
    }
    assert_eq!(stats.output.blocks_opened, 0);
}

mini_linq::observed_configured_program! {
    counter_storage ::cq_storage;
    access ::mini_linq::access_to_support_iter;

    result_rows ::mini_linq::array_sorted_rows;
    inputs { R => crate::bound_block; S => ::mini_linq::array_hash_rows; }
    result_bindings {}
    candidates crate::bound_prune;
    candidate_bindings {  }
    keys crate::bound_zone;
    key_sets {  }

    pairs crate::bound_hash;
    pair_bindings {}
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
    let (answer, stats) = Composite::run(&r, &s);
    assert_eq!(answer, vec![(20,), (30,)]);
    let reduced = stats.steps.iter().find(|s| s.0 == "reduced").unwrap();
    assert_eq!(reduced.2.blocks_skipped, 0);
    assert_eq!(reduced.2.exact_probes, 4);
    assert!(Composite::run(&r, &HashSet::new()).0.is_empty());
}

mini_linq::observed_configured_program! {
    counter_storage ::cq_storage;
    access ::mini_linq::access_to_support_iter;

    result_rows ::mini_linq::array_hash_rows;
    inputs { R => ::mini_linq::array_hash_rows; S => crate::caller_sorted; }
    result_bindings { u => crate::caller_sorted; exists => crate::caller_sorted; }
    candidates crate::bound_scan;
    candidate_bindings {  }
    keys crate::bound_exact;
    key_sets {  }

    pairs crate::bound_hash;
    pair_bindings { ur => crate::bound_nested; }
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
            assert_eq!(NativeUnit::run(&left, &right).0, expected);
        }
    }
}

// For this query, unary S is already the exact projected key set. The caller
// discharges that layout law and replaces Build of the ordinary Access region
// with its source binding; all resource operations retain the exact provider.
macro_rules! borrow_keys {
    ($(work ($work:expr);)? key ($($field:expr),* $(,)?)) => { [$(*($field)),*] };
    (
        $(work ($work:expr);)?
        build {
            $out:ident($row:ident) => ($projection:expr) :-
            Rows($input:ident) => for ($pattern:ident,) in (
                $provider:ident ! { scan ($right:expr) }.map($map:expr)
            );
        }
    ) => { $right };
    ($($request:tt)*) => { crate::bound_exact! { $($request)* } };
}
pub(crate) use borrow_keys;

mini_linq::observed_configured_program! {
    counter_storage ::cq_storage;
    access ::mini_linq::access_to_support_iter;

    result_rows ::mini_linq::array_hash_rows;
    inputs { R => ::mini_linq::array_hash_rows; S => ::mini_linq::array_hash_rows; }
    candidates crate::bound_scan;
    candidate_bindings {  }
    keys crate::borrow_keys;
    key_sets {  }

    pairs crate::bound_hash;
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
    let (answer, stats) = BorrowedKeys::run(&r, &s);
    assert_eq!(answer, vec![(10,)]);
    let reduced = stats.steps.iter().find(|s| s.0 == "reduced").unwrap();
    assert_eq!(reduced.2.index_rows, 0);
    assert_eq!(reduced.2.rows_visited, r.len());
    assert_eq!(reduced.2.exact_probes, r.len());

    s.remove(&[1]);
    s.insert([3]);
    let (answer, stats) = BorrowedKeys::run(&r, &s);
    assert_eq!(answer, vec![(30,)]);
    let reduced = stats.steps.iter().find(|s| s.0 == "reduced").unwrap();
    assert_eq!(reduced.2.index_rows, 0);
    assert_eq!(reduced.2.rows_visited, r.len());
}

mini_linq::observed_configured_program! {
    counter_storage ::cq_storage;
    access ::mini_linq::access_to_support_iter;

    inputs {
        R => crate::bound_block;
        S => ::mini_linq::array_hash_rows;
        T => ::mini_linq::array_hash_rows;
    }
    result_rows ::mini_linq::array_hash_rows;
    candidates crate::bound_scan;
    candidate_bindings { r1 => crate::bound_prune; }
    keys crate::bound_zone;

    pairs crate::bound_hash;
    struct SharedAcrossAccess;
    relation R(k:i64,x:i64);
    relation S(k:i64,y:i64);
    relation T(k:i64);
    answer(x,y) :- R(k,x), S(k,y), T(k);
    relational {
        r = rename R {k -> k,x -> x};
        s = rename S {k -> k,y -> y};
        t = rename T {k -> k};
        r1 = semijoin r with t;
        s1 = semijoin s with t;
        joined = natural_join r1 with s1;
        result = project joined keep {x,y};
        output result as answer(x,y);
    }
}

#[test]
fn one_key_resource_serves_independent_native_and_pruned_accesses() {
    let r = BlockStore::from_rows([[1, 10], [90, 20]], size(1));
    let s = HashSet::from([[1, 30], [90, 40]]);
    let t = HashSet::from([[1]]);
    let (answer, stats) = SharedAcrossAccess::run(&r, &s, &t);
    assert_eq!(answer, vec![(10, 30)]);
    let step = |name| stats.steps.iter().find(|step| step.0 == name).unwrap().2;
    assert_eq!(step("r1").index_rows, 1);
    assert_eq!(step("s1").index_rows, 0);
    assert_eq!(step("r1").blocks_skipped, 1);
    assert_eq!(step("s1").blocks_considered, 0);
    assert_eq!(step("s1").exact_probes, 2);
}

thread_local! {
    static ROW_DECODES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
fn record_row_decode() {
    ROW_DECODES.with(|count| count.set(count.get() + 1));
}
fn take_row_decodes() -> usize {
    ROW_DECODES.with(|count| count.replace(0))
}
macro_rules! counted_block_rows {
    (work ($work:expr); row ($handle:expr)) => { crate::counted_block_rows! { row ($handle) } };
    (row ($handle:expr)) => {{
        crate::record_row_decode();
        ::mini_linq::array_block_rows! { storage ::cq_storage; row ($handle) }
    }};
    ($($operation:tt)*) => { crate::bound_block! { $($operation)* } };
}
pub(crate) use counted_block_rows;

macro_rules! decode_query {
    ($name:ident, $scan:path, $keys:path; $($query:tt)*) => {
        ::mini_linq::observed_configured_program! {
            counter_storage ::cq_storage;
            access ::mini_linq::access_to_support_iter;

            inputs { R => crate::counted_block_rows; S => ::mini_linq::array_hash_rows; }
            result_rows ::mini_linq::array_hash_rows;
            candidates $scan;
            keys $keys;

            pairs crate::bound_hash;
            struct $name;
            $($query)*
        }
    };
}
macro_rules! composite_decode_query {
    ($name:ident, $scan:path, $keys:path) => {
        decode_query! {
            $name, $scan, $keys;
            relation R(a:i64,b:i64,x:i64);
            relation S(b:i64,a:i64);
            answer(a,b,x) :- R(a,b,x), S(b,a);
            relational {
                r = rename R {a -> a,b -> b,x -> x};
                s = rename S {b -> b,a -> a};
                reduced = semijoin r with s;
                output reduced as answer(a,b,x);
            }
        }
    };
}
macro_rules! empty_decode_query {
    ($name:ident, $scan:path, $keys:path) => {
        decode_query! {
            $name, $scan, $keys;
            relation R(a:i64,b:i64,x:i64);
            relation S(y:i64);
            answer(a,b,x) :- R(a,b,x), S(y);
            relational {
                r = rename R {a -> a,b -> b,x -> x};
                s = rename S {y -> y};
                reduced = semijoin r with s;
                output reduced as answer(a,b,x);
            }
        }
    };
}
composite_decode_query!(CompositeDecode, crate::bound_scan, crate::bound_exact);
composite_decode_query!(PrunedCompositeDecode, crate::bound_prune, crate::bound_zone);
empty_decode_query!(EmptyDecode, crate::bound_scan, crate::bound_exact);
empty_decode_query!(PrunedEmptyDecode, crate::bound_prune, crate::bound_zone);

#[test]
fn exact_keys_decode_each_candidate_once_and_empty_keys_need_no_decode() {
    let r = BlockStore::from_rows([[1, 2, 10], [1, 3, 20], [4, 2, 30]], size(1));
    let s = HashSet::from([[2, 1]]);
    macro_rules! check_composite {
        ($query:ident, $count:expr) => {
            take_row_decodes();
            assert_eq!($query::run(&r, &s).0, vec![(1, 2, 10)]);
            assert_eq!(take_row_decodes(), $count);
            let (answer, stats) = $query::run(&r, &s);
            assert_eq!(answer, vec![(1, 2, 10)]);
            assert_eq!(take_row_decodes(), $count);
            let work = stats
                .steps
                .iter()
                .find(|step| step.0 == "reduced")
                .unwrap()
                .2;
            assert_eq!(work.exact_probes, $count);
        };
    }
    check_composite!(CompositeDecode, 3);
    check_composite!(PrunedCompositeDecode, 1);

    for s in [HashSet::new(), HashSet::from([[7]])] {
        let expected = if s.is_empty() {
            vec![]
        } else {
            vec![(1, 2, 10), (1, 3, 20), (4, 2, 30)]
        };
        macro_rules! check_empty {
            ($query:ident) => {
                assert_eq!($query::run(&r, &s).0, expected);
                assert_eq!(take_row_decodes(), 0);
                assert_eq!($query::run(&r, &s).0, expected);
                assert_eq!(take_row_decodes(), 0);
            };
        }
        check_empty!(EmptyDecode);
        check_empty!(PrunedEmptyDecode);
    }
}

#[test]
fn callable_packing_captures_configuration_and_returns_owned_answers() {
    use std::cell::RefCell;
    let events = RefCell::new(Vec::new());
    let answer = {
        let rows = HashSet::from([[1, 10], [2, 20]]);
        let packing = size(2);
        let observed = &events;
        macro_rules! source_rows {
            (scan ($rows:expr)) => {{
                observed.borrow_mut().push("rows");
                ::mini_linq::array_hash_rows! { scan ($rows) }
            }};
            ($($op:tt)*) => { ::mini_linq::array_hash_rows! { $($op)* } };
        }
        macro_rules! packed {
            ($($op:tt)*) => { ::mini_linq::array_block_rows! {
                storage ::cq_storage;
                packing ({ observed.borrow_mut().push("packing"); packing });
                $($op)*
            }};
        }
        let query = ::mini_linq::configured_callable! {
            access ::mini_linq::access_to_support_iter;
            inputs { R => source_rows; }
            result_rows packed;
            candidates crate::bound_scan;
            keys crate::bound_exact;
            pairs crate::bound_hash;
            struct Packed;
            relation R(k:i64,x:i64);
            answer(x) :- R(k,x);
            relational {
                r = rename R {k -> k,x -> x};
                result = project r keep {x};
                output result as answer(x);
            }
        };
        assert_eq!(query(&rows), vec![(10,), (20,)]);
        query(&rows)
    };
    assert_eq!(answer, vec![(10,), (20,)]);
    assert_eq!(*events.borrow(), vec!["rows", "packing", "rows", "packing"]);
}

#[test]
fn observed_callable_has_its_own_counter_scope_on_every_call() {
    let __ml_work = size(1);
    macro_rules! packed {
        ($($op:tt)*) => { ::mini_linq::array_block_rows! {
            storage ::cq_storage; packing (__ml_work); $($op)*
        }};
    }
    let query = ::mini_linq::observed_configured_callable! {
        counter_storage ::cq_storage;
        access ::mini_linq::access_to_support_iter;
        inputs { R => ::mini_linq::array_hash_rows; }
        result_rows packed;
        candidates crate::bound_scan;
        keys crate::bound_exact;
        pairs crate::bound_hash;
        struct Packed;
        relation R(k:i64,x:i64);
        answer(x) :- R(k,x);
        relational {
            r = rename R {k -> k,x -> x};
            result = project r keep {x};
            output result as answer(x);
        }
    };
    let (answer, first) = {
        let rows = HashSet::from([[1, 10], [2, 20]]);
        query(&rows)
    };
    let (again, second) = query(&HashSet::from([[1, 10], [2, 20]]));
    assert_eq!(answer, vec![(10,), (20,)]);
    assert_eq!(again, answer);
    assert_eq!(first.steps, second.steps);
    assert_eq!(first.output, second.output);
    assert_eq!(first.output.rows_visited, 2);
}

#[allow(non_snake_case)]
mod __MlStatistics {
    pub use cq_storage::*;
    pub use mini_linq::{access_to_support_iter, array_hash_rows};

    pub fn packing() -> std::num::NonZeroUsize {
        std::num::NonZeroUsize::new(1).unwrap()
    }
}

#[test]
fn callable_statistics_helper_cannot_shadow_configured_paths_or_opaque_bindings() {
    let direct = ::mini_linq::observed_configured_callable! {
        counter_storage __MlStatistics;
        access __MlStatistics::access_to_support_iter;
        inputs { R => __MlStatistics::array_hash_rows; }
        result_rows __MlStatistics::array_hash_rows;
        candidates crate::bound_scan;
        keys crate::bound_exact;
        pairs crate::bound_hash;
        struct Q;
        relation R(x:i64); answer(x) :- R(x);
        relational {
            r = rename R {x -> x};
            result = project r keep {x};
            output result as answer(x);
        }
    };
    let rows = HashSet::from([[1]]);
    let (answer, stats) = direct(&rows);
    assert_eq!(answer, vec![(1,)]);
    assert_eq!(stats.steps.len(), 2);

    macro_rules! opaque_rows {
        ($($op:tt)*) => { __MlStatistics::array_hash_rows! { $($op)* } };
    }
    macro_rules! opaque_access {
        ($($op:tt)*) => { __MlStatistics::access_to_support_iter! { $($op)* } };
    }
    macro_rules! opaque_packing {
        ($($op:tt)*) => { ::mini_linq::array_block_rows! {
            storage __MlStatistics;
            packing (__MlStatistics::packing());
            $($op)*
        }};
    }
    let opaque = ::mini_linq::observed_configured_callable! {
        counter_storage ::cq_storage;
        access opaque_access;
        inputs { R => opaque_rows; }
        result_rows opaque_packing;
        candidates crate::bound_scan;
        keys crate::bound_exact;
        pairs crate::bound_hash;
        struct Q;
        relation R(x:i64); answer(x) :- R(x);
        relational {
            r = rename R {x -> x};
            result = project r keep {x};
            output result as answer(x);
        }
    };
    let (answer, stats) = opaque(&rows);
    assert_eq!(answer, vec![(1,)]);
    assert_eq!(stats.steps.len(), 2);
    assert_eq!(stats.output.rows_visited, 1);
}
