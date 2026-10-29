use cq_testkit::harness::{RefinementFailure, check_refinement};
use std::collections::BTreeSet;

fn set<T: Ord>(rows: impl IntoIterator<Item = T>) -> BTreeSet<T> {
    rows.into_iter().collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Kind {
    A,
    Pair(i32, (i32, i32)),
}

#[test]
fn option_fields_bind_join_keys_and_drop_failed_matches() {
    let links = [(1_i32, 10_i32), (2, 20), (2, 20), (3, 30)];
    for options in [
        vec![],
        vec![(None,)],
        vec![(Some(1),), (None,), (Some(2),), (Some(2),)],
    ] {
        let (values, joined) = cq_datalog::datalog! {
            struct OptionalValues;
            relation Optional(o:Option<i32>) = (&options);
            relation Link(x:i32,label:i32) = (links);
            relation Value(x:i32);
            relation Joined(x:i32,label:i32);
            output Value, Joined;
            Value(x) :- Optional(o), if let Some(x) = *o;
            Joined(x,*label) :- Optional(o), if let Some(x) = *o, Link(x,label);
        };
        ascent::ascent! {
            struct Reference;
            relation optional(Option<i32>);
            relation link(i32,i32);
            relation value(i32);
            relation joined(i32,i32);
            value(x) <-- optional(o), if let Some(x) = *o;
            joined(x,*label) <-- optional(o), if let Some(x) = *o, link(x,label);
        }
        let mut reference = Reference {
            optional: options,
            link: links.to_vec(),
            ..Default::default()
        };
        reference.run();
        let values: Vec<_> = values.collect();
        let joined: Vec<_> = joined.collect();
        assert_eq!(values.len(), set(&values).len());
        assert_eq!(joined.len(), set(&joined).len());
        assert_eq!(set(values), set(reference.value));
        assert_eq!(set(joined), set(reference.joined));
    }
}

#[test]
fn enum_variants_nested_tuples_and_patterns_without_bindings() {
    let kinds = [
        (Kind::A,),
        (Kind::Pair(1, (2, 3)),),
        (Kind::Pair(4, (5, 6)),),
        (Kind::A,),
    ];
    let nested = [(Some((7_i32, (8_i32, 9_i32))),), (None,)];
    let (pairs, tuples, accepted) = cq_datalog::datalog! {
        struct Patterns;
        relation Input(k:Kind) = (kinds);
        relation Nested(o:Option<(i32,(i32,i32))>) = (nested);
        relation Pair(x:i32,y:i32);
        relation Tuple(x:i32,y:i32);
        relation Accepted(k:Kind);
        output Pair, Tuple, Accepted;
        Pair(x,y) :- Input(k), if let Kind::Pair(x, (y, _)) = *k;
        Tuple(x,y) :- Nested(o), if let Some((x, (y, _))) = *o;
        Accepted(*k) :- Input(k), if let Kind::A = *k;
    };
    ascent::ascent! {
        struct Reference;
        relation input(Kind);
        relation nested(Option<(i32,(i32,i32))>);
        relation pair(i32,i32);
        relation tuple(i32,i32);
        relation accepted(Kind);
        pair(x,y) <-- input(k), if let Kind::Pair(x, (y, _)) = *k;
        tuple(x,y) <-- nested(o), if let Some((x, (y, _))) = *o;
        accepted(*k) <-- input(k), if let Kind::A = *k;
    }
    let mut reference = Reference {
        input: kinds.to_vec(),
        nested: nested.to_vec(),
        ..Default::default()
    };
    reference.run();
    assert_eq!(set(pairs), set(reference.pair));
    assert_eq!(set(tuples), set(reference.tuple));
    assert_eq!(set(accepted), set(reference.accepted));
}

#[test]
fn refutable_bindings_feed_recursive_heads_and_later_guards() {
    let seeds = [(Some(0_i32),), (None,), (Some(0),)];
    let actual = cq_datalog::datalog! {
        struct RecursiveMatches;
        relation Seed(o:Option<i32>) = (seeds);
        relation Reach(x:i32);
        output Reach;
        Reach(x) :- Seed(o), if let Some(x) = *o;
        Reach(y) :- Reach(x), if let Some(y) = x.checked_add(1), where y <= 4;
    };
    ascent::ascent! {
        struct Reference;
        relation seed(Option<i32>);
        relation reach(i32);
        reach(x) <-- seed(o), if let Some(x) = *o;
        reach(y) <-- reach(x), if let Some(y) = x.checked_add(1), if y <= 4;
    }
    let mut reference = Reference {
        seed: seeds.to_vec(),
        ..Default::default()
    };
    reference.run();
    let actual = set(actual);
    assert_eq!(actual, set([(0,), (1,), (2,), (3,), (4,)]));
    assert_eq!(actual, set(reference.reach));
}

fn partial_option(value: i32) -> Option<i32> {
    assert_ne!(value, 0, "partial if-let expression");
    (value > 0).then_some(value)
}

#[test]
fn expression_panics_violate_totality_instead_of_causing_match_failure() {
    for input in [vec![], vec![(1_i32,), (-1,)], vec![(0,)]] {
        let observation = check_refinement(
            || {
                ascent::ascent! {
                    struct Reference;
                    relation input(i32);
                    relation result(i32);
                    result(y) <-- input(x), if let Some(y) = partial_option(*x);
                }
                let mut reference = Reference {
                    input: input.clone(),
                    ..Default::default()
                };
                reference.run();
                set(reference.result)
            },
            || {
                set(cq_datalog::datalog! {
                    struct PartialMatch;
                    relation Input(x:i32) = (&input);
                    relation Result(y:i32);
                    output Result;
                    Result(y) :- Input(x), if let Some(y) = partial_option(*x);
                })
            },
        );
        if input == [(0,)] {
            assert!(matches!(
                observation,
                Err(RefinementFailure::ReferencePanicked { .. })
            ));
        } else {
            observation.unwrap();
        }
    }
}

#[test]
fn preceding_opaque_guard_protects_partial_match_expression() {
    let input = [(0_i32,), (1,), (-1,)];
    check_refinement(
        || {
            ascent::ascent! {
                struct Reference;
                relation input(i32);
                relation result(i32);
                result(y) <-- input(x), if x.is_positive(), if let Some(y) = partial_option(*x);
            }
            let mut reference = Reference {
                input: input.to_vec(),
                ..Default::default()
            };
            reference.run();
            set(reference.result)
        },
        || {
            set(cq_datalog::datalog! {
                struct GuardedMatch;
                relation Input(x:i32) = (input);
                relation Result(y:i32);
                output Result;
                Result(y) :- Input(x), if x.is_positive(), if let Some(y) = partial_option(*x);
            })
        },
    )
    .unwrap();
}
