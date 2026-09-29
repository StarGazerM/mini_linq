//! Inspect where normalization and ordering place a query's expression
//! arguments: a closed expression before the atom, one over a bound variable
//! after it. This is a planning check, not an observation of generated Rust
//! execution.
use cq_ir::{cq, relational_plan::Operator, valuation};
use mini_linq_paper_overview::{boundaries, directory};
use quote::ToTokens;

fn main() {
    let source: cq::Module = syn::parse_quote! {
        struct Placement;
        relation R(a:i32,b:i32,c:i32);
        answer(x) :- R(x, f(x), g());
    };
    let directory = directory("logical-argument-placement");
    let planned = boundaries(&directory, "planned", &source, true);
    // Normalization binds the closed `g()` before the atom and guards the
    // bound `f(x)` after it; written order keeps both where they are.
    let expected_vo: valuation::Module = syn::parse_quote! {
        struct Placement;
        relation R(a:i32,b:i32,c:i32);
        answer(x) :- let __cq_0_2: i32 = g(), R(x, __cq_0_1, __cq_0_2), if __cq_0_1.eq(&(f(x)));
        valuation {
            let __cq_0_2: i32 = g();
            R@1(x, __cq_0_1, __cq_0_2);
            if __cq_0_1.eq(&(f(x)));
        }
    };
    assert_eq!(planned.valuation, expected_vo);
    let ra = &planned.relational;
    let g = ra
        .plan
        .definitions
        .iter()
        .position(|d| match &d.operator {
            Operator::Extend(e) => e.value.to_token_stream().to_string().contains("g ()"),
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
