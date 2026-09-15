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
        if !source.program.query.is_basic() {
            continue;
        }
        let target = cq_plan::canonical::compile(&source).unwrap();
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
fn public_boundary_rejects_wrong_source_and_unsupported_clauses() {
    let source: cq::Module = syn::parse_quote! {
        struct P; relation R(a:i32,b:i32); answer(x) :- R(x,y);
    };
    let target = cq_plan::canonical::compile(&source).unwrap();
    cq_plan::canonical::check(&source, &target).unwrap();
    let changed: cq::Module = syn::parse_quote! {
        struct P; relation R(a:i32,b:i32); answer(y) :- R(x,y);
    };
    assert!(cq_plan::canonical::check(&changed, &target).is_err());
    let unsupported: cq::Module = syn::parse_quote! {
        struct P; relation R(a:i32,b:i32); answer(x) :- R(x,y), if x > 0;
    };
    assert!(cq_plan::canonical::compile(&unsupported).is_err());
}
