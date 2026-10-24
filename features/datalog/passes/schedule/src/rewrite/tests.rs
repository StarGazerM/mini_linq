use super::*;
use quote::{quote, ToTokens};

fn fixture() -> round::Program {
    let source = syn::parse_quote! {
        struct Mutual;
        relation E(x:i32) = (rows);
        relation A(x:i32);
        relation B(x:i32);
        output A;
        A(*x) :- E(x);
        A(*x) :- B(x);
        A(*x) :- A(x);
        B(*x) :- A(x);
    };
    let strata = cq_datalog_stratify::compile(&source).unwrap();
    let incremental = cq_datalog_incremental::compile(&strata).unwrap();
    let storage = cq_datalog_storage::compile(&incremental).unwrap();
    cq_datalog_fork::compile(&storage).unwrap()
}

#[test]
fn advice_pass_cases_and_round_text_roundtrip() {
    let source = fixture();
    assert_eq!(source.context.strata.len(), 1);
    assert_eq!(source.context.strata[0].seeds.len(), 1);
    assert_eq!(source.context.strata[0].steps.len(), 3);
    for advice in [quote!(whole), quote!(per_hole), quote!(by_head),
        quote!(strata(0 { seed [0] step [0] [1 2] }))] {
        let target = cq_datalog_testkit::PassCase {
            source: source.clone(),
            expected_next: None,
            compile: Box::new(move |source| compile_with(source, advice.clone())),
            check: Box::new(check),
            mutations: vec![],
            answers: None,
        }.run(None).unwrap();
        let parsed: round::Program = syn::parse2(target.to_token_stream()).unwrap();
        let reparsed: round::Program = syn::parse2(parsed.to_token_stream()).unwrap();
        assert_eq!(target, parsed);
        assert_eq!(parsed, reparsed);
    }
    assert_eq!(compile_with(&source, quote!(strata())).unwrap(), source);
}

#[test]
fn rejects_invalid_advice_and_wave_partitions() {
    let source = fixture();
    for advice in [
        quote!(unknown), quote!(whole extra),
        quote!(strata(0 { seed [0 0] step [0 1 2] })),
        quote!(strata(0 { seed [0] step [0 1] })),
        quote!(strata(0 { step [0 1 2] seed [0] })),
        quote!(strata(0 { seed [9] step [0 1 2] })),
        quote!(strata(9 { seed [0] step [0 1 2] })),
        quote!(strata(0 {}, 0 {})),
    ] {
        assert!(compile_with(&source, advice.clone()).is_err(), "{advice}");
    }
}

#[test]
fn wf_dl_schedule_rejects_hand_edited_wave_list() {
    let mut target = fixture();
    target.schedule.strata[0].waves[0].holes.push(0);
    assert!(round::contract::check(&target).is_err());
}

#[test]
fn checker_rejects_otherwise_admitted_non_schedule_edits() {
    let source = fixture();
    let mut target = source.clone();
    target.holes[0].pipeline = syn::parse_quote!(pipeline { other::printer });
    assert!(check(&source, &target).is_err());
}
