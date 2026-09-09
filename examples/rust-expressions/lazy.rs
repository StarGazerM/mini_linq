//! The default route defers clause evaluation until a cursor advances.
use mini_linq::mini_linq;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

static CALLS: AtomicUsize = AtomicUsize::new(0);
fn label(x: &i32) -> String {
    CALLS.fetch_add(1, Relaxed);
    format!("item-{x}")
}
mini_linq! {
    struct Lazy;
    relation R(key: i32);
    relation S(key: i32, witness: i32);
    relation answer(label: String);
    answer(text) :- R(x), let text = label(x), S(x, y), if !text.is_empty();
}

pub fn run() {
    CALLS.store(0, Relaxed);
    let storage = Lazy::build([(1,)], [(1, 10), (1, 20)]);
    assert_eq!(CALLS.load(Relaxed), 0);
    let mut cursor = storage.query();
    assert_eq!(CALLS.load(Relaxed), 0);
    assert_eq!(cursor.next(), Some(("item-1".into(),)));
    assert_eq!(CALLS.load(Relaxed), 1);
    assert_eq!(cursor.next(), None);
    assert_eq!(CALLS.load(Relaxed), 1);
    // A fresh cursor evaluates the query again over the same stored inputs.
    assert_eq!(storage.materialize(), vec![("item-1".into(),)]);
    assert_eq!(CALLS.load(Relaxed), 2);
    println!("  lazy execution: build=0 calls, cursor=0, first next=1, fresh traversal=2");
}
