#![cfg(feature = "compiled-workloads")]
#![allow(dead_code, non_camel_case_types)]

use mini_linq::mini_linq;

mini_linq! {
    pub struct HygieneCollisionProgram;
    relation R(c0: ::std::primitive::i32, c1: ::std::primitive::i32);
    answer(result, relation0) :- R(result, relation0).
}

mini_linq! {
    pub struct Vec;
    relation Seed(c0: ::std::primitive::i32);
    relation Edge(c0: ::std::primitive::i32, c1: ::std::primitive::i32);
    answer(x, z) :- Seed(x), Edge(x, z).
}

mini_linq! {
    pub struct r#i32;
    relation R(c0: ::std::primitive::i32);
    answer(x) :- R(x).
}

mini_linq! {
    pub struct UnusedInputProgram;
    relation Used(c0: ::std::primitive::i32);
    relation Unused(c0: ::std::primitive::i32);
    answer(x) :- Used(x).
}

#[test]
fn real_proc_macro_expansion_keeps_query_variables_hygienic() {
    let storage = HygieneCollisionProgram::build([(2, 20), (1, 10), (2, 20)]);
    assert_eq!(storage.materialize(), vec![(1, 10), (2, 20)]);
    assert_eq!(
        storage.query().collect::<std::collections::HashSet<_>>(),
        std::collections::HashSet::from([(1, 10), (2, 20)]),
    );

    // A dropped result cursor does not consume the built physical storage.
    let mut prefix = storage.query().take(1);
    assert!(prefix.next().is_some());
    drop(prefix);
    assert_eq!(storage.materialize(), vec![(1, 10), (2, 20)]);

    assert_eq!(
        HygieneCollisionProgram::run([(2, 20), (1, 10), (2, 20)]),
        vec![(1, 10), (2, 20)],
    );
}

#[test]
fn generated_collection_types_are_absolute() {
    assert_eq!(
        Vec::run([(2,), (1,)], [(1, 10), (2, 20), (2, 21)]),
        vec![(1, 10), (2, 20), (2, 21)],
    );
}

#[test]
fn caller_qualified_primitive_types_survive_a_type_name_collision() {
    assert_eq!(r#i32::run([(2,), (1,), (2,)]), vec![(1,), (2,)]);
}

#[test]
fn generated_build_does_not_iterate_an_unused_input() {
    let panic_on_next = std::iter::from_fn(|| -> Option<(::std::primitive::i32,)> {
        panic!("an unused declared input must not be iterated")
    });

    let storage = UnusedInputProgram::build([(2,), (1,), (2,)], panic_on_next);
    assert_eq!(storage.materialize(), vec![(1,), (2,)]);
}
