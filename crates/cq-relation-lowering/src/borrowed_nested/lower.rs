//! Resolve RA operators to existing Access syntax, without emitting execution.
use super::Options;
use cq_ir::{relational_plan as ra, rust_access_plan as access, symbol_name};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct Lowering<'a> {
    source: &'a ra::Module,
    options: &'a Options,
    arguments: BTreeMap<String, syn::Ident>,
    information: BTreeMap<String, ra::RelationInfo>,
    bindings: BTreeMap<String, syn::Ident>,
    reusable: BTreeSet<String>,
    next: usize,
}

impl<'a> Lowering<'a> {
    pub fn new(
        source: &'a ra::Module,
        options: &'a Options,
        arguments: BTreeMap<String, syn::Ident>,
        information: BTreeMap<String, ra::RelationInfo>,
    ) -> Self {
        Self {
            source,
            options,
            arguments,
            information,
            bindings: BTreeMap::new(),
            reusable: reusable_definitions(source),
            next: 0,
        }
    }

    fn fresh(&mut self, role: &str) -> syn::Ident {
        let number = self.next;
        self.next += 1;
        format_ident!("__cq_{role}{number}", span = Span::mixed_site())
    }

    fn heading(&self, result: &syn::Ident) -> Vec<String> {
        self.information[&symbol_name(result)]
            .heading
            .iter()
            .cloned()
            .collect()
    }

    /// Capture an owning resource handle before a deferred Access region. A
    /// single-use stream instead moves into that region without a replay owner.
    fn input(&mut self, result: &syn::Ident, setup: &mut TokenStream) -> TokenStream {
        let name = symbol_name(result);
        let binding = self.bindings[&name].clone();
        if self.reusable.contains(&name) {
            let handle = self.fresh("capture");
            setup.extend(quote!(let #handle = ::core::clone::Clone::clone(&#binding);));
            let replay = &self.options.replay;
            quote!(#replay! { scan (#handle) })
        } else {
            quote!(#binding)
        }
    }

    pub fn definitions(&mut self) -> syn::Result<TokenStream> {
        let mut definitions = TokenStream::new();
        for definition in &self.source.plan.definitions {
            let name = symbol_name(&definition.result);
            let (setup, plan) = self.region(definition)?;
            // BS-Retain: a shared or restarted region builds a selected resource.
            let value = if self.reusable.contains(&name) {
                let replay = &self.options.replay;
                quote!(#replay! { build { #plan } })
            } else {
                // BS-Stream: one consumer owns a support-stream region.
                let stream = &self.options.stream;
                quote!(#stream! { #plan })
            };
            let binding = self.fresh("relation");
            definitions.extend(quote!(let #binding = { #setup #value };));
            self.bindings.insert(name, binding);
        }
        Ok(definitions)
    }

    pub fn output(&mut self) -> syn::Result<TokenStream> {
        let mut setup = TokenStream::new();
        let result = &self.source.plan.output.input;
        let rows = self.input(result, &mut setup);
        let heading = self.heading(result);
        let row = self.fresh("answer");
        let fields = self.source.plan.output.head.variables().map(|variable| {
            let column = position(&heading, &symbol_name(variable));
            quote!(#row.#column)
        });
        let plan = checked(quote! {
            out(#row) => ((#(#fields,)*)) :-
                Rows(#row) => for #row in (#rows);
        })?;
        // BS-Output: the answer region uses its distinct-iterator continuation.
        let output = &self.options.output;
        Ok(quote!({ #setup #output! { #plan } }))
    }

    fn region(&mut self, definition: &ra::Definition) -> syn::Result<(TokenStream, access::Plan)> {
        let mut setup = TokenStream::new();
        let result = &definition.result;
        let row = self.fresh("row");
        let plan = match &definition.operator {
            // BS-Unit: resolve this RA operator to a checked Access region.
            ra::Operator::Unit(_) => quote!(out() => (()) :- ;),
            // BS-Rename: resolve this RA operator to a checked Access region.
            ra::Operator::Rename(rename) => {
                let name = symbol_name(&rename.relation);
                let provider = &self.options.inputs[&name];
                let input = &self.arguments[&name];
                let declaration = self
                    .source
                    .program
                    .inputs()
                    .find(|input| symbol_name(&input.name) == name)
                    .unwrap();
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
                quote! {
                    out(#row) => ((#(#fields,)*)) :-
                        Rows(#row) => for #row in (#provider! { scan (#input) });
                }
            }
            // BS-Project: resolve this RA operator to a checked Access region.
            ra::Operator::Project(project) => {
                let rows = self.input(&project.input, &mut setup);
                let heading = self.heading(&project.input);
                let fields = self
                    .heading(result)
                    .iter()
                    .map(|name| {
                        let column = position(&heading, name);
                        quote!(#row.#column)
                    })
                    .collect::<Vec<_>>();
                quote! {
                    out(#row) => ((#(#fields,)*)) :-
                        Rows(#row) => for #row in (#rows);
                }
            }
            // BS-Join: resolve this RA operator to a checked Access region.
            ra::Operator::NaturalJoin(join) => {
                let left = self.input(&join.left, &mut setup);
                let right = self.input(&join.right, &mut setup);
                let lh = self.heading(&join.left);
                let rh = self.heading(&join.right);
                let other = self.fresh("row");
                let equal = equal_fields(&lh, &rh, &row, &other);
                let fields = self
                    .heading(result)
                    .iter()
                    .map(|name| {
                        if lh.contains(name) {
                            let column = position(&lh, name);
                            quote!(#row.#column)
                        } else {
                            let column = position(&rh, name);
                            quote!(#other.#column)
                        }
                    })
                    .collect::<Vec<_>>();
                quote! {
                    out(#row, #other) => ((#(#fields,)*)) :-
                        Rows(#row) => for #row in (#left),
                        Rows(#other) => for #other in (#right),
                        if #equal => if (#equal);
                }
            }
            // BS-Semi: resolve this RA operator to a checked Access region.
            ra::Operator::Semijoin(join) => {
                let left = self.input(&join.left, &mut setup);
                let resource = self.bindings[&symbol_name(&join.right)].clone();
                let captured = self.fresh("capture");
                setup.extend(quote!(let #captured = ::core::clone::Clone::clone(&#resource);));
                let witness_handle = self.fresh("witness");
                let witness_row = self.fresh("row");
                let equal = equal_fields(
                    &self.heading(&join.left),
                    &self.heading(&join.right),
                    &row,
                    &witness_row,
                );
                let replay = &self.options.replay;
                let witness = checked(quote! {
                    out(#witness_row) => (()) :-
                        Rows(#witness_row) => for #witness_row in (#replay! { scan (#witness_handle) }),
                        if #equal => if (#equal);
                })?;
                let exists = &self.options.exists;
                let condition = quote!({
                    let #witness_handle = ::core::clone::Clone::clone(&#captured);
                    #exists! { #witness }
                });
                quote! {
                    out(#row) => (#row) :-
                        Rows(#row) => for #row in (#left),
                        if #condition => if (#condition);
                }
            }
            ra::Operator::Select(_) | ra::Operator::Extend(_) => unreachable!("domain checked"),
        };
        Ok((setup, checked(plan)?))
    }
}

fn checked(tokens: TokenStream) -> syn::Result<access::Plan> {
    let plan = syn::parse2(tokens)?;
    access::contract::check(&plan)?;
    Ok(plan)
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
    let comparisons = left.iter().filter(|name| right.contains(name)).map(|name| {
        let li = position(left, name);
        let ri = position(right, name);
        quote!(#l.#li == #r.#ri)
    });
    quote!(true #(&& #comparisons)*)
}

/// Retain direct fanout and repeatedly scanned right operands. This is a
/// resource-lifetime choice; the selected Native provider implements storage.
fn reusable_definitions(source: &ra::Module) -> BTreeSet<String> {
    let mut uses = BTreeMap::<String, usize>::new();
    let mut reusable = BTreeSet::new();
    for definition in &source.plan.definitions {
        let operands = match &definition.operator {
            ra::Operator::Unit(_) | ra::Operator::Rename(_) => Vec::new(),
            ra::Operator::Project(project) => vec![&project.input],
            ra::Operator::NaturalJoin(join) => {
                reusable.insert(symbol_name(&join.right));
                vec![&join.left, &join.right]
            }
            ra::Operator::Semijoin(join) => {
                reusable.insert(symbol_name(&join.right));
                vec![&join.left, &join.right]
            }
            ra::Operator::Select(_) | ra::Operator::Extend(_) => unreachable!("domain checked"),
        };
        for input in operands {
            *uses.entry(symbol_name(input)).or_default() += 1;
        }
    }
    *uses
        .entry(symbol_name(&source.plan.output.input))
        .or_default() += 1;
    reusable.extend(
        uses.into_iter()
            .filter_map(|(name, uses)| (uses > 1).then_some(name)),
    );
    reusable
}
