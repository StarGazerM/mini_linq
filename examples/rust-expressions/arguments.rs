//! Atom arguments bind bare names and match the values of Rust expressions.
use super::{check, query};

fn successor(x: &i32) -> i32 {
    x + 1
}

// Two generic arguments and a comma inside the call use Rust's grammar.
fn add<T: Into<i32>, U: Into<i32>>(x: T, y: U) -> i32 {
    x.into() + y.into()
}

query! {
    struct Constant;
    relation R(key: i32, value: String);
    answer(value) :- R(7, value);
}
query! {
    struct Repeated;
    relation R(left: String, right: String);
    answer(x, x) :- R(x, x);
}
query! {
    struct Wildcard;
    relation R(key: i32, value: i32);
    answer(value) :- R(_, value);
}
query! {
    struct PrefixArgument;
    relation R(key: i32);
    relation S(key: i32, value: i32);
    answer(x, y) :- R(x), S(successor(x), y);
}
query! {
    struct SameAtom;
    relation R(key: i32, successor: i32);
    answer(x) :- R(x, successor(x));
}
query! {
    struct GenericArgument;
    relation R(key: i32);
    relation S(key: i32, value: i32);
    answer(x, y) :- R(x), S(add::<i32, i32>(*x, 1), y);
}

pub fn run() {
    check(
        "literal argument",
        Constant::run([(7, "yes".into()), (8, "no".into())]),
        vec![("yes".into(),)],
    );
    check(
        "repeated String argument and head",
        Repeated::run([("a".into(), "a".into()), ("a".into(), "b".into())]),
        vec![("a".into(), "a".into())],
    );
    // Different witnesses produce one set result after projection.
    check(
        "wildcard and projected duplicates",
        Wildcard::run([(1, 5), (2, 5), (3, 7)]),
        vec![(5,), (7,)],
    );
    let r = [(1,), (2,)];
    let s = [(2, 10), (3, 20), (9, 90)];
    check(
        "prefix expression argument",
        PrefixArgument::run(r, s),
        vec![(1, 10), (2, 20)],
    );
    check(
        "generic call argument",
        GenericArgument::run(r, s),
        vec![(1, 10), (2, 20)],
    );
    check(
        "same-atom expression argument",
        SameAtom::run([(1, 2), (1, 7), (2, 3), (2, 3)]),
        vec![(1,), (2,)],
    );
}
