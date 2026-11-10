use cq_lang_cq as cq;
use cq_planning::{PlanOptions, prefix_valuation};
use quote::{ToTokens, quote};

const EVIDENCE: &str = include_str!("../../../runtime/cq-evidence/tests/fixtures/planner-evidence.json");

fn source(attached: bool) -> cq::Program {
    let attachment = if attached {
        quote!(declared_evidence #EVIDENCE { R = "R" full [a = "a", b = "b"]; })
    } else {
        quote!()
    };
    syn::parse2(quote! {
        struct Query;
        #attachment
        relation R(a:i32,b:i32);
        relation S(b:i32,c:i32);
        answer(a,c) :- R(a,b), S(b,c);
    }).unwrap()
}

#[test]
fn incomplete_statistics_keep_default_and_explicit_orders() {
    for options in ["", "order(S@1,R@0)"] {
        let args: PlanOptions = syn::parse_str(options).unwrap();
        let (_, before) = prefix_valuation(&source(false), &args).unwrap();
        let (normal, after) = prefix_valuation(&source(true), &args).unwrap();
        assert_eq!(before.valuation, after.valuation);
        assert!(normal.declared_evidence.is_some());
        let reparsed = syn::parse::Parser::parse2(
            |input: syn::parse::ParseStream| cq_lang_vo::Program::parse(input, &normal),
            after.to_token_stream(),
        ).unwrap();
        assert_eq!(after, reparsed);
        cq_lang_vo::contract::check(&normal, &reparsed).unwrap();
    }
}

#[test]
fn attachment_is_preserved_in_the_separate_normalized_source() {
    let source = source(true);
    let normalized = cq::normal::normalize(&source).unwrap();
    assert_eq!(normalized.declared_evidence, source.declared_evidence);
    let (recorded, plan) = prefix_valuation(&source, &PlanOptions::default()).unwrap();
    assert_eq!(recorded, normalized);
    cq_order::check(&recorded, &plan, &cq_order::Options {
        heuristic: cq_order::Heuristic::Prefix(cq_order::Ranking::Connected),
        split: cq_order::Split::Coarse,
    }).unwrap();
}

#[test]
fn invalid_binding_is_not_treated_as_absent_evidence() {
    for bindings in [
        quote!(Missing = "R" full [a="a",b="b"];),
        quote!(R = "Missing" full [a="a",b="b"];),
        quote!(R = "R" full [a="a"];),
        quote!(R = "R" full [a="a",a="b"];),
        quote!(R = "R" full [a="a",b="a"];),
        quote!(R = "R" full [a="a",b="missing"];),
        quote!(R = "R" full [a="a",b="b"]; R = "R" full [a="a",b="b"];),
    ] {
        let program: cq::Program = syn::parse2(quote! {
            struct Query;
            declared_evidence #EVIDENCE { #bindings }
            relation R(a:i32,b:i32);
            answer(a,b) :- R(a,b);
        }).unwrap();
        assert!(cq::contract::check(&program).is_err(), "accepted {bindings}");
        assert!(prefix_valuation(&program, &PlanOptions::default()).is_err());
    }
}

#[test]
fn malformed_document_is_rejected_at_the_input_boundary() {
    for json in ["{}", "not json", "{\"schema\":\"bad\",\"schema\":\"bad\"}"] {
        assert!(syn::parse2::<cq::Program>(quote! {
            struct Query;
            declared_evidence #json { R = "R" full [a="a",b="b"]; }
            relation R(a:i32,b:i32);
            answer(a,b) :- R(a,b);
        }).is_err());
    }
}
