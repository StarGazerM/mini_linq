//! Run the explicitly composed caller query over heterogeneous borrowed hosts.
use mini_linq_composition_example::query;
use std::collections::{BTreeMap, HashMap};

fn main() {
    let left = [(1,), (1,), (2,), (9,)];
    let middle = HashMap::from([((1,), vec![(10,), (11,)]), ((2,), vec![(20,)])]);
    let right = BTreeMap::from([
        ((10,), vec![(100,), (101,)]),
        ((11,), vec![]),
        ((20,), vec![(200,)]),
    ]);
    let borrowed = query(&left, &middle, &right);
    // Copy only for deterministic display; the query itself returns references.
    let mut rows: Vec<_> = borrowed.into_iter().map(|(a, b, c)| (*a, *b, *c)).collect();
    rows.sort_unstable();
    assert_eq!(rows, [(1, 10, 100), (1, 10, 101), (2, 20, 200)]);
    println!("Borrowed heterogeneous join: {rows:?}");
}
