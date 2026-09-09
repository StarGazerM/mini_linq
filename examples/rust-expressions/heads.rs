//! Computed heads declare their public output types and run before deduplication.
use super::{check, query};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

query! {
    struct ComputedHead;
    relation R(key: i32);
    relation answer(label: String);
    answer(format!("item-{x}")) :- R(x);
}

// The caller supplies Rust expression fragments with their syntax contexts.
macro_rules! wrapped_query {
    ($name:ident, $arg:ident, $guard:expr, $head:expr) => {
        query! {
            struct $name;
            relation R(key: i32);
            relation answer(label: String);
            answer($head) :- R($arg), if $guard;
        }
    };
}
wrapped_query!(Wrapped, x, *x > 0, format!("wrapped-{x}"));

static ARG_CALLS: AtomicUsize = AtomicUsize::new(0);
static HEAD_CALLS: AtomicUsize = AtomicUsize::new(0);

fn key(x: &i32) -> i32 {
    ARG_CALLS.fetch_add(1, Relaxed);
    *x
}
fn head(x: &i32) -> i32 {
    HEAD_CALLS.fetch_add(1, Relaxed);
    *x
}
query! {
    struct Witnesses;
    relation R(key: i32, witness: i32);
    relation S(key: i32, witness: i32);
    relation answer(key: i32);
    answer(head(x)) :- R(x, witness), S(key(x), _);
}

pub fn run() {
    check(
        "computed head and implicit format capture",
        ComputedHead::run([(1,), (2,)]),
        vec![("item-1".into(),), ("item-2".into(),)],
    );
    check(
        "DSL macro wrapper",
        Wrapped::run([(0,), (1,)]),
        vec![("wrapped-1".into(),)],
    );
    ARG_CALLS.store(0, Relaxed);
    HEAD_CALLS.store(0, Relaxed);
    check(
        "computed head before distinct",
        Witnesses::run([(1, 10), (1, 20)], [(1, 30), (1, 40)]),
        vec![(1,)],
    );
    // Key computation belongs to the R prefix, head computation to R join S.
    assert_eq!(ARG_CALLS.load(Relaxed), 2);
    assert_eq!(HEAD_CALLS.load(Relaxed), 4);
    println!("  evaluation counts: 2 argument calls, 4 head calls, 1 result");
}
