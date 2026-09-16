//! Compiled routes, exact answers, and evidence of actual masked block access.
use cq_storage as storage;
use std::collections::BTreeSet;
use std::num::NonZeroUsize;
use storage::BlockStore;

#[path = "support/sorted_keys.rs"]
mod sorted_keys;

mod sorted_storage {
    pub use cq_storage::*;
    pub type ZoneSummary<const K: usize> = crate::observed_summary::ZoneSummary<K>;
}

// A facade may instrument summary primitives without owning row traversal.
mod observed_summary {
    use std::cell::Cell;
    thread_local! {
        static GUARDS: Cell<usize> = const { Cell::new(0) };
    }
    pub fn take_guards() -> usize {
        GUARDS.with(|guards| guards.replace(0))
    }
    #[derive(Default)]
    pub struct ZoneSummary<const K: usize>(cq_storage::ZoneSummary<K>);
    impl<const K: usize> cq_storage::RangeSummary<K> for ZoneSummary<K> {
        fn observe(&mut self, key: &[i64; K]) {
            cq_storage::RangeSummary::observe(&mut self.0, key);
        }
        fn may_match(&self, bounds: &[(i64, i64)], columns: &[usize]) -> bool {
            GUARDS.with(|guards| guards.set(guards.get() + 1));
            cq_storage::RangeSummary::may_match(&self.0, bounds, columns)
        }
    }
}

// Configured bindings own representation and resource construction choices.
macro_rules! block_rows {
    ($($op:tt)*) => { ::mini_linq::block_rows! { storage crate::storage; packing (crate::size(2)); $($op)* } };
}
macro_rules! sorted_rows {
    ($($op:tt)*) => { ::mini_linq::block_rows! { storage crate::sorted_storage; packing (crate::size(2)); $($op)* } };
}
macro_rules! exact_keys {
    ($($op:tt)*) => { ::mini_linq::exact_keys! {
        storage crate::storage; iterator ::mini_linq::pull_stream;
        keys ::std::collections::HashSet; $($op)*
    } };
}
macro_rules! zone_keys {
    ($($op:tt)*) => { ::mini_linq::zone_keys! {
        storage crate::storage; iterator ::mini_linq::pull_stream;
        keys ::std::collections::HashSet; $($op)*
    } };
}
macro_rules! sorted_key_resource {
    ($($op:tt)*) => { ::mini_linq::exact_keys! {
        storage crate::sorted_storage; iterator ::mini_linq::pull_stream;
        keys crate::sorted_keys::SortedKeys; $($op)*
    } };
}
macro_rules! sorted_zone_keys {
    ($($op:tt)*) => { ::mini_linq::zone_keys! {
        storage crate::sorted_storage; iterator ::mini_linq::pull_stream;
        keys crate::sorted_keys::SortedKeys; $($op)*
    } };
}
macro_rules! prune {
    ($($op:tt)*) => { ::mini_linq::prune_rows! { iterator ::mini_linq::pull_stream; $($op)* } };
}
macro_rules! witnesses {
    ($($op:tt)*) => { ::mini_linq::hash_pairs! { $($op)* } };
}
pub(crate) use {
    block_rows, exact_keys, prune, sorted_key_resource, sorted_rows, sorted_zone_keys, witnesses,
    zone_keys,
};

macro_rules! uniform_physical {
    ($rows:path, $scan:path, $keys:path;
     $vis:vis struct $name:ident;
     $(relation $relation:ident($($column:tt)*);)*
     answer $($rest:tt)*) => {
        ::mini_linq::observed_relation_program! {
            storage crate::storage;
            access ::mini_linq::pull_stream;
            materialize $rows;
            inputs { $($relation => $rows;)* }
            scan $scan;
            keys $keys;
            join crate::witnesses;
            $vis struct $name;
            $(relation $relation($($column)*);)*
            answer $($rest)*
        }
    };
}
macro_rules! block_backend {
    ($($plan:tt)*) => {
        uniform_physical! { crate::block_rows, ::mini_linq::scan_rows, crate::exact_keys; $($plan)* }
    };
}
macro_rules! zone_backend {
    ($($plan:tt)*) => {
        uniform_physical! { crate::block_rows, crate::prune, crate::zone_keys; $($plan)* }
    };
}
macro_rules! sorted_backend {
    ($($plan:tt)*) => {
        uniform_physical! { crate::sorted_rows, ::mini_linq::scan_rows, crate::sorted_key_resource; $($plan)* }
    };
}
macro_rules! sorted_zone_backend {
    ($($plan:tt)*) => {
        uniform_physical! { crate::sorted_rows, crate::prune, crate::sorted_zone_keys; $($plan)* }
    };
}
pub(crate) use {block_backend, sorted_backend, sorted_zone_backend, zone_backend};

mod block_wiring {
    pub(crate) use crate::block_backend as choose_indexes;
    #[allow(unused_imports)]
    pub use ::mini_linq::{
        collect_pipeline, graph_query, iterator_pipeline, optimized_query, prefix_query, pull,
        pull_set, pull_stream, stream_pipeline,
    };
}
mod zone_wiring {
    pub(crate) use crate::zone_backend as choose_indexes;
    #[allow(unused_imports)]
    pub use ::mini_linq::{
        collect_pipeline, graph_query, iterator_pipeline, optimized_query, prefix_query, pull,
        pull_set, pull_stream, stream_pipeline,
    };
}
mod sorted_wiring {
    pub(crate) use crate::sorted_backend as choose_indexes;
    #[allow(unused_imports)]
    pub use ::mini_linq::{
        collect_pipeline, graph_query, iterator_pipeline, optimized_query, prefix_query, pull,
        pull_set, pull_stream, stream_pipeline,
    };
}
mod sorted_zone_wiring {
    pub(crate) use crate::sorted_zone_backend as choose_indexes;
    #[allow(unused_imports)]
    pub use ::mini_linq::{
        collect_pipeline, graph_query, iterator_pipeline, optimized_query, prefix_query, pull,
        pull_set, pull_stream, stream_pipeline,
    };
}

macro_rules! chosen_hash {
    ($($plan:tt)*) => {
        ::mini_linq::hash_materialized! { access mini_linq::pull_set; $($plan)* }
    };
}
pub(crate) use chosen_hash;
mod hash_wiring {
    pub(crate) use crate::chosen_hash as choose_realization;
    pub use ::mini_linq::result_indexes as choose_indexes;
    #[allow(unused_imports)]
    pub use ::mini_linq::{
        collect_pipeline, graph_query, iterator_pipeline, optimized_query, prefix_query, pull,
        pull_set, pull_stream, stream_pipeline,
    };
}

macro_rules! chain {
    ($name:ident) => {
        mini_linq::optimized_query! {
            pub struct $name;
            relation R(k: i64, x: i64);
            relation S(z: i64, k: i64);
            relation T(y: i64, z: i64);
            answer(x,y) :- R(k,x), S(z,k), T(y,z);
        }
    };
}
mod default_queries {
    use crate::hash_wiring as mini_linq;
    chain!(HashChain);
}
mod block_queries {
    use crate::block_wiring as mini_linq;
    chain!(BlockChain);
}
mod zone_queries {
    use crate::zone_wiring as mini_linq;
    chain!(ZoneChain);
}
mod sorted_queries {
    use crate::sorted_wiring as mini_linq;
    chain!(SortedChain);
}
mod sorted_zone_queries {
    use crate::sorted_zone_wiring as mini_linq;
    chain!(SortedZoneChain);
}
mod composed_hash_queries {
    use crate::hash_wiring as mini_linq;
    chain!(ComposedHashChain);
}
use block_queries::BlockChain;
use composed_hash_queries::ComposedHashChain;
use default_queries::HashChain;
use sorted_queries::SortedChain;
use sorted_zone_queries::SortedZoneChain;
use zone_queries::ZoneChain;

macro_rules! sweeps {
    ($entry:ident, $name:ident) => {
        $entry! {
            struct $name;
            relation R(k: i64, x: i64);
            relation S(z: i64, k: i64);
            relation T(y: i64, z: i64);
            answer(x,y) :- R(k,x), S(z,k), T(y,z);
            relational {
                r = rename R {k -> k, x -> x};
                s = rename S {z -> z, k -> k};
                t = rename T {y -> y, z -> z};
                s1 = semijoin s with t;
                r1 = semijoin r with s1;
                s2 = semijoin s1 with r1;
                t1 = semijoin t with s2;
                rs = natural_join r1 with s2;
                live = project rs keep {x, z};
                joined = natural_join live with t1;
                result = project joined keep {x, y};
                output result as answer(x,y);
            }
        }
    };
}
use hash_wiring as mini_linq;
use hash_wiring::choose_indexes;
sweeps!(choose_indexes, HashSweeps);
sweeps!(block_backend, BlockSweeps);
sweeps!(zone_backend, ZoneSweeps);
sweeps!(sorted_backend, SortedSweeps);
sweeps!(sorted_zone_backend, SortedZoneSweeps);

fn size(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).unwrap()
}
fn tuples(rows: &[[i64; 2]]) -> Vec<(i64, i64)> {
    rows.iter().map(|r| (r[0], r[1])).collect()
}

fn oracle(r: &[[i64; 2]], s: &[[i64; 2]], t: &[[i64; 2]]) -> Vec<(i64, i64)> {
    let mut out = BTreeSet::new();
    for &[k, x] in r {
        for &[z, sk] in s {
            for &[y, tz] in t {
                if k == sk && z == tz {
                    out.insert((x, y));
                }
            }
        }
    }
    out.into_iter().collect()
}

fn check(r: &[[i64; 2]], s: &[[i64; 2]], t: &[[i64; 2]]) {
    let expected = oracle(r, s, t);
    assert_eq!(HashChain::run(tuples(r), tuples(s), tuples(t)), expected);
    assert_eq!(
        ComposedHashChain::run(tuples(r), tuples(s), tuples(t)),
        expected
    );
    let (r, s, t) = (
        BlockStore::from_rows(r.iter().copied(), size(2)),
        BlockStore::from_rows(s.iter().copied(), size(2)),
        BlockStore::from_rows(t.iter().copied(), size(2)),
    );
    assert_eq!(BlockChain::run(&r, &s, &t).0, expected);
    assert_eq!(ZoneChain::run(&r, &s, &t).0, expected);
    assert_eq!(SortedChain::run(&r, &s, &t).0, expected);
    assert_eq!(SortedZoneChain::run(&r, &s, &t).0, expected);
}

#[test]
fn exhaustive_chain_routes_and_independent_reference_agree() {
    let rows = |mask: u8| {
        (0..4)
            .filter(|bit| mask & (1 << bit) != 0)
            .map(|bit| [i64::from(bit / 2), i64::from(bit % 2)])
            .collect::<Vec<_>>()
    };
    for r in 0..16 {
        for s in 0..16 {
            for t in 0..16 {
                check(&rows(r), &rows(s), &rows(t));
            }
        }
    }
    check(
        &[[i64::MIN, 1], [i64::MAX, 2], [i64::MAX, 2]],
        &[[4, i64::MAX], [3, i64::MIN]],
        &[[8, 4], [7, 3], [7, 3]],
    );
}

#[test]
fn rejected_blocks_and_pruned_index_inputs_are_observable() {
    let r = [[1, 10], [1, 11], [50, 20], [60, 21], [2, 30], [2, 31]];
    let s = [[100, 1], [200, 2], [900, 90]];
    let t = [[7, 100], [8, 200]];
    let (expected, hash) = HashSweeps::run_with_stats(tuples(&r), tuples(&s), tuples(&t));
    let (r, s, t) = (
        BlockStore::from_rows(r, size(2)),
        BlockStore::from_rows(s, size(1)),
        BlockStore::from_rows(t, size(2)),
    );
    let (actual, zone) = ZoneSweeps::run(&r, &s, &t);
    let (all, scan) = BlockSweeps::run(&r, &s, &t);
    let (sorted, sorted_zone) = SortedZoneSweeps::run(&r, &s, &t);
    let (sorted_all, sorted_scan) = SortedSweeps::run(&r, &s, &t);
    assert_eq!(actual, expected);
    assert_eq!(all, expected);
    assert_eq!(sorted, expected);
    assert_eq!(sorted_all, expected);
    assert_eq!(sorted_zone.steps, zone.steps);
    assert_eq!(sorted_scan.steps, scan.steps);
    for (((name, rows, _), (_, all_rows, _)), (hash_name, _, hash_rows)) in
        zone.steps.iter().zip(&scan.steps).zip(&hash.results)
    {
        assert_eq!(name, hash_name);
        assert_eq!(rows, hash_rows);
        assert_eq!(rows, all_rows);
    }
    let step = |name| zone.steps.iter().find(|(n, _, _)| *n == name).unwrap();
    assert_eq!(step("s1").1, 2);
    assert_eq!(step("s1").2.blocks_skipped, 1);
    assert_eq!(step("r1").2.index_rows, 2); // s1, not base S's three rows.
    assert_eq!(step("r1").2.blocks_skipped, 1);
    assert_eq!(step("r1").1, 4);
    assert_eq!(step("s2").2.index_rows, 4); // r1, not base R's six rows.
    assert_eq!(step("rs").2.index_rows, 2);
    assert!(
        zone.steps.iter().map(|s| s.2.rows_visited).sum::<usize>()
            < scan.steps.iter().map(|s| s.2.rows_visited).sum()
    );
    assert!(
        zone.steps.iter().map(|s| s.2.blocks_opened).sum::<usize>()
            < scan.steps.iter().map(|s| s.2.blocks_opened).sum()
    );
}

#[test]
fn caller_owned_collection_and_summary_are_used_by_the_compiled_route() {
    sorted_keys::take_counts();
    observed_summary::take_guards();
    let rows = BlockStore::from_rows([[1, 1], [2, 2]], size(1));
    let (answer, stats) = SortedZoneSweeps::run(&rows, &rows, &rows);
    let (builds, probes) = sorted_keys::take_counts();
    assert_eq!(answer, vec![(1, 1), (2, 2)]);
    // Four different right versions/separators demand four key collections.
    assert_eq!(builds, 4);
    assert_eq!(
        probes,
        stats
            .steps
            .iter()
            .filter(|(name, _, _)| ["s1", "r1", "s2", "t1"].contains(name))
            .map(|(_, _, work)| work.exact_probes)
            .sum::<usize>()
    );
    assert!(probes > 0);
    let guards = observed_summary::take_guards();
    assert_eq!(
        guards,
        stats
            .steps
            .iter()
            .map(|s| s.2.blocks_considered)
            .sum::<usize>()
    );
    assert!(guards > 0);
}

mod expression_route {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static PIPELINE_CALLS: AtomicUsize = AtomicUsize::new(0);

    macro_rules! observed_pipeline {
        ($($plan:tt)*) => {{
            crate::expression_route::PIPELINE_CALLS.fetch_add(1, Ordering::Relaxed);
            ::mini_linq::iterator_pipeline! { $($plan)* }
        }};
    }
    pub(crate) use observed_pipeline;

    mod wiring {
        pub(crate) use super::observed_pipeline as iterator_pipeline;
        pub use ::mini_linq::pull;
    }
    use mini_linq::pull as chosen_access;
    use wiring as mini_linq;

    #[test]
    fn imported_macro_aliases_accept_their_existing_languages() {
        PIPELINE_CALLS.store(0, Ordering::Relaxed);
        let rows = chosen_access! {
            out(x) => ((x,)) :- R(x) => for (x,) in ([(1,), (2,)]);
        };
        assert_eq!(rows.collect::<Vec<_>>(), vec![(1,), (2,)]);
        assert_eq!(PIPELINE_CALLS.load(Ordering::Relaxed), 1);

        // The same macro's default route still coexists in a sibling module.
        assert_eq!(super::default_expression::rows(), vec![(1,), (2,)]);
        assert_eq!(PIPELINE_CALLS.load(Ordering::Relaxed), 1);
    }
}
mod default_expression {
    pub fn rows() -> Vec<(i32,)> {
        mini_linq::pull! {
            out(x) => ((x,)) :- R(x) => for (x,) in ([(1,), (2,)]);
        }
        .collect()
    }
}

macro_rules! pair {
    ($module:ident, $hash:ident, $blocked:ident; $($body:tt)*) => {
        mod $module {
            pub mod hash {
                use crate::hash_wiring as mini_linq;
                mini_linq::optimized_query! { pub struct $hash; $($body)* }
            }
            pub mod exact {
                use crate::block_wiring as mini_linq;
                mini_linq::optimized_query! { pub struct $blocked; $($body)* }
            }
            pub mod zone {
                use crate::zone_wiring as mini_linq;
                mini_linq::optimized_query! { pub struct $blocked; $($body)* }
            }
            pub mod sorted {
                use crate::sorted_wiring as mini_linq;
                mini_linq::optimized_query! { pub struct $blocked; $($body)* }
            }
            pub mod sorted_zone {
                use crate::sorted_zone_wiring as mini_linq;
                mini_linq::optimized_query! { pub struct $blocked; $($body)* }
            }
        }
    };
}
pair!(composite, HashComposite, BlockComposite;
    relation R(a:i64,b:i64,x:i64); relation S(b:i64,a:i64,y:i64);
    answer(x,y) :- R(a,b,x), S(b,a,y);
);
pair!(product, HashProduct, BlockProduct;
    relation R(a:i64); relation S(b:i64);
    answer(x,y) :- R(x), S(y);
);
pair!(exists, HashExists, BlockExists;
    relation R(a:i64); relation S(b:i64);
    answer(x) :- R(x), S(y);
);
pair!(raw, HashRaw, BlockRaw;
    relation R(a:i64); relation S(b:i64);
    answer(r#type,r#type) :- R(r#type), S(r#type);
);

#[test]
fn composite_keys_products_empty_projection_and_raw_bindings_compile() {
    let r = [[1, 2, 10], [2, 1, 20], [1, 1, 30]];
    let s = [[1, 2, 40], [2, 1, 50], [2, 2, 60]];
    let expected = composite::hash::HashComposite::run(
        r.map(|[a, b, x]| (a, b, x)),
        s.map(|[b, a, y]| (b, a, y)),
    );
    assert_eq!(expected, vec![(10, 50), (20, 40)]);
    let (r, s) = (
        BlockStore::from_rows(r, size(1)),
        BlockStore::from_rows(s, size(1)),
    );
    for actual in [
        composite::exact::BlockComposite::run(&r, &s).0,
        composite::zone::BlockComposite::run(&r, &s).0,
        composite::sorted::BlockComposite::run(&r, &s).0,
        composite::sorted_zone::BlockComposite::run(&r, &s).0,
    ] {
        assert_eq!(actual, expected);
    }
    for left in [vec![], vec![[1], [1], [2]]] {
        for right in [vec![], vec![[2], [3]]] {
            let l = BlockStore::from_rows(left.iter().copied(), size(1));
            let r = BlockStore::from_rows(right.iter().copied(), size(1));
            let lt: Vec<_> = left.iter().map(|r| (r[0],)).collect();
            let rt: Vec<_> = right.iter().map(|r| (r[0],)).collect();
            let product = product::hash::HashProduct::run(lt.clone(), rt.clone());
            let exists = exists::hash::HashExists::run(lt.clone(), rt.clone());
            let raw = raw::hash::HashRaw::run(lt, rt);
            macro_rules! check_route {
                ($route:ident) => {
                    assert_eq!(product::$route::BlockProduct::run(&l, &r).0, product);
                    assert_eq!(exists::$route::BlockExists::run(&l, &r).0, exists);
                    assert_eq!(raw::$route::BlockRaw::run(&l, &r).0, raw);
                };
            }
            check_route!(exact);
            check_route!(zone);
            check_route!(sorted);
            check_route!(sorted_zone);
        }
    }
}

macro_rules! unit_plan {
    ($entry:ident, $name:ident) => {
        $entry! {
            struct $name; relation R(a:i64); answer(x) :- R(x);
            relational {
                u = unit;
                r = rename R {a -> x};
                ur = natural_join u with r;
                exists = project ur keep {};
                joined = natural_join r with exists;
                output joined as answer(x);
            }
        }
    };
}
unit_plan!(choose_indexes, HashUnit);
unit_plan!(block_backend, BlockUnit);
unit_plan!(zone_backend, ZoneUnit);
unit_plan!(sorted_backend, SortedUnit);
unit_plan!(sorted_zone_backend, SortedZoneUnit);

#[test]
fn explicit_unit_and_zero_column_views_compose() {
    for rows in [vec![], vec![[1], [1], [2]]] {
        let input = BlockStore::from_rows(rows.iter().copied(), size(1));
        let expected = HashUnit::run(rows.iter().map(|r| (r[0],)));
        assert_eq!(BlockUnit::run(&input).0, expected);
        assert_eq!(ZoneUnit::run(&input).0, expected);
        assert_eq!(SortedUnit::run(&input).0, expected);
        assert_eq!(SortedZoneUnit::run(&input).0, expected);
    }
}

block_backend! {
    struct RawSource;
    relation R(a:i64);
    answer(x) :- R(x);
    relational {
        r = rename r#R {r#a -> x};
        output r as answer(x);
    }
}

#[test]
fn alternate_raw_spellings_resolve_to_the_same_source_and_column() {
    let rows = BlockStore::from_rows([[3], [1], [3]], size(1));
    assert_eq!(RawSource::run(&rows).0, vec![(1,), (3,)]);
}

// Query identifiers must not capture generated data, statistics or helper types.
macro_rules! primitive_query {
    ($name:ident) => {
        mini_linq::optimized_query! {
            pub struct $name;
            relation R(a: i64);
            relation S(a: i64);
            answer(x) :- R(x), S(x);
        }
    };
}
mod primitive_query_name {
    #![allow(non_camel_case_types)]
    use crate::zone_wiring as mini_linq;
    primitive_query!(r#i64);
    primitive_query!(r#usize);
    primitive_query!(r#str);
    primitive_query!(r#bool);
}

#[test]
fn query_name_cannot_shadow_generated_primitive_types() {
    let rows = BlockStore::from_rows([[3], [1], [3]], size(1));
    let expected = vec![(1,), (3,)];
    macro_rules! check {
        ($name:ident) => {
            assert_eq!(primitive_query_name::$name::run(&rows, &rows).0, expected);
            let (answer, stats) = primitive_query_name::$name::run(&rows, &rows);
            assert_eq!(answer, expected);
            assert!(stats.steps.iter().any(|(_, _, work)| work.exact_probes > 0));
        };
    }
    check!(r#i64);
    check!(r#usize);
    check!(r#str);
    check!(r#bool);
}

sorted_zone_backend! {
    struct SharedKeys;
    relation R(k:i64,x:i64); relation S(k:i64,y:i64); relation T(k:i64);
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
fn same_current_right_version_and_separator_share_one_exact_collection() {
    let r = BlockStore::from_rows([[1, 10], [2, 20]], size(1));
    let s = BlockStore::from_rows([[1, 30], [2, 40]], size(1));
    let t = BlockStore::from_rows([[1]], size(1));
    sorted_keys::take_counts();
    let (answer, stats) = SharedKeys::run(&r, &s, &t);
    let (builds, probes) = sorted_keys::take_counts();
    assert_eq!(answer, vec![(10, 30)]);
    assert_eq!(builds, 1);
    let step = |name| stats.steps.iter().find(|(n, _, _)| *n == name).unwrap();
    assert_eq!(step("r1").2.index_rows, 1);
    assert_eq!(step("s1").2.index_rows, 0);
    assert_eq!(
        probes,
        step("r1").2.exact_probes + step("s1").2.exact_probes
    );
}

macro_rules! composite_reduction {
    ($entry:ident, $name:ident) => {
        $entry! {
            struct $name;
            relation R(a:i64,b:i64,x:i64); relation S(b:i64,a:i64);
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
composite_reduction!(choose_indexes, HashReduction);
composite_reduction!(block_backend, BlockReduction);
composite_reduction!(zone_backend, ZoneReduction);
composite_reduction!(sorted_backend, SortedReduction);
composite_reduction!(sorted_zone_backend, SortedZoneReduction);

macro_rules! empty_separator {
    ($entry:ident, $name:ident) => {
        $entry! {
            struct $name;
            relation R(a:i64); relation S(b:i64);
            answer(x) :- R(x), S(y);
            relational {
                r = rename R {a -> x};
                s = rename S {b -> y};
                exists = project s keep {};
                result = semijoin r with exists;
                output result as answer(x);
            }
        }
    };
}
empty_separator!(choose_indexes, HashEmptySeparator);
empty_separator!(block_backend, BlockEmptySeparator);
empty_separator!(zone_backend, ZoneEmptySeparator);
empty_separator!(sorted_backend, SortedEmptySeparator);
empty_separator!(sorted_zone_backend, SortedZoneEmptySeparator);

#[test]
fn semijoin_output_is_exact_without_a_later_join_to_remove_false_positives() {
    // Independent column ranges accept every row here; the composite key guard
    // must still reject (1,1) and (2,2), including when both fit one block.
    let left = [[1, 1, 10], [1, 2, 20], [2, 1, 30], [2, 2, 40]];
    let right = [[1, 2], [2, 1]];
    let expected = vec![(20,), (30,)];
    assert_eq!(
        HashReduction::run(left.map(|[a, b, x]| (a, b, x)), right.map(|[b, a]| (b, a))),
        expected,
    );
    for block_rows in [size(1), size(4)] {
        let left = BlockStore::from_rows(left, block_rows);
        let right = BlockStore::from_rows(right, block_rows);
        for actual in [
            BlockReduction::run(&left, &right).0,
            ZoneReduction::run(&left, &right).0,
            SortedReduction::run(&left, &right).0,
            SortedZoneReduction::run(&left, &right).0,
        ] {
            assert_eq!(actual, expected);
        }
    }
    for left in [vec![], vec![[1], [2], [2]]] {
        for right in [vec![], vec![[7]]] {
            let expected = if right.is_empty() {
                vec![]
            } else {
                left.iter()
                    .map(|row| (row[0],))
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect()
            };
            assert_eq!(
                HashEmptySeparator::run(
                    left.iter().map(|r| (r[0],)),
                    right.iter().map(|r| (r[0],))
                ),
                expected
            );
            let left = BlockStore::from_rows(left.iter().copied(), size(2));
            let right = BlockStore::from_rows(right, size(2));
            for actual in [
                BlockEmptySeparator::run(&left, &right).0,
                ZoneEmptySeparator::run(&left, &right).0,
                SortedEmptySeparator::run(&left, &right).0,
                SortedZoneEmptySeparator::run(&left, &right).0,
            ] {
                assert_eq!(actual, expected);
            }
        }
    }
}
