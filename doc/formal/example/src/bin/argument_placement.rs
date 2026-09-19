//! Inspect A-Prefix versus A-Local in the actual logical derivation.
//! This is a planning check, not an observation of generated Rust execution.
use cq_ir::{cq, relational_plan::Operator};
use cq_query_lowering::{JoinPolicy, PlanningOptions, Rule, composition};
use mini_linq_paper_overview::{boundaries, directory};
use quote::ToTokens;

fn main() {
    let source: cq::Module = syn::parse_quote! {
        struct Placement;
        relation R(a:i32,b:i32,c:i32);
        answer(x) :- R(x, f(x), g());
    };
    let (ra, report) = composition::explain_cq_plan(
        &source,
        PlanningOptions {
            strategy: JoinPolicy::Auto,
            max_arity: None,
        },
    )
    .unwrap();
    composition::check_cq_plan(&source, &ra, &report).unwrap();
    assert!(
        report
            .derivation
            .iter()
            .any(|rule| matches!(rule, Rule::Match { clause: 0 }))
    );
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
    let directory = directory("logical-argument-placement");
    boundaries(&directory, "planned", &source, true);
    println!(
        "PASS: g() is planned before input matching, f(x) after x is bound; no runtime call-count claim."
    );
}
