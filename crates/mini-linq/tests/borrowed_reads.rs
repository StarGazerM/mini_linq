use std::{
    cell::Cell,
    collections::{BTreeSet, HashSet},
    hash::{Hash, Hasher},
};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Key(String);

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Payload(String);

impl Payload {
    fn read_metric(&self) -> i32 {
        self.0.len() as i32
    }
}

#[test]
fn repeated_native_selection_borrows_nonclone_hash_and_sorted_rows() {
    macro_rules! check {
        ($provider:ident, $set:ident) => {{
            let rows = $set::from([
                (Key("keep".into()), Payload("alpha".into()), 1),
                (Key("keep".into()), Payload("beta".into()), 2),
                (Key("drop".into()), Payload("gamma".into()), 3),
            ]);
            let first = mini_linq::$provider! {
                select (&rows) (rows.iter().filter(|row| row.0.0 == "keep"))
            };
            let second = mini_linq::$provider! {
                select (&first) (first.iter().filter(|row| row.2 == 2))
            };
            assert_eq!(first.len(), 2);
            assert_eq!(second.len(), 1);
            let selected = second.iter().next().unwrap();
            let original = rows.iter().find(|row| row.2 == 2).unwrap();
            assert_eq!(selected.1.0, "beta");
            assert!(std::ptr::eq(&selected.1, &original.1));
            assert_eq!(rows.len(), 3);
        }};
    }
    check!(tuple_hash_rows, HashSet);
    check!(tuple_sorted_rows, BTreeSet);
}

macro_rules! exact {
    ($($operation:tt)*) => { mini_linq::exact_keys! {
        storage ::cq_storage;
        access_iter mini_linq::access_to_support_iter;
        keys ::std::collections::HashSet;
        $($operation)*
    } };
}

mini_linq::configured_program! {
    access mini_linq::access_to_support_iter;
    result_rows mini_linq::tuple_hash_rows;
    inputs { R => mini_linq::tuple_hash_rows; S => mini_linq::tuple_sorted_rows; }
    candidates mini_linq::scan_candidates;
    keys exact;
    struct BorrowedSemijoin;
    relation R(key: Key, payload: Payload, value: i32);
    relation S(key: Key);
    answer(value) :- R(key,payload,value), S(key),
        if key.0.as_str() != "drop", if !payload.0.is_empty();
    relational {
        r = rename R {key -> key, payload -> payload, value -> value};
        s = rename S {key -> key};
        named = select r if (key.0.as_str() != "drop");
        present = select named if (!payload.0.is_empty());
        kept = semijoin present with s;
        numbers = project kept keep {value};
        output numbers as answer(value);
    }
}

mini_linq::configured_program! {
    inputs { R => mini_linq::tuple_hash_rows; }
    struct BorrowedSourceExpression;
    relation R(key: Key, payload: Payload, value: i32);
    relation answer(result: i32);
    answer(payload.read_metric() + *value) :- R(key,payload,value);
    relational {
        r = rename R {key -> key, payload -> payload, value -> value};
        output r as answer(payload.read_metric() + *value);
    }
}

fn nonclone_rows() -> HashSet<(Key, Payload, i32)> {
    HashSet::from([
        (Key("match".into()), Payload("alpha".into()), 7),
        (Key("absent".into()), Payload("beta".into()), 13),
        (Key("drop".into()), Payload("ignored".into()), 11),
        (Key("empty".into()), Payload(String::new()), 17),
    ])
}

#[test]
fn semijoin_keys_and_unused_payloads_do_not_require_clone() {
    let rows = nonclone_rows();
    let keys = BTreeSet::from([
        (Key("match".into()),),
        (Key("drop".into()),),
        (Key("empty".into()),),
    ]);
    assert_eq!(BorrowedSemijoin::run(&rows, &keys), vec![(7,)]);
    assert_eq!(rows.len(), 4);
    assert_eq!(keys.len(), 3);
}

#[test]
fn ordinary_rust_output_expressions_read_nonclone_source_fields() {
    let rows = nonclone_rows();
    assert_eq!(
        BorrowedSourceExpression::run(&rows),
        vec![(12,), (17,), (18,)]
    );
}

macro_rules! ignore_value {
    ($value:expr) => {
        true
    };
}

mini_linq::configured_program! {
    access mini_linq::access_to_support_iter;
    result_rows mini_linq::tuple_hash_rows;
    inputs { R => mini_linq::tuple_hash_rows; }
    candidates mini_linq::scan_candidates;
    struct BorrowedComputedExpression;
    relation R(value: i32);
    relation answer(result: i32);
    answer(measured.read_metric()) :- R(value),
        let measured: Payload = Payload(value.to_string()), if ignore_value!(measured);
    relational {
        r = rename R {value -> value};
        measured = extend r with { measured: Payload } value ((Payload(value.to_string()),));
        accepted = select measured if (ignore_value!(measured));
        output accepted as answer(measured.read_metric());
    }
}

#[test]
fn opaque_macros_and_output_expressions_read_nonclone_computed_fields() {
    let rows = HashSet::from([(-1,), (2,), (30,)]);
    assert_eq!(BorrowedComputedExpression::run(&rows), vec![(1,), (2,)]);
}

#[derive(Debug)]
struct CountedKey {
    text: String,
    counter: usize,
}

thread_local! {
    static KEY_CLONES: Cell<[usize; 5]> = const { Cell::new([0; 5]) };
}

impl Clone for CountedKey {
    fn clone(&self) -> Self {
        KEY_CLONES.with(|clones| {
            let mut counts = clones.get();
            counts[self.counter] += 1;
            clones.set(counts);
        });
        Self {
            text: self.text.clone(),
            counter: self.counter,
        }
    }
}

impl PartialEq for CountedKey {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
    }
}
impl Eq for CountedKey {}
impl PartialOrd for CountedKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for CountedKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.text.cmp(&other.text)
    }
}
impl Hash for CountedKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.text.hash(state);
    }
}

macro_rules! join_program {
    ($name:ident, $algorithm:ident) => {
        mini_linq::configured_program! {
            access mini_linq::access_to_support_iter;
            result_rows mini_linq::tuple_hash_rows;
            inputs { L => mini_linq::tuple_hash_rows; R => mini_linq::tuple_sorted_rows; }
            candidates mini_linq::scan_candidates;
            pairs mini_linq::$algorithm;
            struct $name;
            relation L(key: CountedKey, left: i32);
            relation R(key: CountedKey, right: i32);
            answer(left,right) :- L(key,left), R(key,right);
            relational {
                l = rename L {key -> key, left -> left};
                r = rename R {key -> key, right -> right};
                joined = natural_join l with r;
                numbers = project joined keep {left,right};
                output numbers as answer(left,right);
            }
        }
    };
}
join_program!(HashJoin, hash_pairs);
join_program!(NestedJoin, nested_pairs);
join_program!(MergeJoin, sort_merge_pairs);

#[test]
fn pair_algorithms_clone_keys_only_for_matching_owned_records() {
    type Join = fn(&HashSet<(CountedKey, i32)>, &BTreeSet<(CountedKey, i32)>) -> Vec<(i32, i32)>;
    for (name, run) in [
        ("hash", HashJoin::run as Join),
        ("nested", NestedJoin::run as Join),
        ("merge", MergeJoin::run as Join),
    ] {
        KEY_CLONES.with(|clones| clones.set([0; 5]));
        let key = |text: &str, index: usize| CountedKey {
            text: text.into(),
            counter: index,
        };
        let left = HashSet::from([
            (key("match", 0), 10),
            (key("left-only", 1), 11),
            (key("another-left", 2), 12),
        ]);
        let right = BTreeSet::from([(key("match", 3), 20), (key("right-only", 4), 21)]);
        let answer = run(&left, &right);
        drop((left, right));
        let counts = KEY_CLONES.with(Cell::get);
        assert_eq!(answer, vec![(10, 20)], "{name}");
        for index in [1, 2, 4] {
            assert_eq!(counts[index], 0, "{name}: unmatched key {index}");
        }
        // The explicit join record owns its common key. Numeric projection and
        // final output retain no key, so its one matching record needs one clone.
        assert_eq!(counts[0] + counts[3], 1, "{name}");
    }
}
