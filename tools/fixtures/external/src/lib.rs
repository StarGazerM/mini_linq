#[test]
fn packaged_facade_accepts_an_external_rewrite() {
    #[derive(PartialEq, Eq, PartialOrd, Ord)]
    struct Key(i32);
    let rows = [(Key(1),), (Key(1),), (Key(2),)];
    let answer: Vec<_> = mini_linq::compile! {
        pipeline { structures = [external_pass_fixture::ordered(a + (b * c))]; }
        struct Q; relation rows(x: Key); answer(x) :- rows(x);
    }
    .collect();
    assert_eq!(answer.len(), 2);
    assert!(std::ptr::eq(answer[0].0, &rows[0].0));
}
