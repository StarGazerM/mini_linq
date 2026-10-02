use cq_testkit::{Case, Rows, cq_semantics, databases};
use std::collections::BTreeMap;

type Raw = BTreeMap<String, Vec<Vec<i32>>>;

fn check(case: Case, execute: impl Fn(&Raw) -> (Vec<Vec<i32>>, Vec<Vec<i32>>)) {
    let raw: Raw = case
        .inputs
        .iter()
        .map(|(name, rows)| {
            (
                (*name).to_owned(),
                rows.iter().map(|row| row.to_vec()).collect(),
            )
        })
        .collect();
    agreement(case.name, &raw, &case.expected_rows(), &execute);
    let cq = case.parse_cq().unwrap();
    for seed in [0, 1, 2, 3, 5, 7, 11, 19] {
        let database = databases::binary_database(&cq, seed);
        let expected = cq_semantics::evaluate_cq(&cq, &database);
        let raw = database
            .into_iter()
            .map(|(name, rows)| (name, rows.into_iter().collect()))
            .collect();
        agreement(case.name, &raw, &expected, &execute);
    }
}

fn agreement(
    name: &str,
    raw: &Raw,
    expected: &Rows,
    execute: &impl Fn(&Raw) -> (Vec<Vec<i32>>, Vec<Vec<i32>>),
) {
    let (push, pull) = execute(raw);
    let push_set: Rows = push.iter().cloned().collect();
    let pull_set: Rows = pull.iter().cloned().collect();
    assert_eq!(
        push.len(),
        push_set.len(),
        "{name}: push emitted duplicate answers"
    );
    assert_eq!(
        pull.len(),
        pull_set.len(),
        "{name}: pull emitted duplicate answers"
    );
    assert_eq!(
        &push_set, expected,
        "{name}: push vs authored/oracle answers"
    );
    assert_eq!(
        &pull_set, expected,
        "{name}: pull vs authored/oracle answers"
    );
    assert_eq!(push_set, pull_set, "{name}: push vs pull");
}

include!(concat!(env!("OUT_DIR"), "/corpus.rs"));
