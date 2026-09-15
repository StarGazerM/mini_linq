//! Hygienic compiler-local names for expression-preserving CQ lowering.
use cq_ir::{cq, symbol_name};
use proc_macro2::{TokenStream, TokenTree};
use std::collections::BTreeSet;

pub(in crate::prefix) fn reserved(source: &cq::Module) -> BTreeSet<String> {
    use quote::ToTokens;
    fn visit(tokens: TokenStream, names: &mut BTreeSet<String>) {
        for token in tokens {
            match token {
                TokenTree::Ident(id) => {
                    names.insert(symbol_name(&id));
                }
                TokenTree::Group(g) => visit(g.stream(), names),
                _ => (),
            }
        }
    }
    let mut names = BTreeSet::new();
    visit(source.to_token_stream(), &mut names);
    names
}

pub(in crate::prefix) fn fresh(
    names: &mut BTreeSet<String>,
    clause: usize,
    arg: usize,
) -> proc_macro2::Ident {
    let mut name = format!("__cq_{clause}_{arg}");
    while !names.insert(name.clone()) {
        name.push('_');
    }
    quote::format_ident!("{name}")
}
