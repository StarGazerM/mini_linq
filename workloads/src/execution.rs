//! Tiny checked-in CSV fixtures and DuckDB goldens shared by executable consumers.
use crate::{Scenario, dataset::Dataset};
use std::path::Path;

/// A Cartesian product of nonempty relations cannot have no matches.
/// This is the corpus's sole scenario exception, also checked by catalog tests.
pub fn scenarios(name: &str) -> &'static [Scenario] {
    if name == "cartesian-product" {
        &[Scenario::Coverage, Scenario::EmptyInput]
    } else {
        &Scenario::ALL
    }
}

pub fn load(name: &str, scenario: Scenario) -> Dataset {
    let case = crate::case(name).expect("catalog case");
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("data")
        .join(case.suite.as_str())
        .join(name)
        .join(scenario.as_str())
        .join("tiny");
    Dataset::read_from_dir(&case.module().unwrap(), directory).unwrap()
}

pub fn check(name: &str, scenario: Scenario, mut actual: Vec<Vec<i32>>) {
    let case = crate::case(name).expect("catalog case");
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("golden")
        .join(case.suite.as_str())
        .join(name)
        .join(scenario.as_str())
        .join("tiny.csv");
    let expected =
        crate::dataset::read_result_rows(path, case.module().unwrap().query.head.args.len())
            .unwrap();
    let returned_rows = actual.len();
    actual.sort();
    actual.dedup();
    assert_eq!(
        actual.len(),
        returned_rows,
        "{name}/{} returned duplicate answers",
        scenario.as_str()
    );
    assert_eq!(actual, expected, "{name}/{}", scenario.as_str());
}
