//! The condition after the CQ `if` is a Rust expression.
use super::{check, mini_linq, query};

fn accepts(x: &i32) -> bool {
    *x > 0 && *x < 4
}

query! {
    struct Call;
    relation R(key: i32);
    answer(x) :- R(x), if accepts(x);
}
query! {
    struct Compound;
    relation R(key: i32);
    answer(x) :- R(x), if *x > 0 && *x < 4;
}
query! {
    struct Block;
    relation R(key: i32);
    answer(x) :- R(x), if {
        // Lexical Rust shadowing does not rebind the CQ variable.
        let x = *x + 1;
        let between = |lower, upper| x > lower && x < upper;
        between(1, 5)
    };
}
query! {
    struct Matching;
    relation R(key: i32);
    answer(x) :- R(x), if match Some(x) {
        Some(x) => *x > 0,
        None => false,
    };
}
query! {
    struct Conditional;
    relation R(key: i32);
    answer(x) :- R(x), if if *x > 0 { accepts(x) } else { false };
}
macro_rules! positive {
    ($x:expr) => {
        *$x > 0
    };
}
query! {
    struct Macro;
    relation R(key: i32);
    answer(x) :- R(x), if positive!(x);
}

pub fn run() {
    let rows = [(-1,), (0,), (1,), (2,), (4,)];
    check("function guard", Call::run(rows), vec![(1,), (2,)]);
    check("compound guard", Compound::run(rows), vec![(1,), (2,)]);
    check(
        "block, closure, and lexical shadowing",
        Block::run(rows),
        vec![(1,), (2,)],
    );
    check("match guard", Matching::run(rows), vec![(1,), (2,), (4,)]);
    check(
        "Rust if expression",
        Conditional::run(rows),
        vec![(1,), (2,)],
    );
    check(
        "opaque Rust macro",
        Macro::run(rows),
        vec![(1,), (2,), (4,)],
    );
}
