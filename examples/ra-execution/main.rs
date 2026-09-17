//! Borrowed YA execution through every selected compiler boundary.
use std::collections::{BTreeSet, HashSet};

mod wiring {
    pub use ::cq_storage as storage;
    pub use ::mini_linq::{
        borrowed_scans as choose_realization, exists_pipeline, graph_query as query,
        iterator_pipeline, no_indexes as choose_indexes, pull, pull_exists, pull_stream,
        replay_rows, stream_pipeline,
    };
    pub mod inputs {
        pub use ::mini_linq::{
            tuple_hash_rows as Facts, tuple_hash_rows as Tags, tuple_sorted_rows as Allowed,
        };
    }
}
use wiring as mini_linq;

mini_linq::query! {
    pub struct Star;
    relation Facts(key: i32, value: String);
    relation Tags(key: i32, tag: i32);
    relation Allowed(key: i32);
    answer(value, tag) :- Facts(key, value), Tags(key, tag), Allowed(key);
}

fn main() {
    let facts = HashSet::from([(1, "alpha".to_owned()), (2, "beta".to_owned())]);
    let tags = HashSet::from([(1, 10), (1, 20), (2, 30)]);
    let allowed = BTreeSet::from([(1,)]);
    let actual: HashSet<_> = Star::iter(&facts, &tags, &allowed).collect();
    let expected: HashSet<_> = facts
        .iter()
        .flat_map(|fact| {
            tags.iter().filter_map(|tag| {
                (fact.0 == tag.0 && allowed.contains(&(fact.0,))).then_some((&fact.1, &tag.1))
            })
        })
        .collect();
    assert_eq!(actual, expected);
    assert_eq!(actual.len(), 2);
    println!(
        "Staged YA produced {} distinct borrowed answers.",
        actual.len()
    );
}
