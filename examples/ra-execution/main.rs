//! Change the RA continuation to choose whole-query execution.
use std::collections::{BTreeSet, HashSet};

macro_rules! query {
    () => {
        mini_linq::graph_query! {
            pub struct Star;
            relation Facts(key: i32, value: String);
            relation Tags(key: i32, tag: i32);
            relation Allowed(key: i32);
            answer(value, tag) :- Facts(key, value), Tags(key, tag), Allowed(key);
        }
    };
}

mod pull {
    macro_rules! realize {
        ($($ra:tt)*) => { ::mini_linq::nested_pull! {
            inputs {
                Facts => ::mini_linq::tuple_hash_rows;
                Tags => ::mini_linq::tuple_hash_rows;
                Allowed => ::mini_linq::tuple_sorted_rows;
            }
            $($ra)*
        } };
    }
    use realize as choose_indexes;
    mod routing {
        pub(super) use super::choose_indexes;
        pub use ::mini_linq::graph_query;
    }
    use routing as mini_linq;
    query!();
}

mod push {
    macro_rules! realize {
        ($($ra:tt)*) => { ::mini_linq::nested_push! {
            inputs {
                Facts => ::mini_linq::tuple_hash_rows;
                Tags => ::mini_linq::tuple_hash_rows;
                Allowed => ::mini_linq::tuple_sorted_rows;
            }
            $($ra)*
        } };
    }
    use realize as choose_indexes;
    mod routing {
        pub(super) use super::choose_indexes;
        pub use ::mini_linq::graph_query;
    }
    use routing as mini_linq;
    query!();
}

fn main() {
    let facts = HashSet::from([(1, "alpha".to_owned()), (2, "beta".to_owned())]);
    let tags = HashSet::from([(1, 10), (1, 20), (2, 30)]);
    let allowed = BTreeSet::from([(1,)]);

    let pulled: HashSet<_> = pull::Star::iter(&facts, &tags, &allowed).collect();
    let mut pushed = HashSet::new();
    push::Star::for_each(&facts, &tags, &allowed, |row| {
        pushed.insert(row);
    });
    assert_eq!(pulled, pushed);
    assert_eq!(pulled.len(), 2);
    assert!(pulled.iter().all(|(value, _)| value.as_str() == "alpha"));
    println!(
        "Both RA realizations produced {} distinct borrowed answers.",
        pulled.len()
    );
}
