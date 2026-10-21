//! Move a captured body into a separately partitioned runtime monomorphization.
use crate::RoundContext;
use proc_macro2::{Group, Ident, Span, TokenStream, TokenTree};
use quote::{format_ident, quote};
use std::collections::{BTreeMap, BTreeSet};

fn identifiers(tokens: TokenStream, names: &mut BTreeSet<String>) {
    for token in tokens {
        match token {
            TokenTree::Ident(name) => {
                names.insert(name.to_string());
            }
            TokenTree::Group(group) => identifiers(group.stream(), names),
            _ => {}
        }
    }
}

// Storage hosts are compiler-owned identifiers, distinct from CQ relation names,
// types, and binders. Substitution also traverses the still-unexpanded CQ macro:
// its host expressions are opaque to syn's Rust expression visitor.
fn substitute(tokens: TokenStream, names: &BTreeMap<String, Ident>) -> TokenStream {
    tokens
        .into_iter()
        .flat_map(|token| match token {
            TokenTree::Ident(name) if names.contains_key(&name.to_string()) => {
                let parameter = &names[&name.to_string()];
                quote!((*#parameter))
            }
            TokenTree::Group(group) => {
                let mut replacement =
                    Group::new(group.delimiter(), substitute(group.stream(), names));
                replacement.set_span(group.span());
                TokenStream::from(TokenTree::Group(replacement))
            }
            token => TokenStream::from(token),
        })
        .collect()
}

pub(crate) fn split(
    body: TokenStream,
    context: &RoundContext,
    slot: usize,
    mutable: &BTreeSet<String>,
    locals: &BTreeSet<String>,
) -> TokenStream {
    let mut used = BTreeSet::new();
    identifiers(body.clone(), &mut used);
    let hosts = context
        .relations
        .iter()
        .flat_map(|relation| {
            [&relation.all, &relation.seen, &relation.next]
                .into_iter()
                .chain(relation.delta_host.iter())
        })
        .chain(
            context
                .layouts
                .iter()
                .chain(&context.delta_layouts)
                .map(|layout| &layout.host),
        );
    let mut names = BTreeMap::new();
    let mut arguments = Vec::new();
    let mut parameters = Vec::new();
    for host in hosts {
        let name = host.to_string();
        if !used.contains(&name) || locals.contains(&name) {
            continue;
        }
        let parameter = format_ident!(
            "__cq_codegen_{}_{}",
            slot,
            parameters.len(),
            span = Span::mixed_site()
        );
        arguments.push(if mutable.contains(&name) {
            quote!(&mut #host)
        } else {
            quote!(&#host)
        });
        parameters.push(parameter.clone());
        names.insert(name, parameter);
    }
    let body = substitute(body, &names);
    let module = format_ident!("partition_{}", slot % 16);
    quote!({ ::cq_datalog::__codegen::#module::run(
        (#(#arguments,)*),
        #[inline(always)] |(#(#parameters,)*)| { #body },
    ) })
}
