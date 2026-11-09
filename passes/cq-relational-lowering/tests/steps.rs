//! The seam side table: RA result name -> VO step position.
use cq_lang_vo as vo;
use cq_relational_lowering::{Steps, check, compile};
use quote::quote;

fn planned(text: &str) -> (vo::cq::Program, vo::Program) {
    let source = syn::parse_str(text).unwrap();
    let source = vo::cq::normal::normalize(&source).unwrap();
    let planned = cq_order::compile(&source, &Default::default()).unwrap();
    (source, planned)
}

fn table(entries: &[(&str, &[usize])]) -> Steps {
    Steps(
        entries
            .iter()
            .map(|(name, position)| (name.to_string(), position.to_vec()))
            .collect(),
    )
}

#[test]
fn renames_name_their_first_read_and_reductions_add_no_entry() {
    let cq: vo::cq::Program = syn::parse_quote! {
        struct Q; relation R(x:i32); relation S(x:i32);
        answer(x) :- R(x), S(x);
    };
    let source = syn::parse::Parser::parse2(
        |input: syn::parse::ParseStream<'_>| vo::Program::parse(input, &cq),
        quote! { valuation {
            reduce R@0 with S@1(x); R@0(x); S@1(x);
        }
    })
    .unwrap();
    let (target, steps) = compile(&cq, &source).unwrap();
    assert_eq!(steps, table(&[("r0", &[1]), ("r1", &[2])]));
    check(&cq, &source, &target, &steps).unwrap();
}

#[test]
fn negations_and_nested_block_arms_have_their_step_positions() {
    let (cq, source) = planned(
        "struct Q; relation R(x:i32); relation S(x:i32); answer(x) :- \
         R(x), !S(x), { S(x), { R(x) } | { S(x) } } | { S(x) };",
    );
    let (target, steps) = compile(&cq, &source).unwrap();
    let mut positions: Vec<_> = steps.0.values().cloned().collect();
    positions.sort();
    assert_eq!(
        positions,
        [
            vec![0],
            vec![1],
            vec![2],
            vec![2, 0, 0],
            vec![2, 0, 1],
            vec![2, 0, 1, 0, 0],
            vec![2, 0, 1, 1, 0],
            vec![2, 1, 0],
        ]
    );
    check(&cq, &source, &target, &steps).unwrap();
}

#[test]
fn forged_side_tables_are_rejected() {
    let (cq, source) = planned(
        "struct Q; relation R(x:i32); relation S(x:i32); answer(x) :- R(x), S(x);",
    );
    let (target, steps) = compile(&cq, &source).unwrap();
    check(&cq, &source, &target, &steps).unwrap();
    // Swapped entries name the wrong occurrence's rename.
    assert!(check(&cq, &source, &target, &table(&[("r0", &[1]), ("r1", &[0])])).is_err());
    // A rename without a step, and a step without a result.
    assert!(check(&cq, &source, &target, &table(&[("r0", &[0])])).is_err());
    // Two results for one step.
    assert!(check(&cq, &source, &target, &table(&[("r0", &[0]), ("r1", &[0])])).is_err());
    // A non-rename result.
    assert!(check(&cq, &source, &target, &table(&[("r0", &[0]), ("r2", &[1])])).is_err());
}

#[test]
fn total_lowering_requires_checked_valuation() {
    let (cq, source) = planned("struct Q; relation R(x:i32); answer(x) :- R(x);");
    let checked = cq_relational_lowering::Checked::new(&cq, &source).unwrap();
    assert_eq!(cq_relational_lowering::lower(&checked), compile(&cq, &source).unwrap());
    let mut invalid = source.clone();
    invalid.valuation.steps.clear();
    assert!(cq_relational_lowering::Checked::new(&cq, &invalid).is_err());
}

#[test]
fn repeated_head_variables_do_not_repeat_reference_projection_columns() {
    let (cq, source) = planned("struct Q; relation R(x:i32); answer(x,x) :- R(x);");
    let (target, steps) = compile(&cq, &source).unwrap();
    check(&cq, &source, &target, &steps).unwrap();
}
