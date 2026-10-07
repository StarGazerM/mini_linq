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

#[test]
fn authored_block_programs_check_at_every_stage_and_match_all_oracles() {
    for scenario in cq_testkit::blocks::SCENARIOS {
        let cq = scenario.parse_cq().unwrap();
        cq_testkit::cq::contract::check(&cq).unwrap();
        let vo = scenario.parse_vo().unwrap();
        cq_testkit::vo::contract::check(&vo).unwrap();
        let ra = scenario.parse_ra().unwrap();
        cq_testkit::ra::contract::check(&ra).unwrap();
        let ix = scenario.parse_ix().unwrap();
        cq_testkit::ix::contract::check(&ix).unwrap();
        let physical = scenario.parse_physical().unwrap();
        cq_testkit::physical::contract::check(&physical).unwrap();
        let structures = scenario.parse_structures().unwrap();
        cq_testkit::structures::contract::check(&structures).unwrap();
        let case = scenario.case().unwrap();
        let database = case.database();
        assert_eq!(
            cq_testkit::cq_semantics::evaluate_cq(&cq, &database),
            case.expected_rows()
        );
        assert_eq!(
            cq_testkit::valuation_semantics::evaluate(&vo, &database),
            case.expected_rows()
        );
        assert_eq!(
            cq_testkit::semantics::evaluate_ra(&ra, &database).output,
            case.expected_rows()
        );
    }
}
