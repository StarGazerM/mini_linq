mod support;

use support::{fixtures, harness};

#[test]
fn supplied_fixtures_are_well_formed() {
    harness::assert_fixtures_well_formed();
}

#[test]
fn local_ir_contracts_reject_semantically_invalid_parsed_sources() {
    harness::assert_semantically_invalid_sources_fail_local_contracts();
}

#[test]
fn malformed_sources_fail_direct_syn_parsing() {
    harness::assert_malformed_sources_fail_direct_syn_parsing();
}

#[test]
fn supplied_pull_pass_returns_a_diagnostic_for_invalid_typed_source() {
    harness::assert_pull_rejects_invalid_typed_source();
}

#[test]
fn cq_to_relational_plan_contract() {
    for fixture in fixtures::cq_to_relational_plan_and_indexes() {
        harness::assert_cq_to_relational_plan(fixture);
    }
}

#[test]
fn relational_plan_to_index_requirements_contract() {
    for fixture in fixtures::cq_to_relational_plan_and_indexes() {
        harness::assert_relational_plan_to_index_requirements(fixture);
    }
}

#[test]
fn index_requirements_to_staged_rust_contract() {
    for fixture in fixtures::index_requirements_to_staged_rust() {
        harness::assert_index_requirements_to_staged_rust(fixture);
    }
    harness::assert_missing_required_index_rejected(fixtures::missing_required_index());
}
