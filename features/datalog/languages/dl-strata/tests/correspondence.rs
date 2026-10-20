use cq_lang_datalog_strata::{Program, contract, check_rewrite};
use quote::ToTokens;
type Mutation = Box<dyn Fn(&mut Program)>;
const STRATIFIED: &str = include_str!("fixtures/stratified.datalog");
fn scheduled() -> Program { syn::parse_str(STRATIFIED).unwrap() }
#[test]
fn schedule_cover_and_order_mutations_are_rejected() {
    let original = scheduled();
    let mutations: Vec<Mutation> = vec![
        Box::new(|p| {
            p.components.clear();
        }),
        Box::new(|p| {
            p.components[0].clear();
        }),
        Box::new(|p| {
            p.components.pop();
        }),
        Box::new(|p| {
            p.components.reverse();
        }),
        Box::new(|p| {
            p.components[0].push(syn::parse_quote!(Reach));
        }),
        Box::new(|p| {
            p.components[0].push(syn::parse_quote!(Edge));
        }),
        Box::new(|p| {
            p.components[0].push(syn::parse_quote!(Unknown));
        }),
        Box::new(|p| {
            p.components = vec![vec![
                syn::parse_quote!(Reach),
                syn::parse_quote!(Result),
            ]];
        }),
    ];
    for (index, mutate) in mutations.into_iter().enumerate() {
        let mut program = original.clone();
        mutate(&mut program);
        assert!(contract::check(&program).is_err(), "mutation {index}");
    }
}

#[test]
fn mutual_sccs_cannot_be_split_and_independent_decisions_are_preserved() {
    let mutual: Program = syn::parse_quote! {
        struct Mutual;
        relation E(x:i32) = (e);
        relation A(x:i32); relation B(x:i32);
        output A;
        A(*x) :- E(x);
        A(*x) :- B(x);
        B(*x) :- A(x);
        strata { component [A,B]; }
    };
    contract::check(&mutual).unwrap();
    let mut split = mutual.clone();
    split.components = vec![vec![syn::parse_quote!(A)], vec![syn::parse_quote!(B)]];
    assert!(contract::check(&split).is_err());
    let mut reordered = mutual.clone();
    reordered.components[0].reverse();
    contract::check(&reordered).unwrap();
    assert!(check_rewrite(&mutual, &reordered).is_err());
}
