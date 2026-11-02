//! C1 through Physical lowering (see `stage_support::mutants`). The Physical
//! rewrite stages run in `features/rewrites/tests/rewrite_stages.rs`.
//! Default tier: authored records. Opt-in (`--ignored`): generated seeds.
mod stage_support;

#[test]
fn authored_records_reject_mutants_or_keep_the_answer() {
    stage_support::mutants::authored_records_reject_mutants_or_keep_the_answer(
        stage_support::lowered_only,
    );
}

#[test]
#[ignore = "generated: opt in with --ignored (CQ_MUTANT_CASES, CQ_FUZZ_SEED)"]
fn generated_records_reject_mutants_or_keep_the_answer() {
    stage_support::mutants::generated_records_reject_mutants_or_keep_the_answer(
        stage_support::lowered_only,
    );
}
