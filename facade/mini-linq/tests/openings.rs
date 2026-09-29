//! The four interfaces compose ordinary Rust contexts around checked core holes.
#![cfg(all(
    feature = "logical-selection",
    feature = "hypergraph",
    feature = "index-demands",
    feature = "physical-lowering",
    feature = "structure-selection",
    feature = "physical-realization"
))]
#![allow(non_snake_case)]
mod mini_linq {
    pub use crate::strategy as physical_policy;
    pub use ::mini_linq::{
        cq_to_ra, index_demands as choose_index_policy, select_structures, structures_to_rust,
    };
    pub(crate) use cq_state_providers::default_indexes as index_provider;
    pub use cq_state_providers::hash_distinct as distinct_state;
}
#[macro_export]
macro_rules! strategy {
    ($($ix:tt)*) => { ::mini_linq::lower_physical! { $($ix)* policy {} } };
}
use cq_storage::{Answers, GroupedAnswers};

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
