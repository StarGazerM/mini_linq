//! Observe A-Prefix versus A-Local placement using both public macro routes.
//! This asserts the generated schedule, not Rust function-argument semantics.
use std::cell::RefCell;

thread_local! {
    static EVENTS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn f(x: &i32) -> i32 {
    EVENTS.with(|events| events.borrow_mut().push(format!("f({x})")));
    *x
}

fn g() -> i32 {
    EVENTS.with(|events| events.borrow_mut().push("g()".to_owned()));
    100
}

mini_linq::mini_linq! {
    struct Lazy;
    relation R(a:i32,b:i32,c:i32);
    answer(x) :- R(x, f(x), g());
}

mini_linq::planned_query! {
    struct Materialized;
    relation R(a:i32,b:i32,c:i32);
    answer(x) :- R(x, f(x), g());
}

fn check_observation(route: &str, rows: Vec<(i32,)>) {
    assert_eq!(rows, [(1,), (2,)]);
    let events = EVENTS.with(|events| std::mem::take(&mut *events.borrow_mut()));
    assert_eq!(events.first().map(String::as_str), Some("g()"));
    let mut after_g = events[1..].to_vec();
    after_g.sort();
    assert_eq!(after_g, ["f(1)", "f(2)"]);
    println!("{route}: answer={rows:?}, observed events={events:?}");
}

fn main() {
    let rows = [(1, 1, 100), (2, 2, 100)];
    check_observation("lazy", Lazy::run(rows));
    check_observation("materialized", Materialized::run(rows));
}
