use cq_testkit::{Rows, SCENARIOS, harness};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

type Payload<'a, K, V> = cq_storage::HashIndex<'a, K, V>;
struct Membership<K>(BTreeSet<K>);

impl<K: Ord, V, I: Iterator<Item = (K, V)>> cq_storage::Build<I> for Membership<K> {
    fn build(source: I) -> Self {
        Self(source.map(|(key, _)| key).collect())
    }
}

impl<K: Ord> cq_storage::ExactMembership<K> for Membership<K> {
    fn contains(&self, key: &K) -> bool {
        self.0.contains(key)
    }
}
type Ordered<K, V> = BTreeMap<K, V>;
type Curried<'a, K, I> = cq_storage::Trie<'a, K, I>;
type State<K> = BTreeSet<K>;
type Buffer<T> = VecDeque<T>;
type Pairs = Vec<(i32, i32)>;

fn intersection_inputs() -> (Pairs, Pairs, Rows) {
    let case = SCENARIOS
        .iter()
        .find(|s| s.name == "intersection")
        .unwrap()
        .case()
        .unwrap();
    let input = |name| {
        case.inputs
            .iter()
            .find(|(n, _)| *n == name)
            .unwrap()
            .1
            .iter()
            .map(|r| (r[0], r[1]))
            .collect()
    };
    (input("R"), input("S"), case.expected_rows())
}

#[test]
#[allow(non_snake_case)]
fn merge_native_hosts_executes_corpus_ix_with_ordered_retype() {
    let (r, s, expected) = intersection_inputs();
    let R: BTreeMap<_, _> = r.into_iter().map(|row| (row, vec![()])).collect();
    let S: BTreeMap<_, _> = s.into_iter().map(|row| (row, vec![()])).collect();
    let actual: Rows = cq_rewrite_test_driver::corpus_ix! {
        intersection; pipeline {
            mini_linq::passes::lower,
            cq_rewrites::merge_joins(all),
            mini_linq::passes::select,
            cq_rewrites::retype(ordered => Ordered<_, Vec<_>>),
            mini_linq::passes::print_pull
        }
    }
    .map(|(x, y)| vec![*x, *y])
    .collect();
    assert_eq!(actual, expected);
}

#[test]
#[allow(non_snake_case)]
fn merge_mixed_native_and_retained_keys_execute_corpus_answers() {
    let (r, S, expected) = intersection_inputs();
    let R: BTreeMap<_, _> = r.into_iter().map(|row| (row, vec![()])).collect();
    let actual: Rows = cq_rewrites::merge_joins! {
        arguments { r2 };
        pipeline { mini_linq::passes::select, mini_linq::passes::print_pull };
        physical {
            relation R(first:i32, second:i32); relation S(first:i32, second:i32);
            r0 = pipe rename R {first -> x, second -> y};
            r1 = let rename S {first -> x, second -> y};
            r2 = pipe probe first [x,y] r0,r1;
            r3 = pipe canon project r2 keep {x,y}; return r3 as answer(x,y);
        }
    }
    .map(|(x, y)| vec![*x, *y])
    .collect();
    assert_eq!(actual, expected);
}

#[test]
#[allow(non_snake_case)]
fn named_payload_and_dedup_retypes_execute_corpus_physical() {
    let (R, S, expected) = intersection_inputs();
    let actual: Rows = cq_rewrite_test_driver::corpus_physical! {
        intersection; pipeline {
            mini_linq::passes::select,
            cq_rewrites::retype(r1 => Payload<'_, _, _>),
            cq_rewrites::retype(dedup => State<_>),
            mini_linq::passes::print_pull
        }
    }
    .map(|(x, y)| vec![*x, *y])
    .collect();
    assert_eq!(actual, expected);
}

#[test]
#[allow(non_snake_case)]
fn membership_trie_and_rows_retypes_execute_their_trait_obligations() {
    let L = BTreeMap::from([((1,), vec![()]), ((2,), vec![()]), ((3,), vec![()])]);
    let R = [(1, 10), (1, 11), (2, 20), (3, 30), (1, 10)];
    let S = [(1,), (3,)];
    let T = [(1, 10), (2, 20), (3, 30)];
    let actual: Rows = mini_linq::passes::select! {
        pipeline {
            cq_rewrites::retype(ordered => Ordered<_, Vec<_>>),
            cq_rewrites::retype(r => Payload<'_, _, _>),
            cq_rewrites::retype(s => Membership<_>),
            cq_rewrites::retype(trie => Curried<'_, _, Payload<'_, _, _>>),
            cq_rewrites::retype(rows => Buffer<_>),
            cq_rewrites::retype(dedup => State<_>),
            mini_linq::passes::print_pull
        };
        physical {
            relation L(a:i32); relation R(a:i32,b:i32); relation S(a:i32); relation T(a:i32,b:i32);
            l = pipe rename L {a -> x}; o = let rename L {a -> x};
            r = let rename R {a -> x,b -> y}; s = let rename S {a -> x};
            t = let rename T {a -> x,b -> y};
            j = pipe merge[x] l,o; n = pipe semi eager[x] j,t;
            p = let probe first[x] n,r; m = pipe semi eager[x] p,s;
            q = pipe probe first[x,y] m,t;
            out = pipe canon project q keep {x,y}; return out as answer(x,y);
        }
    }
    .map(|(x, y)| vec![*x, *y])
    .collect();
    assert_eq!(actual, Rows::from([vec![1, 10], vec![3, 30]]));
}

#[test]
fn facade_pipeline_keeps_opaque_rust_steps_in_written_order() {
    fn allowed(value: &i32) -> bool {
        *value != 0
    }
    fn run(rows: &[(i32, i32)]) -> Rows {
        mini_linq::compile! {
            pipeline {
                physical = [cq_rewrites::canon_prefixes, cq_rewrites::smallest_intersections, cq_rewrites::lazy_semijoins];
                structures = [cq_rewrites::retype(dedup => State<_>)];
            }
            struct Division;
            relation R(x:i32, y:i32) = (rows);
            answer(x, quotient) :- R(x,y), if (allowed(x)), let quotient:i32 = 100 / *x, if (*y < quotient + 1);
        }.map(|(x, quotient)| vec![*x, quotient]).collect()
    }
    let input = [(0, 999), (1, 2), (1, 2), (2, 60), (-1, -101)];
    harness::check_refinement_on(
        &input,
        |rows| {
            rows.iter()
                .filter(|(x, _)| *x != 0)
                .filter_map(|(x, y)| {
                    let quotient = 100 / x;
                    (*y < quotient + 1).then_some(vec![*x, quotient])
                })
                .collect::<Rows>()
        },
        |rows| run(rows),
    )
    .unwrap();
}
