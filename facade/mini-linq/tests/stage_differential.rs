//! R1 through Physical lowering (see `stage_support::differential`). The
//! Physical rewrite stages run in `features/rewrites/tests/rewrite_stages.rs`.
//! Generated tests are opt-in (`--ignored`).
mod stage_support;

#[test]
#[ignore = "generated: opt in with --ignored (CQ_STAGE_CASES, CQ_FUZZ_SEED)"]
fn every_stage_denotes_the_reference_answer() {
    stage_support::differential::every_stage_denotes_the_reference_answer(
        stage_support::lowered_only,
    );
}
