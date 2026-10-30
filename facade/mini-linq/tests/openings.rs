//! The four interfaces compose ordinary Rust contexts around checked core holes.
#![allow(non_snake_case)]

use cq_storage::{Answers, GroupedAnswers};

#[test]
fn computed_head_images_are_sets_after_projection() {
    let R = [(2, 3), (2, 4), (2, 4), (4, 5)];
    let projected: Vec<_> = ::mini_linq::mini_linq! {
        struct ProjectedImage; relation R(x: i32, y: i32); relation answer(x: i32);
        answer(*x) :- R(x, _);
    }
    .collect();
    assert_eq!(projected, [(2,), (4,)]);

    // Even when every source column is used, a computed head can map
    // different valuations to the same answer.
    let collapsed: Vec<_> = ::mini_linq::mini_linq! {
        struct ComputedImage; relation R(x: i32, y: i32); relation answer(x: i32);
        answer((*x + *y) % 2) :- R(x, y);
    }
    .collect();
    assert_eq!(collapsed, [(0,), (1,)]);
}

#[test]
fn literal_and_arithmetic_keys_use_owned_host_fallbacks() {
    let R = [(-2, 7), (3, 7), (3, 8), (-2, 7)];
    let positive: std::collections::BTreeSet<_> = ::mini_linq::mini_linq! {
        struct Nonleading; relation R(x: i32, tag: i32); relation answer(x: i32);
        answer(*x) :- R(x, 7);
    }
    .collect();
    assert_eq!(positive, [(-2,), (3,)].into());
    let negative: Vec<_> = ::mini_linq::mini_linq! {
        struct Leading; relation R(tag: i32, x: i32); relation answer(x: i32);
        answer(*x) :- R(-2, x);
    }
    .collect();
    assert_eq!(negative, [(7,)]);
    let complete: Vec<_> = ::mini_linq::mini_linq! {
        struct Complete; relation R(tag: i32, x: i32); relation answer(x: i32);
        answer(1) :- R(-2, 7);
    }
    .collect();
    assert_eq!(complete, [(1,)]);

    let Seed = [(1,), (2,)];
    let Dest = [(2, 10), (3, 20), (3, 20)];
    let computed: std::collections::BTreeSet<_> = ::mini_linq::mini_linq! {
        struct ArithmeticKey;
        relation Seed(x: i32); relation Dest(key: i32, value: i32);
        relation answer(x: i32, value: i32);
        answer(*x, *value) :- Seed(x), let key: i32 = *x + 1, Dest(key, value);
    }
    .collect();
    assert_eq!(computed, [(1, 10), (2, 20)].into());

    #[derive(PartialEq, Eq, PartialOrd, Ord)]
    struct Payload(i32);
    let Words = [(2, Payload(10))];
    let borrowed: Vec<_> = ::mini_linq::mini_linq! {
        struct BorrowedPayload; relation Words(key: i32, value: Payload);
        answer(value) :- Words(2, value);
    }
    .collect();
    assert!(std::ptr::eq(borrowed[0].0, &Words[0].1));
}

#[test]
fn reduced_literal_keys_probe_built_indexes() {
    let R = [(1, 7), (2, 7), (-1, 7), (1, 7)];
    let actual: std::collections::BTreeSet<_> =
        ::mini_linq::compile! { pipeline { plan = ::core_pass_fixtures::plan_hypergraph; }
            struct ReducedLiteral; relation R(x: i32, tag: i32); relation answer(x: i32);
            answer(*x) :- R(x, 7), where x > 0;
        }
        .collect();
    assert_eq!(actual, [(1,), (2,)].into());
}

mod computed_candidate_keys {

    #[test]
    fn arithmetic_prefix_keys_intersect_borrowed_candidates() {
        let Seed = [(1,)];
        let R = [(2, 5), (2, 6), (2, 5)];
        let S = [(2, 6), (3, 8)];
        let actual: Vec<_> = ::mini_linq::compile! { pipeline { plan = ::core_pass_fixtures::plan_generic; physical = [::core_pass_fixtures::smallest]; }
            struct Candidates;
            relation Seed(k: i32); relation R(k: i32, x: i32); relation S(k: i32, x: i32);
            relation answer(x: i32);
            answer(*x) :- Seed(k), let bound: i32 = *k + 1, R(bound, x), S(bound, x);
        }
        .collect();
        assert_eq!(actual, [(6,)]);
    }
}

#[test]
fn answers_from_a_query_are_a_zero_copy_canonical_host() {
    // String is deliberately non-Copy; both stages keep references to it.
    let R = [
        (String::from("a"),),
        (String::from("b"),),
        (String::from("a"),),
    ];
    let A = Answers::collect_from(::mini_linq::mini_linq! {
        struct First; relation R(x: String); answer(x) :- R(x);
    });
    let allowed = ["b"];
    let B = Answers::collect_from(::mini_linq::mini_linq! {
        struct Second;
        relation A(x: &String);
        relation answer(x: &String);
        answer(*x) :- A(x), if allowed.contains(&x.as_str());
    });
    assert_eq!(B.len(), 1);
    let (value,) = B.iter().next().unwrap();
    assert!(std::ptr::eq(*value, &R[1].0));
}

#[test]
fn grouped_answers_are_probed_in_place() {
    let R = [(1, 10), (1, 10), (2, 20), (2, 21)];
    let G = &GroupedAnswers::collect_from(
        ::mini_linq::mini_linq! {
            struct First; relation R(k: i32, v: i32); answer(k, v) :- R(k, v);
        }
        .map(|(k, v)| ((k,), (v,))),
    );
    let L = [(&R[2].0,)];
    let result: Vec<_> = ::mini_linq::mini_linq! {
        struct Probe;
        relation L(k: &i32);
        relation G(k: &i32, v: &i32);
        relation answer(v: i32);
        answer(**v) :- L(k), G(k, v);
    }
    .collect();
    assert_eq!(
        result
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>(),
        [(20,), (21,)].into()
    );
}

#[test]
fn owned_answers_and_once_evaluated_fresh_leaf_holes() {
    let calls = std::cell::Cell::new(0);
    let __cq_leaf_0 = 7;
    let result: Vec<_> = ::mini_linq::mini_linq! {
        struct Hole;
        relation Neg(x: i32) = ({
            calls.set(calls.get() + 1);
            Answers::collect_from([(1,), (2,), (2,)])
        });
        relation answer(x: i32, y: i32);
        answer(*x + __cq_leaf_0, *y) :- Neg(x), Neg(y);
    }
    .collect();
    assert_eq!(calls.get(), 1);
    assert_eq!(
        result
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>(),
        [(8, 1), (8, 2), (9, 1), (9, 2)].into()
    );
}

#[test]
fn a_borrowed_grouped_leaf_is_an_ordinary_host() {
    let G = GroupedAnswers::collect_from([((1,), (10,)), ((1,), (10,)), ((2,), (20,))]);
    let calls = std::cell::Cell::new(0);
    let L = [(1,)];
    let result: Vec<_> = ::mini_linq::mini_linq! {
        struct Hole;
        relation L(k: i32);
        relation Leaf(k: i32, v: i32) = ({ calls.set(calls.get()+1); &G });
        relation answer(v: i32);
        answer(*v) :- L(k), Leaf(k, v);
    }
    .collect();
    assert_eq!(result, [(10,)]);
    assert_eq!(calls.get(), 1);
}

#[test]
fn a_context_hole_can_supply_a_leaf_hole_and_leaf_order_is_written_order() {
    let R = [(2,), (3,)];
    let order = std::cell::RefCell::new(Vec::new());
    let result: Vec<_> = ::mini_linq::mini_linq! {
        struct Outer;
        relation Inner(x: i32) = ({
            order.borrow_mut().push(1);
            Answers::collect_from(::mini_linq::mini_linq! {
                struct Inner;
                relation R(x: i32); relation answer(x: i32);
                answer(*x * 2) :- R(x);
            })
        });
        relation Tail(y: i32) = ({ order.borrow_mut().push(2); [(1,)] });
        relation answer(x: i32);
        answer(*x + *y) :- Inner(x), Tail(y);
    }
    .collect();
    assert_eq!(*order.borrow(), [1, 2]);
    assert_eq!(
        result
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>(),
        [(5,), (7,)].into()
    );
}

#[test]
fn generated_result_names_do_not_shadow_input_or_output_relations() {
    // Differential seed 0: internal rN names collided with declared hosts.
    let r0 = [(3,), (3,), (7,)];
    let r1: [(i32,); 0] = [];
    let mut result: Vec<_> = ::mini_linq::mini_linq! {
        struct Collision;
        relation r0(x: i32);
        relation r1(x: i32);
        relation r2(x: i32);
        r2(*x + 1) :- r0(x);
    }
    .collect();
    result.sort();
    assert_eq!(result, [(4,), (8,)]);
}

mod fuzz_owned_merge_keys {

    #[test]
    fn constant_atom_merges_owned_literal_with_borrowed_host() {
        // Differential seed 2: the two merge cursors exposed i32 and &i32 keys.
        let R = std::collections::BTreeMap::from([((2,), vec![(), ()]), ((3,), vec![()])]);
        let rows: Vec<_> = ::mini_linq::compile! { pipeline { physical = [::core_pass_fixtures::merged, ::core_pass_fixtures::canonical_projects]; }
            struct LiteralMerge;
            relation R(x: i32);
            relation answer(value: i32);
            answer(1) :- R(2);
        }
        .collect();
        assert_eq!(rows, [(1,)]);
    }
}

#[test]
fn repeated_plain_head_variables_preserve_positions_and_input_references() {
    // Differential seed 4: the CQ normal-form check rejected answer(x, x).
    let R = [(String::from("left"),), (String::from("right"),)];
    let mut rows: Vec<_> = ::mini_linq::compile! { pipeline { plan = ::core_pass_fixtures::plan_prefix_connected_fine; }
        struct RepeatedHead;
        relation R(value: String);
        answer(x, x) :- R(x);
    }
    .collect();
    rows.sort();
    assert_eq!(rows, [(&R[0].0, &R[0].0), (&R[1].0, &R[1].0)]);
    assert!(std::ptr::eq(rows[0].0, rows[0].1));
    assert!(std::ptr::eq(rows[1].0, rows[1].1));
}

#[test]
fn fine_plan_probes_a_nonleading_literal_key() {
    // The fine-plan case found beside differential seed 5's computed keys.
    let R = [(1, -3, 4), (2, -3, 5), (2, -3, 5), (3, 7, 8)];
    let mut rows: Vec<_> =
        ::mini_linq::compile! { pipeline { plan = ::core_pass_fixtures::plan_prefix_greedy_fine; }
            struct FineLiteral;
            relation R(witness: i32, tag: i32, value: i32);
            answer(x) :- R(_, -3, x);
        }
        .collect();
    rows.sort();
    assert_eq!(rows, [(&4,), (&5,)]);
}

macro_rules! wide_distinct_regressions {
    ($module:ident, $provider:path) => {
        mod $module {
            #[test]
            fn thirteen_internal_columns_have_unbounded_distinct_keys() {
                // Differential seed 27 exposed std tuples' twelve-field Ord limit.
                // Five atoms of arity at most three legitimately bind thirteen columns.
                let R = [(1, 2, 3), (4, 5, 6), (1, 2, 3)];
                let S = [(7,), (8,), (7,)];
                let mut rows: Vec<_> = ::mini_linq::compile! { pipeline { physical = [::core_pass_fixtures::canonical_projects]; structures = [$provider]; }
                    struct Wide;
                    relation R(x: i32, y: i32, z: i32);
                    relation S(x: i32);
                    relation answer(value: i32);
                    answer(*a + *d + *g + *j + *m) :-
                        R(a, b, c), R(d, e, f), R(g, h, i), R(j, k, l), S(m);
                }
                .collect();
                rows.sort();
                assert_eq!(
                    rows,
                    [
                        (11,),
                        (12,),
                        (14,),
                        (15,),
                        (17,),
                        (18,),
                        (20,),
                        (21,),
                        (23,),
                        (24,)
                    ]
                );
            }

            #[test]
            fn computed_output_width_is_independent_of_input_heading() {
                let R = [(1, 0), (1, 2), (1, 1), (1, 3)];
                let rows: Vec<_> = ::mini_linq::compile! { pipeline { physical = [::core_pass_fixtures::canonical_projects]; structures = [$provider]; }
                    struct WideHead;
                    relation R(x: i32, y: i32);
                    relation answer(c0: i32, c1: i32, c2: i32, c3: i32, c4: i32,
                        c5: i32, c6: i32, c7: i32, c8: i32, c9: i32, c10: i32,
                        c11: i32, c12: i32);
                    answer(*x, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, *y % 2) :- R(x, y);
                }
                .collect();
                let mut ends = rows.iter().map(|row| (row.0, row.12)).collect::<Vec<_>>();
                ends.sort();
                assert_eq!(ends, [(1, 0), (1, 1)]);
            }
        }
    };
}
wide_distinct_regressions!(wide_sort, core_pass_fixtures::identity_structures);
