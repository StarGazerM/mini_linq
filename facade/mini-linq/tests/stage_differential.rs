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

#[test]
#[ignore = "recorded generated findings; opt in with --ignored once fixed"]
fn recorded_findings_are_fixed() {
    stage_support::differential::recorded_findings_are_fixed(stage_support::lowered_only);
}

/// F1 (design-notes/test-layer-findings.md), minimized from seed 157: the
/// elaborate checker rejects the elaborate pass's own output.
#[test]
#[ignore = "F1 open: cq_elaborate::check rejects cq_elaborate::compile output"]
fn f1_elaborate_checker_accepts_fine_tree_split_projection() {
    let source: cq_lang_cq::Program = syn::parse_quote! {
        struct Q; relation r0(c0: i32, c1: i32); relation r2(c0: i32, c1: i32);
        relation answer(c0: i32);
        answer(v3) :- r2(v0, v1), r0(v3, v1), where v0 >= 3;
    };
    let normal = cq_lang_cq::normal::normalize(&source).unwrap();
    let order = cq_order::Options {
        heuristic: cq_order::Heuristic::Elimination,
        split: cq_order::Split::Fine,
    };
    let ordered = cq_order::compile(&normal, &order).unwrap();
    let reduce = cq_reduce::Options {
        semijoins: cq_reduce::Reductions::Tree,
    };
    let reduced = cq_reduce::compile(&ordered, &reduce).unwrap();
    let ra = cq_elaborate::compile(&reduced).unwrap();
    cq_elaborate::check(&reduced, &ra).unwrap();
}
