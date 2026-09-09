// Public macro tests: these exercise every lowering and Rust's type checker.
use mini_linq::mini_linq;

fn successor(x: &i32) -> i32 {
    x + 1
}
fn accepts(x: &i32) -> bool {
    *x > 0 && *x < 4
}

query! {
    struct Calls;
    relation R(value: i32);
    answer(x) :- R(x), if accepts(x);
}
query! {
    struct Compound;
    relation R(value: i32);
    answer(x) :- R(x), if *x > 0 && *x < 4;
}
query! {
    struct SameAtom;
    relation R(a: i32, b: i32);
    answer(x) :- R(x, successor(x));
}
query! {
    struct PrefixArgument;
    relation R(value: i32);
    relation S(key: i32, value: i32);
    answer(x, y) :- R(x), S(successor(x), y);
}
query! {
    struct Repeated;
    relation R(a: String, b: String);
    answer(x, x) :- R(x, x);
}
query! {
    struct Literal;
    relation R(a: i32, b: String);
    answer(y) :- R(7, y);
}
query! {
    struct Wildcard;
    relation R(a: i32, b: i32);
    answer(x) :- R(_, x);
}
query! {
    struct ComputedHead;
    relation R(value: i32);
    relation answer(value: String);
    answer(format!("item-{x}")) :- R(x);
}
query! {
    struct Destructure;
    relation R(value: i32);
    relation answer(value: String);
    answer(label) :- R(x), let (n, label) = (*x + 1, format!("item-{x}")), if n > 1;
}
query! {
    struct Block;
    relation R(value: i32);
    answer(x) :- R(x), if { let x = *x + 1; let f = |x| x > 1; f(x) };
}
query! {
    struct Matching;
    relation R(value: i32);
    answer(x) :- R(x), if match Some(x) { Some(x) => *x > 0, _ => false };
}
macro_rules! positive {
    ($x:expr) => {
        *$x > 0
    };
}
query! {
    struct NestedMacro;
    relation R(value: i32);
    answer(x) :- R(x), if positive!(x);
}
mini_linq! {
    struct BasicExpressions;
    relation R(a: i32, b: i32);
    relation answer(value: String);
    answer(format!("{x}")) :- R(x, successor(x)), if accepts(x);
}

query! {
    struct TypedLet;
    relation R(value: i32);
    answer(label) :- R(x), let label: String = format!("item-{x}");
}
query! {
    struct BoundRepeated;
    relation R(value: i32);
    relation S(left: String, right: String);
    answer(label) :- R(x), let label: String = format!("item-{x}"), S(label, label);
}

#[test]
fn general_rust_conditions_execute() {
    let rows = [(-1,), (0,), (1,), (2,), (4,)];
    assert_eq!(Calls::run(rows), vec![(1,), (2,)]);
    assert_eq!(Compound::run(rows), vec![(1,), (2,)]);
    assert_eq!(Block::run(rows), vec![(1,), (2,), (4,)]);
    assert_eq!(Matching::run(rows), vec![(1,), (2,), (4,)]);
    assert_eq!(NestedMacro::run(rows), vec![(1,), (2,), (4,)]);
}

#[test]
fn expression_and_repeated_atom_arguments_match_values() {
    assert_eq!(
        SameAtom::run([(1, 2), (1, 7), (2, 3), (2, 3)]),
        vec![(1,), (2,)]
    );
    assert_eq!(
        PrefixArgument::run([(1,), (2,)], [(2, 10), (3, 20), (9, 90)]),
        vec![(1, 10), (2, 20)]
    );
    assert_eq!(
        Repeated::run([("a".into(), "a".into()), ("a".into(), "b".into())]),
        vec![("a".into(), "a".into())]
    );
    assert_eq!(
        Literal::run([(7, "yes".into()), (8, "no".into())]),
        vec![("yes".into(),)]
    );
    assert_eq!(Wildcard::run([(1, 5), (2, 5), (3, 7)]), vec![(5,), (7,)]);
}

#[test]
fn computed_output_types_and_let_patterns_execute() {
    assert_eq!(TypedLet::run([(1,)]), vec![("item-1".into(),)]);
    assert_eq!(
        BoundRepeated::run(
            [(1,)],
            [
                ("item-1".into(), "item-1".into()),
                ("item-1".into(), "other".into())
            ]
        ),
        vec![("item-1".into(),)]
    );

    assert_eq!(
        ComputedHead::run([(1,), (2,)]),
        vec![("item-1".into(),), ("item-2".into(),)]
    );
    assert_eq!(
        Destructure::run([(0,), (1,), (2,)]),
        vec![("item-1".into(),), ("item-2".into(),)]
    );
    assert_eq!(BasicExpressions::run([(1, 2), (2, 8)]), vec![("1".into(),)]);
}

static ARG_CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static HEAD_CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
fn counted_arg(x: &i32) -> i32 {
    ARG_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    *x
}
fn counted_head(x: &i32) -> i32 {
    HEAD_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    *x
}
query! {
    struct EvaluationCount;
    relation R(x: i32, witness: i32);
    relation S(x: i32, witness: i32);
    relation answer(x: i32);
    answer(counted_head(x)) :- R(x, witness), S(counted_arg(x), _);
}
#[test]
fn computation_precedes_matching_and_head_precedes_deduplication() {
    use std::sync::atomic::Ordering::Relaxed;
    ARG_CALLS.store(0, Relaxed);
    HEAD_CALLS.store(0, Relaxed);
    assert_eq!(
        EvaluationCount::run([(1, 10), (1, 20)], [(1, 30), (1, 40)]),
        vec![(1,)]
    );
    assert_eq!(ARG_CALLS.load(Relaxed), 2);
    assert_eq!(HEAD_CALLS.load(Relaxed), 4);
}

static LAZY_CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
fn lazy_label(x: &i32) -> String {
    LAZY_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("item-{x}")
}
mini_linq! {
    struct LazyBindings;
    relation R(x: i32);
    relation S(x: i32, y: i32);
    relation answer(value: String);
    answer(text) :- R(x), let text = lazy_label(x), S(x, y), if !text.is_empty();
}
#[test]
fn default_route_preserves_laziness_and_owned_bindings_across_joins() {
    use std::sync::atomic::Ordering::Relaxed;
    LAZY_CALLS.store(0, Relaxed);
    let storage = LazyBindings::build([(1,)], [(1, 10), (1, 20)]);
    assert_eq!(LAZY_CALLS.load(Relaxed), 0);
    let mut cursor = storage.query();
    assert_eq!(LAZY_CALLS.load(Relaxed), 0);
    assert_eq!(cursor.next(), Some(("item-1".into(),)));
    assert_eq!(LAZY_CALLS.load(Relaxed), 1);
    assert_eq!(cursor.next(), None);
    assert_eq!(LAZY_CALLS.load(Relaxed), 1);
    assert_eq!(storage.materialize(), vec![("item-1".into(),)]);
    assert_eq!(LAZY_CALLS.load(Relaxed), 2);
}

macro_rules! wrapped_query {
    ($name:ident, $arg:ident, $condition:expr, $head:expr) => {
        query! {
            struct $name;
            relation R(value:i32);
            relation answer(value:String);
            answer($head) :- R($arg), if $condition;
        }
    };
}
wrapped_query!(Wrapped, x, accepts(x), format!("wrapped-{x}"));
#[test]
fn nested_dsl_macros_preserve_rust_syntax_contexts() {
    assert_eq!(Wrapped::run([(0,), (1,)]), vec![("wrapped-1".into(),)]);
}
