use cq_testkit::{CASES, SCENARIOS, cq_semantics, databases, semantics, valuation_semantics};

#[test]
fn cq_evaluator_reproduces_every_authored_case() {
    for case in CASES {
        assert_eq!(
            cq_semantics::evaluate_cq(&case.parse_cq().unwrap(), &case.database()),
            case.expected_rows(),
            "{}",
            case.name,
        );
    }
}

#[test]
fn ra_evaluator_reproduces_every_scenario_including_alternative_plans() {
    for scenario in SCENARIOS {
        let case = scenario.case().unwrap();
        assert_eq!(
            semantics::evaluate_ra(&scenario.parse_ra().unwrap(), &case.database()).output,
            case.expected_rows(),
            "{}",
            scenario.name,
        );
    }
}

#[test]
fn valuation_evaluator_reproduces_every_scenario_including_reductions() {
    for scenario in SCENARIOS {
        let case = scenario.case().unwrap();
        assert_eq!(
            valuation_semantics::evaluate(&scenario.parse_vo().unwrap(), &case.database()),
            case.expected_rows(),
            "{}",
            scenario.name,
        );
    }
}

#[test]
fn binary_database_has_deterministic_empty_full_and_sampled_seeds() {
    for scenario in SCENARIOS {
        let source = scenario.parse_cq().unwrap();
        let empty = databases::binary_database(&source, 0);
        assert!(empty.values().all(|rows| rows.is_empty()));
        let full = databases::binary_database(&source, 1);
        for declaration in source.inputs() {
            let rows = &full[&cq_testkit::cq::symbol_name(&declaration.name)];
            assert_eq!(rows.len(), 1 << declaration.columns.len());
        }
        assert_eq!(
            databases::binary_database(&source, 37),
            databases::binary_database(&source, 37),
        );
    }
}
