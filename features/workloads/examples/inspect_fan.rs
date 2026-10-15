//! Inspect the smallest-candidate rewrite on one triangle query.
//! From features/workloads: cargo run -p cq-workloads --example inspect_fan

use cq_workloads::pipelines::{
    Pipeline,
    ir::{Choice, Operator},
};
use quote::ToTokens;

fn main() -> syn::Result<()> {
    let source = syn::parse_quote! {
        struct Triangle;
        relation r(a: i32, b: i32);
        relation s(b: i32, c: i32);
        relation t(a: i32, c: i32);
        answer(a, b, c) :- r(a, b), s(b, c), t(a, c);
    };
    // Written/fine planning exposes candidate probes. Canonicalize their
    // prefixes before recording which candidate can drive each intersection.
    let pipeline = Pipeline {
        plan: 1, // written, fine
        canon: true,
        smallest: true,
        ..Default::default()
    };
    let physical = pipeline.physical(&source)?;
    println!("{}", physical.to_token_stream());

    // Inspect the recorded decision, rather than inferring it from timing.
    let smallest_choices = physical
        .program
        .definitions
        .iter()
        .filter(|definition| match &definition.operator {
            Operator::Probe(_, probe) => matches!(probe.choice, Some(Choice::Smallest(_))),
            _ => false,
        })
        .count();
    println!("active smallest choices: {smallest_choices}");
    Ok(())
}
