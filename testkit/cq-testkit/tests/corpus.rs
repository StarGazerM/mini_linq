use cq_testkit::{CASES, SCENARIOS, cq, cq_semantics, ix, physical, ra, structures, vo};
use std::collections::BTreeSet;

#[test]
fn every_corpus_stage_parses_and_satisfies_its_language_contract() {
    assert_eq!(SCENARIOS.len(), 30);
    assert_eq!(CASES.len(), 25);
    let names: BTreeSet<_> = SCENARIOS.iter().map(|scenario| scenario.name).collect();
    assert_eq!(names.len(), SCENARIOS.len());
    for scenario in SCENARIOS {
        let check = || -> syn::Result<()> {
            cq::contract::check(&scenario.parse_cq()?)?;
            vo::contract::check(&scenario.parse_vo()?)?;
            ra::contract::check(&scenario.parse_ra()?)?;
            ix::contract::check(&scenario.parse_ix()?)?;
            physical::contract::check(&scenario.parse_physical()?)?;
            structures::contract::check(&scenario.parse_structures()?)?;
            Ok(())
        };
        check().unwrap_or_else(|error| panic!("{}: {error}", scenario.name));
        let case = scenario
            .case()
            .expect("every built-in scenario has authored data");
        assert_eq!(scenario.parse_cq().unwrap(), case.parse_cq().unwrap());
        cq_semantics::validate_database(&scenario.parse_cq().unwrap(), &case.database());
    }
    for case in CASES {
        assert!(names.contains(case.name), "{} has no scenario", case.name);
        assert_eq!(case.expected.len(), case.expected_rows().len());
    }
}

#[test]
fn raw_authored_inputs_preserve_duplicates_but_databases_are_sets() {
    let scan = CASES.iter().find(|case| case.name == "scan").unwrap();
    assert_eq!(scan.inputs[0].1.len(), 3);
    assert_eq!(scan.database()["R"].len(), 2);
}
