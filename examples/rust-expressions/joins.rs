//! Join planning must respect evaluation boundaries, even for disconnected prefixes.
use super::{check, mini_linq, query};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

static KEY_CALLS: AtomicUsize = AtomicUsize::new(0);
static GUARD_CALLS: AtomicUsize = AtomicUsize::new(0);
fn key(x: &i32) -> i32 {
    KEY_CALLS.fetch_add(1, Relaxed);
    x + 1
}
fn guard(x: &i32, y: &i32) -> bool {
    GUARD_CALLS.fetch_add(1, Relaxed);
    x + y > 0
}

query! {
    struct Constant;
    relation R(key: i32, value: i32);
    relation S(key: i32, value: i32);
    relation T(key: i32, value: i32);
    // Planned: let fresh = 7; R, S, T. The written R,T prefix is Cartesian.
    answer(x, y) :- R(7, x), T(y, _), S(x, y);
}
query! {
    struct ConstantLet;
    relation R(key: i32, value: i32);
    relation S(key: i32, value: i32);
    relation T(key: i32, value: i32);
    // Explicit spelling of the constant-argument normalization.
    answer(x, y) :- let k: i32 = 7, R(k, x), T(y, _), S(x, y);
}
query! {
    struct DependentLet;
    relation R(key: i32, value: i32);
    relation S(key: i32, value: i32);
    relation T(key: i32, value: i32);
    // Planned: R, let, S, T. key runs once per R tuple.
    answer(x, y) :- R(x, _), let k = key(x), T(y, _), S(k, y);
}
query! {
    struct GuardBoundary;
    relation R(key: i32, value: i32);
    relation S(key: i32, value: i32);
    relation T(key: i32, value: i32);
    // Planned: R, T, guard, S. Moving S earlier would suppress guard calls.
    answer(x, y) :- R(x, _), T(y, _), if guard(x, y), S(x, y);
}
query! {
    struct ArgumentBoundary;
    relation R(key: i32, value: i32);
    relation S(key: i32, value: i32);
    relation T(key: i32, value: i32);
    // Planned: R, T, key, S. key runs once per R x T tuple, unlike DependentLet.
    answer(x, y) :- R(x, _), T(y, _), S(key(x), y);
}
query! {
    struct PureGuard;
    relation R(key: i32, value: i32);
    relation S(key: i32, value: i32);
    relation T(key: i32, value: i32);
    relation answer(label: String);
    // Planned: R, comparison, S, T, head. Primitive comparison is movable.
    answer(format!("{x}:{y}")) :- R(x, _), T(y, _), S(x, y), if x > 0;
}

pub fn run() {
    let r = [(7, 1), (7, 2), (8, 3)];
    let s = [(1, 10), (2, 20), (3, 30)];
    let t = [(10, 100), (20, 200)];
    check(
        "constant join (planned R,S,T)",
        Constant::run(r, s, t),
        vec![(1, 10), (2, 20)],
    );
    check(
        "equivalent typed-let join",
        ConstantLet::run(r, s, t),
        vec![(1, 10), (2, 20)],
    );

    let r = [(1, 100), (2, 200)];
    let s = [(2, 10), (3, 20)];
    KEY_CALLS.store(0, Relaxed);
    check(
        "dependent let (planned R,let,S,T)",
        DependentLet::run(r, s, t),
        vec![(1, 10), (2, 20)],
    );
    assert_eq!(KEY_CALLS.load(Relaxed), 2);
    KEY_CALLS.store(0, Relaxed);
    check(
        "computed argument (R,T,key,S)",
        ArgumentBoundary::run(r, s, t),
        vec![(1, 10), (2, 20)],
    );
    assert_eq!(KEY_CALLS.load(Relaxed), 4);

    let s = [(1, 10), (2, 20)];
    GUARD_CALLS.store(0, Relaxed);
    check(
        "guard boundary (R,T,guard,S)",
        GuardBoundary::run(r, s, t),
        vec![(1, 10), (2, 20)],
    );
    assert_eq!(GUARD_CALLS.load(Relaxed), 4);
    check(
        "primitive filter pushdown",
        PureGuard::run([(-1, 0), (1, 100), (2, 200)], s, t),
        vec![("1:10".into(),), ("2:20".into(),)],
    );
    println!("  boundary counts: let key=2, argument key=4, guard=4");
}
