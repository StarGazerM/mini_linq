//! Let bindings introduce Rust values; atom variables borrow input fields.
use super::{check, mini_linq, query};

query! {
    struct TypedLet;
    relation R(key: i32);
    // This explicit type also supplies the public result type.
    answer(label) :- R(x), let label: String = format!("item-{x}");
}
query! {
    struct Destructure;
    relation R(key: i32);
    relation answer(label: String);
    answer(label) :- R(x),
        let (n, label) = (*x + 1, format!("item-{x}")),
        if n > 1;
}
query! {
    struct OwnedJoin;
    relation R(label: String);
    relation S(left: String, right: String, value: i32);
    answer(label, y) :- R(text),
        let label: String = text.to_uppercase(),
        S(label, label, y),
        if !label.is_empty();
}

pub fn run() {
    check("typed let", TypedLet::run([(1,)]), vec![("item-1".into(),)]);
    check(
        "destructuring let",
        Destructure::run([(0,), (1,), (2,)]),
        vec![("item-1".into(),), ("item-2".into(),)],
    );
    // An owned String survives two matches; a repeated bound name is equality.
    check(
        "method, owned let, repeated join key",
        OwnedJoin::run(
            [("alice".into(),)],
            [
                ("ALICE".into(), "ALICE".into(), 10),
                ("ALICE".into(), "ALICE".into(), 20),
                ("ALICE".into(), "other".into(), 99),
            ],
        ),
        vec![("ALICE".into(), 10), ("ALICE".into(), 20)],
    );
}
