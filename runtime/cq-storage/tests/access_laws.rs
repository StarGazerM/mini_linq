//! Exercise the semantic boundary through traits rather than inherent APIs.

use cq_storage::ExactMembership;
use std::collections::{BTreeSet, HashSet};

fn has<K: ?Sized>(keys: &impl ExactMembership<K>, key: &K) -> bool {
    keys.contains(key)
}

#[test]
fn exact_collections_and_borrowed_resources_preserve_membership() {
    for mask in 0..8 {
        let values: Vec<_> = (-1..=1)
            .filter(|value| mask & (1 << (value + 1)) != 0)
            .collect();
        let hash: HashSet<_> = values.iter().copied().collect();
        let sorted: BTreeSet<_> = values.iter().copied().collect();
        for key in -2..=2 {
            let expected = values.contains(&key);
            assert_eq!(has(&hash, &key), expected);
            assert_eq!(has(&sorted, &key), expected);
            assert_eq!(has(&&hash, &key), expected);
            assert_eq!(has(&&&sorted, &key), expected);
        }
    }
    let hash = HashSet::from([String::from("present")]);
    let sorted = BTreeSet::from([String::from("present")]);
    assert!(has(&hash, "present"));
    assert!(!has(&sorted, "absent"));
}
