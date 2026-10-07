use cq_stage::Language;
use cq_testkit::{PassCase, SCENARIOS, physical, semantics};
use quote::ToTokens;
use stacked_ir::{physical_to_trace as enter, trace, trace_to_physical as leave};

#[test]
fn corpus_passes_have_exact_fixtures_correspondence_and_set_answers() {
    for scenario in SCENARIOS {
        let source = scenario.parse_physical().unwrap();
        let expected =
            syn::parse_str::<trace::Program>(&format!("{} trace {{}}", scenario.physical)).unwrap();
        let entered = PassCase {
            source: source.clone(),
            expected_next: Some(expected),
            compile: Box::new(enter::compile),
            check: Box::new(enter::check),
            mutations: vec![Box::new(|target: &trace::Program| {
                let mut changed = target.clone();
                changed.physical.program.head.relation = syn::parse_quote!(changed_answer);
                changed
            })],
            answers: Some(Box::new(|target: &trace::Program, database| {
                semantics::evaluate_ra(&cq_physical_lowering::erase(&target.physical), database)
                    .output
            })),
        }
        .run(scenario.case())
        .unwrap_or_else(|error| panic!("{}: {error}", scenario.name));

        let returned = PassCase {
            source: entered,
            expected_next: Some(source.clone()),
            compile: Box::new(leave::compile),
            check: Box::new(leave::check),
            mutations: vec![Box::new(|target: &physical::Program| {
                let mut changed = target.clone();
                changed.program.head.relation = syn::parse_quote!(changed_answer);
                changed
            })],
            answers: Some(Box::new(|target: &physical::Program, database| {
                semantics::evaluate_ra(&cq_physical_lowering::erase(target), database).output
            })),
        }
        .run(scenario.case())
        .unwrap_or_else(|error| panic!("{}: {error}", scenario.name));
        assert_eq!(
            source.to_token_stream().to_string(),
            returned.to_token_stream().to_string()
        );
    }
}

#[test]
fn all_corpus_rust_expressions_are_unchanged() {
    for scenario in SCENARIOS {
        let source = scenario.parse_physical().unwrap();
        let returned = leave::compile(&enter::compile(&source).unwrap()).unwrap();
        let reference = cq_structure_selection::compile(&source).unwrap();
        let stacked = cq_structure_selection::compile(&returned).unwrap();
        let reference = cq_physical_realization::compile(&reference).unwrap();
        let stacked = cq_physical_realization::compile(&stacked).unwrap();
        assert_eq!(reference, stacked, "{}", scenario.name);
        assert_eq!(
            reference.to_token_stream().to_string(),
            stacked.to_token_stream().to_string()
        );
    }
}

fn scan() -> physical::Program {
    SCENARIOS
        .iter()
        .find(|s| s.name == "scan")
        .unwrap()
        .parse_physical()
        .unwrap()
}

#[test]
fn pass_checks_reject_valid_changes_to_written_decisions() {
    let source = scan();
    let entered = enter::compile(&source).unwrap();
    let mut changed = entered.clone();
    changed
        .physical
        .program
        .definitions
        .last_mut()
        .unwrap()
        .mode = physical::Mode::Let(Default::default());
    changed.check().unwrap();
    // Physical permits this representation change, but the embedding passes
    // promise exact preservation of the already-written decisions.
    trace::Program::check_rewrite(&entered, &changed).unwrap();
    assert!(enter::check(&source, &changed).is_err());
    assert!(leave::check(&entered, &changed.physical).is_err());
}

#[test]
fn rust_payloads_are_opaque_and_pinned() {
    let source: physical::Program = syn::parse_quote! {
        physical {
            relation R(x: i32);
            r = pipe rename R@0 {x -> x};
            a = pipe guard r if (first(x));
            b = pipe bind a with y: i32 = (make(x));
            c = pipe guard b if (second(x, y));
            out = pipe canon project c keep {x, y};
            return out as answer(x, y);
        }
    };
    let entered = enter::compile(&source).unwrap();
    assert_eq!(leave::compile(&entered).unwrap(), source);
    for index in [1, 2, 3] {
        let mut changed = entered.clone();
        match &mut changed.physical.program.definitions[index].operator {
            physical::Operator::Guard(guard) => guard.condition = syn::parse_quote!(different(x)),
            physical::Operator::Bind(bind) => bind.expression = syn::parse_quote!(different(x)),
            _ => unreachable!(),
        }
        changed.check().unwrap();
        assert!(enter::check(&source, &changed).is_err());
        assert!(leave::check(&entered, &changed.physical).is_err());
        assert!(trace::Program::check_rewrite(&entered, &changed).is_err());
    }
}

#[test]
fn trace_parsing_and_contract_reject_invalid_programs() {
    let source = scan().to_token_stream().to_string();
    for suffix in ["", "trace { unexpected }", "trace {} trace {}"] {
        assert!(syn::parse_str::<trace::Program>(&format!("{source} {suffix}")).is_err());
    }
    let mut invalid = enter::compile(&scan()).unwrap();
    invalid.physical.program.returned = syn::parse_quote!(missing);
    assert!(invalid.check().is_err());
    assert!(trace::Program::check_rewrite(&invalid, &invalid).is_err());
    assert!(enter::compile(&invalid.physical).is_err());
    assert!(leave::compile(&invalid).is_err());
    assert!(!trace::Program::terminal());
}

#[test]
fn round_trip_preserves_union_anti_and_branch_paths() {
    let a = [(1_i32,), (2,)];
    let b = [(2_i32,), (3,)];
    let rows: std::collections::BTreeSet<_> = mini_linq::compile! {
        pipeline {physical = [stacked_ir::round_trip];}
        struct Q; relation A(x:i32)=(a); relation B(x:i32)=(b);
        answer(x) :- {{A(x)} | {A(x)}} else {B(x)};
    }
    .map(|(x,)| *x)
    .collect();
    assert_eq!(rows, std::collections::BTreeSet::from([1, 2]));
}
