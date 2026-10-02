//! Inspect where normalization and ordering place a query's expression
//! arguments: a closed or prefix-only expression becomes a binding before
//! the atom; an expression using a variable first supplied by that atom becomes
//! a bind and relational identity after it. This checks planning, not generated Rust execution.
use cq_lang_cq as cq;
use cq_lang_ra::Operator;
use cq_lang_vo as valuation;
use mini_linq_paper_overview::{boundaries, directory};
use quote::ToTokens;

fn main() {
    let source: cq::Program = syn::parse_quote! {
        struct Placement;
        relation R(a:i32,b:i32,c:i32);
        answer(x) :- R(x, f(x), g());
    };
    let directory = directory("logical-argument-placement");
    let planned = boundaries(&directory, "planned", &source, true);
    // Normalization binds g() before the atom, f(x) after it, and checks
    // the computed column by relational identity.
    let expected_vo: valuation::Program = syn::parse_quote! {
        struct Placement;
        relation R(a:i32,b:i32,c:i32);
        answer(x) :- let __cq_0_2: i32 = g(), R(x, __cq_0_1, __cq_0_2), let __cq_0_1_:i32 = f(x), if * __cq_0_1 == * __cq_0_1_;
        valuation {
            let __cq_0_2: i32 = g();
            R@1(x, __cq_0_1, __cq_0_2);
            let __cq_0_1_:i32 = f(x);
            __cq_0_1 = __cq_0_1_;
        }
    };
    assert_eq!(planned.valuation, expected_vo);
    let ra = &planned.relational;
    let g = ra
        .plan
        .definitions
        .iter()
        .position(|d| match &d.operator {
            Operator::Bind(e) => e.expression.to_token_stream().to_string().contains("g ()"),
            _ => false,
        })
        .unwrap();
    let r = ra
        .plan
        .definitions
        .iter()
        .position(|d| matches!(d.operator, Operator::Rename(_)))
        .unwrap();
    let f = ra
        .plan
        .definitions
        .iter()
        .position(|d| d.operator.to_token_stream().to_string().contains("f (x)"))
        .unwrap();
    assert!(g < r && r < f);
    println!(
        "PASS: g() is planned before input matching, f(x) after x is bound; no runtime call-count claim."
    );
}
