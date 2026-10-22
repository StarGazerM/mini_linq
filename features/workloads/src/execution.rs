//! Tiny checked-in CSV fixtures and DuckDB goldens shared by executable consumers.
use crate::Scenario;
use std::path::{Path, PathBuf};

/// A Cartesian product of nonempty relations cannot have no matches.
/// This is the corpus's sole scenario exception, also checked by catalog tests.
pub fn scenarios(name: &str) -> &'static [Scenario] {
    if name == "cartesian-product" {
        &[Scenario::Coverage, Scenario::EmptyInput]
    } else {
        &Scenario::ALL
    }
}

pub fn directory(name: &str, scenario: Scenario) -> PathBuf {
    let case = crate::case(name).expect("catalog case");
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("data")
        .join(case.suite.as_str())
        .join(name)
        .join(scenario.as_str())
        .join("tiny")
}

/// Load and validate once, then bind each relation to a typed host.
pub fn inputs(name: &str, scenario: Scenario) -> crate::schema::Inputs {
    let case = crate::case(name).expect("catalog case");
    let dataset = case.load(directory(name, scenario)).unwrap();
    dataset
        .validate_provenance(
            case.suite,
            name,
            crate::GenerationConfig {
                scenario,
                scale: crate::Scale::Tiny,
                seed: crate::seed_for(case),
            },
        )
        .unwrap();
    crate::schema::Inputs::from_dataset(dataset)
}

pub fn check(name: &str, scenario: Scenario, mut actual: Vec<Vec<i32>>) {
    let case = crate::case(name).expect("catalog case");
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("golden")
        .join(case.suite.as_str())
        .join(name)
        .join(scenario.as_str())
        .join("tiny.csv");
    let expected = crate::dataset::read_result_rows(path, case.arity).unwrap();
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
