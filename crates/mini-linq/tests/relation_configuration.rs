#![cfg(all(
    feature = "relations",
    feature = "tuple-hash-rows",
    feature = "hash-pairs"
))]

use std::collections::HashSet;

mini_linq::relation_program! {
    inputs { R => mini_linq::tuple_hash_rows; }
    struct RenameOnly;
    relation R(name: String, value: i32);
    answer(name, value) :- R(name, value);
    relational {
        r = rename R {name -> name, value -> value};
        output r as answer(name, value);
    }
}

mini_linq::relation_program! {
    inputs { R => mini_linq::tuple_hash_rows; S => mini_linq::tuple_hash_rows; }
    results { joined => mini_linq::tuple_hash_rows; }
    joins { joined => mini_linq::hash_pairs; }
    struct JoinOnly;
    relation R(name: String, value: i32);
    relation S(name: String);
    answer(name, value) :- R(name, value), S(name);
    relational {
        r = rename R {name -> name, value -> value};
        s = rename S {name -> name};
        joined = natural_join r with s;
        output joined as answer(name, value);
    }
}

#[test]
fn rename_requires_only_its_input_representation() {
    let rows = HashSet::from([("one".to_owned(), 1), ("two".to_owned(), 2)]);
    assert_eq!(
        RenameOnly::run(&rows),
        vec![("one".to_owned(), 1), ("two".to_owned(), 2)]
    );
}

#[test]
fn join_requires_no_access_scan_keys_or_unused_default() {
    let left = HashSet::from([("one".to_owned(), 1), ("two".to_owned(), 2)]);
    let right = HashSet::from([("two".to_owned(),)]);
    assert_eq!(JoinOnly::run(&left, &right), vec![("two".to_owned(), 2)]);
}
