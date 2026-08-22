#![cfg(feature = "hw4")]

use mini_linq::mini_linq;

mini_linq! {
    pub struct SpendProgram;
    relation Person(c0: i32);
    relation Blocked(c0: i32, c1: i32);
    relation Purchase(c0: i32, c1: i32, c2: i32);
    spend(person, total) :-
        Person(person),
        !Blocked(person, _),
        agg total = sum(amount) in Purchase(person, _, amount).
}

mini_linq! {
    struct FullKeyNegationProgram;
    relation Person(c0: i32);
    relation Blocked(c0: i32);
    allowed(person) :- Person(person), !Blocked(person).
}

mini_linq! {
    struct GlobalSumProgram;
    relation Blocker(c0: i32);
    relation Values(c0: i32, c1: i32);
    global(total) :-
        !Blocker(_),
        agg total = sum(value) in Values(_, value).
}

#[test]
fn negation_and_correlated_sum_execute_with_set_input_semantics() {
    let rows = SpendProgram::run(
        [(1,), (2,), (3,), (3,)],
        [(2, 7), (2, 7)],
        [(1, 10, 5), (1, 20, 5), (1, 20, 5), (1, 30, 5), (2, 30, 99)],
    );

    // Ascent's sum emits zero on an empty matching input, so person 3 remains.
    assert_eq!(rows, vec![(1, 15), (3, 0)]);
}

#[test]
#[should_panic(expected = "MiniLinq sum overflow")]
fn sum_overflow_is_identical_in_debug_and_release_builds() {
    let _ = SpendProgram::run(
        [(1,)],
        ::std::iter::empty::<(::std::primitive::i32, ::std::primitive::i32)>(),
        [(1, 10, ::std::primitive::i32::MAX), (1, 20, 1)],
    );
}

#[test]
fn aggregate_fold_starts_when_the_result_cursor_is_advanced() {
    let storage = GlobalSumProgram::build(
        ::std::iter::empty::<(::std::primitive::i32,)>(),
        [(1, ::std::primitive::i32::MAX), (2, 1)],
    );

    // Constructing the whole-result cursor must not execute the aggregate.
    // The checked sum overflows only when the first result is requested.
    let mut rows = storage.query();
    let first = ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| rows.next()));
    let panic = first.expect_err("advancing the cursor must execute the aggregate fold");
    let message = panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str));
    assert!(
        message.is_some_and(|message| message.contains("MiniLinq sum overflow")),
        "unexpected panic payload: {message:?}",
    );
}

#[test]
fn negation_and_sum_cover_full_and_empty_key_shapes() {
    let allowed = FullKeyNegationProgram::run([(1,), (2,), (3,)], [(2,), (2,)]);
    assert_eq!(allowed, vec![(1,), (3,)]);

    let sum = GlobalSumProgram::run(
        ::std::iter::empty::<(::std::primitive::i32,)>(),
        [(1, ::std::primitive::i32::MAX), (2, 1), (3, -1)],
    );
    assert_eq!(sum, vec![(::std::primitive::i32::MAX,)]);

    let empty_sum = GlobalSumProgram::run(
        ::std::iter::empty::<(::std::primitive::i32,)>(),
        ::std::iter::empty::<(::std::primitive::i32, ::std::primitive::i32)>(),
    );
    assert_eq!(empty_sum, vec![(0,)]);

    let blocked = GlobalSumProgram::run([(9,)], [(1, 10)]);
    assert!(blocked.is_empty());
}
