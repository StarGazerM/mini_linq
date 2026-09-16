//! Demand-driven nested realization of a pure named relational plan.
//!
//! Each recursive expression enumerates the support of its relation with
//! borrowed field tuples. Reused definitions are rescanned, not materialized.
//! Only the returned iterator owns a distinctness set, containing borrowed
//! answer tuples; constructing the iterator does not enumerate input rows.
//! Providers must expose stable borrowed payloads through `scan` and `field`.

use crate::nested_support;
use cq_ir::{relational_plan as ra, symbol_name};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use std::collections::BTreeMap;

/// Emit a query host whose `iter` method borrows every input and output field.
pub fn compile(
    source: &ra::Module,
    inputs: &BTreeMap<String, syn::Path>,
) -> syn::Result<TokenStream> {
    let infos = nested_support::check(source, inputs)?;
    let arguments = nested_support::arguments(source);
    let mut lowering = Lowering {
        source,
        inputs,
        arguments,
        infos,
        next: 0,
    };
    let rows = lowering.relation(&source.plan.output.input);
    let output_heading = lowering.heading(&source.plan.output.input);
    let row = lowering.fresh("output_row");
    let fields = source.plan.output.head.variables().map(|variable| {
        let column = position(&output_heading, &symbol_name(variable));
        quote!(#row.#column)
    });
    let lifetime = syn::Lifetime::new("'__cq_input", Span::mixed_site());
    let seen = lowering.fresh("seen");
    let answer = lowering.fresh("answer");
    let parameters = nested_support::parameters(source, inputs, &lowering.arguments, &lifetime);
    let output_type = nested_support::output_type(
        source,
        &lowering.infos[&symbol_name(&source.plan.output.input)],
        &lifetime,
    )?;
    let (visibility, name) = (&source.program.visibility, &source.program.name);
    Ok(quote! {
        #visibility struct #name;
        impl #name {
            pub fn iter<#lifetime>(#(#parameters),*)
                -> impl ::core::iter::Iterator<Item = #output_type> + #lifetime
            {
                let mut #seen = ::std::collections::HashSet::new();
                ::core::iter::Iterator::filter_map(#rows, move |#row| {
                    let #answer = (#(#fields,)*);
                    if #seen.insert(#answer) {
                        ::core::option::Option::Some(#answer)
                    } else {
                        ::core::option::Option::None
                    }
                })
            }
        }
    })
}

struct Lowering<'a> {
    source: &'a ra::Module,
    inputs: &'a BTreeMap<String, syn::Path>,
    arguments: BTreeMap<String, syn::Ident>,
    infos: BTreeMap<String, ra::RelationInfo>,
    next: usize,
}

impl Lowering<'_> {
    fn fresh(&mut self, role: &str) -> syn::Ident {
        let number = self.next;
        self.next += 1;
        format_ident!("__cq_{role}{number}", span = Span::mixed_site())
    }

    fn heading(&self, result: &syn::Ident) -> Vec<String> {
        self.infos[&symbol_name(result)]
            .heading
            .iter()
            .cloned()
            .collect()
    }

    fn relation(&mut self, result: &syn::Ident) -> TokenStream {
        let definition = self
            .source
            .plan
            .definitions
            .iter()
            .find(|definition| symbol_name(&definition.result) == symbol_name(result))
            .expect("the checked RA names an earlier definition");
        match &definition.operator {
            ra::Operator::Unit(_) => quote!(::core::iter::once(())),
            ra::Operator::Rename(rename) => {
                let name = symbol_name(&rename.relation);
                let provider = self.inputs[&name].clone();
                let input = self.arguments[&name].clone();
                let declaration = self
                    .source
                    .program
                    .inputs()
                    .find(|input| symbol_name(&input.name) == name)
                    .unwrap();
                let row = self.fresh("row");
                let fields = self
                    .heading(result)
                    .iter()
                    .map(|name| {
                        let mapping = rename
                            .mapping
                            .iter()
                            .find(|mapping| symbol_name(&mapping.target) == *name)
                            .unwrap();
                        let column = syn::Index::from(
                            declaration
                                .columns
                                .iter()
                                .position(|column| {
                                    symbol_name(&column.name) == symbol_name(&mapping.source)
                                })
                                .unwrap(),
                        );
                        quote!(&(#provider! { field (#row) (#column) }))
                    })
                    .collect::<Vec<_>>();
                // Defer even construction of the provider traversal until demand.
                quote!(::core::iter::Iterator::flat_map(::core::iter::once(()), move |()| {
                    ::core::iter::Iterator::map(#provider! { scan (#input) }, move |#row| (#(#fields,)*))
                }))
            }
            ra::Operator::Project(project) => {
                let input = self.relation(&project.input);
                let heading = self.heading(&project.input);
                let row = self.fresh("project");
                let fields = self
                    .heading(result)
                    .iter()
                    .map(|name| {
                        let column = position(&heading, name);
                        quote!(#row.#column)
                    })
                    .collect::<Vec<_>>();
                quote!(::core::iter::Iterator::map(#input, move |#row| (#(#fields,)*)))
            }
            ra::Operator::NaturalJoin(join) => {
                let left = self.relation(&join.left);
                let right = self.relation(&join.right);
                let lh = self.heading(&join.left);
                let rh = self.heading(&join.right);
                let l = self.fresh("row");
                let r = self.fresh("row");
                let equal = equal_fields(&lh, &rh, &l, &r);
                let fields = self
                    .heading(result)
                    .iter()
                    .map(|name| {
                        if lh.contains(name) {
                            let column = position(&lh, name);
                            quote!(#l.#column)
                        } else {
                            let column = position(&rh, name);
                            quote!(#r.#column)
                        }
                    })
                    .collect::<Vec<_>>();
                quote!(::core::iter::Iterator::flat_map(#left, move |#l| {
                    ::core::iter::Iterator::filter_map(#right, move |#r| {
                        if #equal { ::core::option::Option::Some((#(#fields,)*)) }
                        else { ::core::option::Option::None }
                    })
                }))
            }
            ra::Operator::Semijoin(join) => {
                let left = self.relation(&join.left);
                let right = self.relation(&join.right);
                let lh = self.heading(&join.left);
                let rh = self.heading(&join.right);
                let l = self.fresh("row");
                let r = self.fresh("row");
                let equal = equal_fields(&lh, &rh, &l, &r);
                quote!(::core::iter::Iterator::filter_map(#left, move |#l| {
                    if ::core::iter::Iterator::any(&mut (#right), move |#r| #equal) {
                        ::core::option::Option::Some(#l)
                    } else { ::core::option::Option::None }
                }))
            }
            ra::Operator::Select(_) | ra::Operator::Extend(_) => unreachable!("domain checked"),
        }
    }
}

fn position(heading: &[String], name: &str) -> syn::Index {
    syn::Index::from(
        heading
            .iter()
            .position(|attribute| attribute == name)
            .unwrap(),
    )
}

fn equal_fields(left: &[String], right: &[String], l: &syn::Ident, r: &syn::Ident) -> TokenStream {
    let equal = left.iter().filter(|name| right.contains(name)).map(|name| {
        let li = position(left, name);
        let ri = position(right, name);
        quote!(#l.#li == #r.#ri)
    });
    quote!(true #(&& #equal)*)
}
