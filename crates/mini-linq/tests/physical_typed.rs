#![cfg(all(
    feature = "configured",
    feature = "tuple-hash-rows",
    feature = "tuple-sorted-rows",
    feature = "array-block-rows",
    feature = "hash-pairs",
    feature = "scan-candidates",
    feature = "exact-keys",
    feature = "summarized-exact-keys",
    feature = "prune-block-candidates",
    feature = "access-to-pipeline",
    feature = "pipeline-set",
    feature = "pipeline-support-iter"
))]

use std::{
    collections::{BTreeSet, HashSet},
    num::NonZeroUsize,
};

macro_rules! blocks {
    ($($operation:tt)*) => { ::mini_linq::array_block_rows! { storage ::cq_storage; $($operation)* } };
}
macro_rules! exact {
    ($($operation:tt)*) => { ::mini_linq::exact_keys! {
        storage ::cq_storage;
        access_iter ::mini_linq::access_to_support_iter;
        keys ::std::collections::HashSet;
        $($operation)*
    } };
}
macro_rules! zone {
    ($($operation:tt)*) => { ::mini_linq::summarized_exact_keys! {
        storage ::cq_storage;
        access_iter ::mini_linq::access_to_support_iter;
        keys ::std::collections::BTreeSet;
        $($operation)*
    } };
}
macro_rules! prune {
    ($($operation:tt)*) => { ::mini_linq::prune_block_candidates! {
        access_iter ::mini_linq::access_to_support_iter; $($operation)*
    } };
}

macro_rules! mixed_query {
    ($name:ident, $scan:path, $keys:path) => {
        ::mini_linq::observed_configured_program! {
            counter_storage ::cq_storage;
            access ::mini_linq::access_to_set;
            result_rows ::mini_linq::tuple_hash_rows;
            inputs {
                People => ::mini_linq::tuple_hash_rows;
                Tags => ::mini_linq::tuple_sorted_rows;
                Facts => blocks;
            }
            result_bindings { extended => ::mini_linq::tuple_sorted_rows; }
            candidates ::mini_linq::scan_candidates;
            candidate_bindings { live_facts => $scan; }
            keys exact;
            key_sets { live_facts => $keys; }
            pairs ::mini_linq::hash_pairs;
            pair_bindings {}
            struct $name;
            relation People(name: String, age: i32, key: i64);
            relation Tags(key: i64, label: String);
            relation Facts(key: i64, value: i64);
            relation answer(text: String, score: i32, value: i64);
            answer(format!("{label}:{name}"), score, value) :-
                People(name, age, key), Tags(key, label), Facts(key, value),
                if *age >= 18, let score: i32 = *age + 1, if score > 19;
            relational {
                people = rename People { name -> name, age -> age, key -> key };
                tags = rename Tags { key -> key, label -> label };
                facts = rename Facts { key -> key, value -> value };
                adults = select people if (*age >= 18);
                named = semijoin adults with tags;
                live_facts = semijoin facts with named;
                labeled = natural_join named with tags;
                joined = natural_join labeled with live_facts;
                extended = extend joined with { score: i32 } value ((*age + 1,));
                accepted = select extended if (score > 19);
                result = project accepted keep {name, label, score, value};
                output result as answer(format!("{label}:{name}"), score, value);
            }
        }
    };
}
mixed_query!(Pruned, prune, zone);
mixed_query!(Scanned, ::mini_linq::scan_candidates, exact);

#[test]
fn typed_relations_expressions_and_block_pruning_compose_in_one_query() {
    let people = HashSet::from([
        ("Ada".to_owned(), 19, 1),
        ("Bob".to_owned(), 17, 2),
        ("Cy".to_owned(), 20, 3),
        ("Dee".to_owned(), 25, 99),
    ]);
    let tags = BTreeSet::from([
        (1, "a".to_owned()),
        (2, "b".to_owned()),
        (3, "c".to_owned()),
        (99, "z".to_owned()),
    ]);
    let facts = cq_storage::BlockStore::from_rows(
        [[1, 10], [3, 30], [400, 400], [500, 500]],
        NonZeroUsize::new(1).unwrap(),
    );
    let (pruned, work) = Pruned::run(&people, &tags, &facts);
    let (scanned, _) = Scanned::run(&people, &tags, &facts);
    assert_eq!(
        pruned,
        vec![("a:Ada".to_owned(), 20, 10), ("c:Cy".to_owned(), 21, 30)]
    );
    assert_eq!(pruned, scanned);
    assert!(
        work.steps
            .iter()
            .find(|step| step.0 == "live_facts")
            .unwrap()
            .2
            .blocks_skipped
            > 0
    );
    // Selection borrowed these heterogeneous sources; it did not replace them
    // with a query-wide homogeneous relation wrapper.
    assert_eq!(people.len(), 4);
    assert_eq!(tags.len(), 4);
}

#[test]
fn tuple_operations_preserve_owned_strings_and_explicit_schema() {
    let rows: mini_linq::tuple_hash_rows! { type (String, i32,) } =
        HashSet::from([("first".to_owned(), 1), ("second".to_owned(), 2)]);
    let selected = mini_linq::tuple_hash_rows! {
        select (&rows) (rows.iter().filter(|row| row.1 > 1))
    };
    assert_eq!(selected.len(), 1);
    let selected_row = selected.into_iter().next().unwrap();
    assert_eq!(selected_row, &("second".to_owned(), 2));
    assert!(std::ptr::eq(selected_row, rows.get(selected_row).unwrap()));
    assert_eq!(rows.len(), 2);
    let unit: mini_linq::tuple_sorted_rows! { type () } = BTreeSet::from([()]);
    assert_eq!(mini_linq::tuple_sorted_rows! { scan (&unit) }.count(), 1);
}
