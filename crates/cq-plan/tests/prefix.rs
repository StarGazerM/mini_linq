//! The component is tested without any peer pass or composition dependency.
#[allow(dead_code)]
#[path = "../../../test-support/relational/cases.rs"]
mod cases;
#[allow(dead_code)]
#[path = "../../../test-support/relational/semantics.rs"]
mod semantics;

use cq_ir::{cq, relational_plan as ra};
use semantics::{Database, Rows};

#[test]
fn standalone_lowering_preserves_authored_answers() {
    for case in cases::CASES {
        let authored: ra::Module = syn::parse_str(case.plan).unwrap();
        let source = cq::Module {
            program: authored.program,
        };

        let target = cq_plan::prefix::compile(&source).unwrap();
        let database: Database = case
            .inputs
            .iter()
            .map(|(name, rows)| {
                (
                    (*name).to_owned(),
                    rows.iter().map(|row| row.to_vec()).collect(),
                )
            })
            .collect();
        let expected: Rows = case.expected.iter().map(|row| row.to_vec()).collect();
        assert_eq!(
            semantics::evaluate_cq(&source.program, &database),
            expected,
            "{} source",
            case.name
        );
        assert_eq!(
            semantics::evaluate_ra(&target, &database).output,
            expected,
            "{} target",
            case.name
        );
    }
}

#[test]
fn public_replay_checks_the_whole_report_and_source() {
    let source: cq::Module = syn::parse_quote! {
        struct P; relation R(a:i32,b:i32); relation S(c:i32,d:i32);
        answer(x,z) :- R(x,y), S(y,z);
    };
    let options = cq_plan::PlanningOptions {
        strategy: cq_plan::JoinPolicy::Auto,
        max_arity: None,
    };
    let (target, report) = cq_plan::prefix::explain(&source, options).unwrap();
    assert_eq!(cq_plan::prefix::replay(&source, &report).unwrap(), target);
    cq_plan::prefix::check(&source, &target, &report).unwrap();

    // The rule sequence still emits valid RA; inconsistent proof metadata must fail.
    let mut wrong_width = report.clone();
    wrong_width.peak_arity += 1;
    assert!(cq_plan::prefix::check(&source, &target, &wrong_width).is_err());
    assert!(cq_plan::prefix::replay(&source, &wrong_width).is_err());

    let changed: cq::Module = syn::parse_quote! {
        struct P; relation R(a:i32,b:i32); relation S(c:i32,d:i32);
        answer(x) :- R(x,y), S(y,z);
    };
    assert!(cq_plan::prefix::check(&changed, &target, &report).is_err());
    let malformed: cq::Module = syn::parse_quote! {
        struct P; relation R(a:i32,b:i32); relation S(c:i32,d:i32);
        answer(unbound) :- R(x,y), S(y,z);
    };
    assert!(cq_plan::prefix::replay(&malformed, &report).is_err());
}
