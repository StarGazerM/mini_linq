//! Planner acceptance keeps the authored H05 schedules as independent costs
//! and semantic references; generated layouts need not duplicate their syntax.

#[allow(dead_code)]
#[path = "support/h05_plans.rs"]
mod h05_plans;

use cq_lang_sequence::{Program, check_grading, check_rewrite};

fn planned(authored: &Program) -> Program {
    let machine = cq_gpu_sequence::machine();
    let source = cq_sequence_lowering::compile(&authored.structures).unwrap();
    let source = cq_gpu_sequence_core::runs::record_representation(&source).unwrap();
    let result = cq_sequence_plan::decompose(&source, &machine, &cq_gpu_sequence::catalog()).unwrap();
    let repeated = cq_sequence_plan::decompose(&source, &machine, &cq_gpu_sequence::catalog()).unwrap();
    assert_eq!(
        result, repeated,
        "PL-Det: repeated planning must be identical"
    );
    let replayed = result
        .trace
        .iter()
        .fold(source.clone(), |previous, advice| {
            let next = advice.apply(&previous).unwrap();
            check_rewrite(&previous, &next).unwrap();
            next
        });
    assert_eq!(
        replayed, result.program,
        "PL-Closure: replay must reproduce every generated decision"
    );
    assert_eq!(
        result.program.structures, authored.structures,
        "decomposition must preserve the authored traversal and provider choices"
    );
    let analysis = check_grading(&result.program, &machine).unwrap();
    cq_gpu_sequence::translate(&result.program, &analysis).unwrap();
    result.program
}

#[test]
fn planned_r_grades_translates_replays_and_costs_no_more_than_authored() {
    assert_cost_acceptance("R", h05_plans::r());
}

#[test]
fn planned_b_grades_translates_replays_and_costs_no_more_than_authored() {
    assert_cost_acceptance("B", h05_plans::b());
}

#[test]
fn planned_a_retains_its_real_authored_traversal_choices() {
    // A is a separately authored Physical traversal, not an alternative a
    // decomposition planner may substitute for either B or R.
    let authored = h05_plans::a();
    planned(&authored);
}

fn assert_cost_acceptance(name: &str, authored: Program) {
    let machine = cq_gpu_sequence::machine();
    let authored = cq_gpu_sequence_core::runs::record_representation(&authored).unwrap();
    check_grading(&authored, &machine).unwrap();
    let generated = planned(&authored);
    let authored_cost = cq_sequence_plan::cost(&authored, &machine).unwrap();
    let planned_cost = cq_sequence_plan::cost(&generated, &machine).unwrap();
    eprintln!("H05 {name}: authored={authored_cost}; planned={planned_cost}");
    eprintln!(
        "H05 {name}: planned/authored ordering={:?}",
        planned_cost.partial_cmp(&authored_cost)
    );
    assert!(
        planned_cost.leq(&authored_cost),
        "H05 {name}: planned cost must be provably no greater than the authored reference"
    );
}
