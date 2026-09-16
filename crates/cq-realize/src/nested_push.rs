//! Whole-RA nested-loop production over borrowed input fields.
//!
//! This pure fragment unfolds the named RA DAG into nested Rust control flow.
//! Projection changes the produced binding, joins compare shared attributes,
//! and semijoins stop their witness traversal at the first match. Duplicate
//! derivations are retained internally and eliminated at the final consumer;
//! this is valid for the admitted positive algebra without Rust effect leaves.

use crate::nested_support;
use cq_ir::{cq, relational_plan as ra, symbol_name};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use std::{cell::Cell, collections::BTreeMap};

/// Compile a pure RA program into `for_each`, borrowing each output field.
///
/// Input providers must supply `Type`, `Scan`, and `Field`; their scans must
/// yield stable field borrows tied to the caller's input lifetime. Rust checks
/// that requirement against each selected provider's expansion.
pub fn compile(
    source: &ra::Module,
    inputs: &BTreeMap<String, syn::Path>,
) -> syn::Result<TokenStream> {
    let information = nested_support::check(source, inputs)?;
    let declarations = source
        .program
        .inputs()
        .map(|input| (symbol_name(&input.name), input))
        .collect::<BTreeMap<_, _>>();
    let output_columns = source
        .plan
        .output
        .head
        .variables()
        .map(symbol_name)
        .collect::<Vec<_>>();
    let lifetime = syn::Lifetime::new("'__cq_data", Span::mixed_site());
    let output_type = nested_support::output_type(
        source,
        &information[&symbol_name(&source.plan.output.input)],
        &lifetime,
    )?;
    let arguments = nested_support::arguments(source);
    let parameters = nested_support::parameters(source, inputs, &arguments, &lifetime);
    let producer = Producer {
        declarations,
        definitions: source
            .plan
            .definitions
            .iter()
            .map(|definition| (symbol_name(&definition.result), &definition.operator))
            .collect(),
        arguments: &arguments,
        inputs,
        sequence: Cell::new(0),
    };
    let seen = producer.local("seen");
    let consumer = producer.local("consumer");
    let output = producer.local("output");
    let body = producer.produce(&source.plan.output.input, &|bindings| {
        let fields = output_columns.iter().map(|name| &bindings[name]);
        Ok(quote! {
            let #output: #output_type = (#(#fields,)*);
            if #seen.insert(#output) {
                #consumer(#output);
            }
        })
    })?;
    let visibility = &source.program.visibility;
    let program = &source.program.name;
    Ok(quote! {
        #visibility struct #program;
        impl #program {
            pub fn for_each<#lifetime>(
                #(#parameters,)*
                mut #consumer: impl ::core::ops::FnMut(#output_type),
            ) {
                let mut #seen: ::std::collections::HashSet<#output_type> =
                    ::std::collections::HashSet::new();
                #body
            }
        }
    })
}

// These are compiler-local expressions for borrowed fields, not another IR.
type Bindings = BTreeMap<String, TokenStream>;

struct Producer<'a> {
    declarations: BTreeMap<String, &'a cq::RelationDecl>,
    definitions: BTreeMap<String, &'a ra::Operator>,
    arguments: &'a BTreeMap<String, syn::Ident>,
    inputs: &'a BTreeMap<String, syn::Path>,
    sequence: Cell<usize>,
}

impl Producer<'_> {
    fn local(&self, role: &str) -> syn::Ident {
        let index = self.sequence.get();
        self.sequence.set(index + 1);
        format_ident!("__cq_{role}{index}", span = Span::mixed_site())
    }

    fn produce(
        &self,
        result: &syn::Ident,
        consumer: &dyn Fn(&Bindings) -> syn::Result<TokenStream>,
    ) -> syn::Result<TokenStream> {
        match self.definitions[&symbol_name(result)] {
            ra::Operator::Unit(_) => consumer(&Bindings::new()),
            ra::Operator::Rename(rename) => {
                let input = symbol_name(&rename.relation);
                let declaration = self.declarations[&input];
                let argument = &self.arguments[&input];
                let provider = &self.inputs[&input];
                let row = self.local("row");
                let bindings = rename
                    .mapping
                    .iter()
                    .map(|mapping| {
                        let column = declaration
                            .columns
                            .iter()
                            .position(|column| {
                                symbol_name(&column.name) == symbol_name(&mapping.source)
                            })
                            .expect("the RA contract checked the rename mapping");
                        let column = syn::Index::from(column);
                        (
                            symbol_name(&mapping.target),
                            quote!(&(#provider! { field (#row) (#column) })),
                        )
                    })
                    .collect();
                let body = consumer(&bindings)?;
                Ok(quote! {
                    for #row in #provider! { scan (#argument) } {
                        #body
                    }
                })
            }
            ra::Operator::Project(project) => self.produce(&project.input, &|bindings| {
                let projected = project
                    .attributes
                    .iter()
                    .map(|attribute| {
                        let name = symbol_name(attribute);
                        (name.clone(), bindings[&name].clone())
                    })
                    .collect();
                consumer(&projected)
            }),
            ra::Operator::NaturalJoin(join) => self.produce(&join.left, &|left| {
                self.produce(&join.right, &|right| {
                    let comparisons = comparisons(left, right);
                    let mut joined = right.clone();
                    joined.extend(left.clone());
                    let body = consumer(&joined)?;
                    if comparisons.is_empty() {
                        Ok(body)
                    } else {
                        Ok(quote! { if #(#comparisons)&&* { #body } })
                    }
                })
            }),
            ra::Operator::Semijoin(join) => self.produce(&join.left, &|left| {
                let label =
                    syn::Lifetime::new(&format!("'{}", self.local("witness")), Span::mixed_site());
                let witness = self.produce(&join.right, &|right| {
                    let comparisons = comparisons(left, right);
                    let matches = if comparisons.is_empty() {
                        quote!(true)
                    } else {
                        quote!(#(#comparisons)&&*)
                    };
                    // Keep the conditional for the empty separator too: even
                    // a Unit witness has an ordinary fallthrough expression.
                    Ok(quote! { if #matches { break #label true; } })
                })?;
                let body = consumer(left)?;
                Ok(quote! {
                    if #label: {
                        #witness
                        false
                    } {
                        #body
                    }
                })
            }),
            ra::Operator::Select(_) | ra::Operator::Extend(_) => {
                unreachable!("unsupported operators were rejected before emission")
            }
        }
    }
}

fn comparisons(left: &Bindings, right: &Bindings) -> Vec<TokenStream> {
    left.iter()
        .filter_map(|(name, left)| right.get(name).map(|right| quote!((#left) == (#right))))
        .collect()
}
