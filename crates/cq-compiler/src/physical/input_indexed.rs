//! Owning input sets, shared base-column indexes, and a lazy query cursor.
//! This realization preserves final-owner transfer and full-key set reuse.

use crate::relational_to_indexes::linear::{AccessKind, InputAccesses, input_accesses};
use cq_ir::{cq, index_requirements};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use std::collections::{BTreeMap, BTreeSet};

type IndexSignature = (String, Vec<usize>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StorageConsumer {
    Rows(usize),
    Index(usize),
}

#[derive(Clone, Debug)]
struct RelationSchema {
    input_number: usize,
    column_types: Vec<syn::Type>,
}

impl RelationSchema {
    fn arity(&self) -> usize {
        self.column_types.len()
    }
}

type Schema = BTreeMap<String, RelationSchema>;

/// Lower IndexRequirements to ordinary staged Rust with one local `pull!`.
pub(super) fn compile(source: &index_requirements::Module) -> syn::Result<syn::File> {
    if source
        .indexes
        .iter()
        .any(|index| index.result_token.is_some())
    {
        return Err(syn::Error::new_spanned(
            source,
            "the input-indexed physical route accepts base-relation indexes only",
        ));
    }
    let logical = input_accesses(&source.relational_plan)?;
    let program = &source.relational_plan.program;
    let visibility = &program.visibility;
    let program_name = &program.name;
    let storage_name = format_ident!("{}Storage", program_name, span = Span::mixed_site());

    let schema = program
        .inputs()
        .enumerate()
        .map(|(number, input)| {
            (
                cq_ir::symbol_name(&input.name),
                RelationSchema {
                    input_number: number,
                    column_types: input
                        .columns
                        .iter()
                        .map(|column| column.ty.clone())
                        .collect(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let variable_types = variable_types(&logical, &schema);
    let output_types = logical
        .head
        .variables()
        .map(|variable| {
            variable_types
                .get(&cq_ir::symbol_name(variable))
                .expect("the logical-plan contract requires every result variable to be bound")
                .clone()
        })
        .collect::<Vec<_>>();
    let output_type = tuple_type(&output_types);
    let available_indexes = source
        .indexes
        .iter()
        .enumerate()
        .map(|(number, index)| (index_signature(index), number))
        .collect::<BTreeMap<_, _>>();

    let input_names = program
        .inputs()
        .enumerate()
        .map(|(number, _)| local_ident("input", number))
        .collect::<Vec<_>>();
    let parameters = program
        .inputs()
        .zip(&input_names)
        .map(|(input, name)| {
            let row_type = tuple_type(
                &input
                    .columns
                    .iter()
                    .map(|column| column.ty.clone())
                    .collect::<Vec<_>>(),
            );
            quote!(
                #name: impl ::std::iter::IntoIterator<
                    Item = #row_type
                >
            )
        })
        .collect::<Vec<_>>();
    let index_layouts = source
        .indexes
        .iter()
        .enumerate()
        .map(|(number, index)| {
            let relation = schema
                .get(&cq_ir::symbol_name(&index.relation))
                .expect("IndexRequirements well-formedness supplies the relation schema");
            (
                number,
                relation.input_number,
                index.key_columns.len() == relation.arity(),
            )
        })
        .collect::<Vec<_>>();
    let full_indexes = index_layouts
        .iter()
        .filter_map(|&(index_number, input_number, full_key)| {
            full_key.then_some((input_number, index_number))
        })
        .collect::<BTreeMap<_, _>>();

    // The relational-plan walk decides which concrete Rust access each Rename
    // occurrence needs. An empty-key enumeration reuses a full-key set when one
    // is already required; otherwise row stores are numbered on first use.
    // Nonempty keys must resolve to a written index requirement.
    let mut row_numbers = BTreeMap::new();
    let mut row_inputs = Vec::new();
    let mut accesses = Vec::new();

    for occurrence in &logical.accesses {
        let atom = &occurrence.atom;
        let relation = cq_ir::symbol_name(&atom.relation);
        let relation_schema = schema
            .get(&relation)
            .expect("IndexRequirements well-formedness supplies the relation schema");
        let input_number = relation_schema.input_number;
        let arity = relation_schema.arity();
        let key_columns = &occurrence.key_columns;

        let access = if matches!(occurrence.kind, AccessKind::Enumerate) && key_columns.is_empty() {
            let variables = atom.variables();
            if let Some(&index_number) = full_indexes.get(&input_number) {
                let index = local_ident("index", index_number);
                quote! {
                    for (#(#variables,)*) in (self.#index.iter())
                }
            } else {
                let row_number = *row_numbers.entry(relation).or_insert_with(|| {
                    let number = row_inputs.len();
                    row_inputs.push(input_number);
                    number
                });
                let rows = local_ident("rows", row_number);
                quote! {
                    for (#(#variables,)*) in (self.#rows.iter())
                }
            }
        } else {
            let signature = (cq_ir::symbol_name(&atom.relation), key_columns.clone());
            let Some(&index_number) = available_indexes.get(&signature) else {
                return Err(missing_index_error(atom, key_columns));
            };
            let index = local_ident("index", index_number);
            let key = key_columns
                .iter()
                .map(|&column| {
                    let variable =
                        cq::rust::variable(&atom.args[column]).expect("normalized access");
                    quote!(::core::clone::Clone::clone(#variable))
                })
                .collect::<Vec<_>>();

            if matches!(occurrence.kind, AccessKind::Membership) {
                debug_assert_eq!(key_columns.len(), arity);
                quote! {
                    if (self.#index.contains(&(#(#key,)*)))
                }
            } else {
                let key_set = key_columns.iter().copied().collect::<BTreeSet<_>>();
                let payload = atom
                    .variables()
                    .enumerate()
                    .filter_map(|(column, variable)| {
                        (!key_set.contains(&column)).then_some(variable)
                    })
                    .collect::<Vec<_>>();
                quote! {
                    for (#(#payload,)*) in (
                        self.#index
                            .get(&(#(#key,)*))
                            .into_iter()
                            .flatten()
                    )
                }
            }
        };

        accesses.push(quote!(#atom => #access));
    }

    // Choose exactly one ownership-taking storage consumer per normalized
    // relation. Full-key sets reuse the normalization representation. Without
    // one, a row store takes priority; otherwise the last proper-key index
    // consumes the set. Every other required representation borrows it first.
    let final_consumers = index_layouts
        .iter()
        .filter_map(|&(index_number, input_number, full_key)| {
            (!full_key).then_some((input_number, StorageConsumer::Index(index_number)))
        })
        .chain(
            row_inputs
                .iter()
                .enumerate()
                .map(|(row_number, &input_number)| {
                    (input_number, StorageConsumer::Rows(row_number))
                }),
        )
        .chain(
            index_layouts
                .iter()
                .filter_map(|&(index_number, input_number, full_key)| {
                    full_key.then_some((input_number, StorageConsumer::Index(index_number)))
                }),
        )
        .collect::<BTreeMap<_, _>>();
    let input_setup = program
        .inputs()
        .enumerate()
        .map(|(number, input)| {
            let input_name = &input_names[number];
            if final_consumers.contains_key(&number) {
                let relation_name = local_ident("relation", number);
                let row_type = tuple_type(
                    &input
                        .columns
                        .iter()
                        .map(|column| column.ty.clone())
                        .collect::<Vec<_>>(),
                );
                quote! {
                    let #relation_name:
                        ::std::collections::HashSet<
                            #row_type
                        > = #input_name.into_iter().collect();
                }
            } else {
                quote!(let _ = #input_name;)
            }
        })
        .collect::<Vec<_>>();
    let row_builders = row_inputs
        .iter()
        .enumerate()
        .map(|(row_number, &input_number)| {
            let rows = local_ident("rows", row_number);
            let relation = local_ident("relation", input_number);
            let row_type = tuple_type(
                &program
                    .inputs()
                    .nth(input_number)
                    .unwrap()
                    .columns
                    .iter()
                    .map(|column| column.ty.clone())
                    .collect::<Vec<_>>(),
            );
            debug_assert_eq!(
                final_consumers.get(&input_number),
                Some(&StorageConsumer::Rows(row_number))
            );
            quote! {
                let #rows: ::std::vec::Vec<#row_type> =
                    #relation.into_iter().collect();
            }
        })
        .collect::<Vec<_>>();
    let row_fields = row_inputs
        .iter()
        .enumerate()
        .map(|(row_number, &input_number)| {
            let rows = local_ident("rows", row_number);
            let row_type = tuple_type(
                &program
                    .inputs()
                    .nth(input_number)
                    .unwrap()
                    .columns
                    .iter()
                    .map(|column| column.ty.clone())
                    .collect::<Vec<_>>(),
            );
            quote! {
                #rows: ::std::vec::Vec<#row_type>
            }
        })
        .collect::<Vec<_>>();
    let (moving_indexes, borrowing_indexes): (Vec<_>, Vec<_>) = source
        .indexes
        .iter()
        .enumerate()
        .map(|(number, index)| {
            let relation = schema
                .get(&cq_ir::symbol_name(&index.relation))
                .expect("IndexRequirements well-formedness supplies the relation schema");
            let takes_ownership = final_consumers.get(&relation.input_number)
                == Some(&StorageConsumer::Index(number));
            (
                takes_ownership,
                build_index(number, index, &schema, takes_ownership),
                index.key_columns.len() == relation.arity(),
            )
        })
        .partition(|(takes_ownership, _, _)| *takes_ownership);
    let borrowing_index_builders = borrowing_indexes
        .into_iter()
        .map(|(_, builder, _)| builder)
        .collect::<Vec<_>>();
    let moving_proper_index_builders = moving_indexes
        .iter()
        .filter_map(|(_, builder, full_key)| (!full_key).then_some(builder))
        .collect::<Vec<_>>();
    let moving_full_index_builders = moving_indexes
        .iter()
        .filter_map(|(_, builder, full_key)| full_key.then_some(builder))
        .collect::<Vec<_>>();
    let index_fields = source
        .indexes
        .iter()
        .enumerate()
        .map(|(number, index)| {
            let index_name = local_ident("index", number);
            let storage_type = index_storage_type(index, &schema);
            quote!(#index_name: #storage_type)
        })
        .collect::<Vec<_>>();
    let storage_names = (0..row_inputs.len())
        .map(|number| local_ident("rows", number))
        .chain((0..source.indexes.len()).map(|number| local_ident("index", number)))
        .collect::<Vec<_>>();

    let output_atom = &logical.head;
    let output_variables = logical
        .head
        .variables()
        .map(|variable| quote!(::core::clone::Clone::clone(#variable)))
        .collect::<Vec<_>>();
    let access_plan = quote! {
        #output_atom => ((#(#output_variables,)*)) :-
            #(#accesses),*;
    };

    syn::parse2(quote! {
        #visibility struct #program_name;

        #visibility struct #storage_name {
            #(#row_fields,)*
            #(#index_fields,)*
        }

        impl #program_name {
            #visibility fn build(
                #(#parameters),*
            ) -> #storage_name {
                #(#input_setup)*
                #(#borrowing_index_builders)*
                #(#moving_proper_index_builders)*
                #(#row_builders)*
                #(#moving_full_index_builders)*

                #storage_name { #(#storage_names),* }
            }

            #visibility fn run(
                #(#parameters),*
            ) -> ::std::vec::Vec<#output_type> {
                Self::build(#(#input_names),*).materialize()
            }
        }

        impl #storage_name {
            #visibility fn query(
                &self
            ) -> impl ::std::iter::Iterator<
                Item = #output_type,
            > + '_ {
                ::mini_linq::pull! { #access_plan }
            }

            #visibility fn materialize(
                &self
            ) -> ::std::vec::Vec<#output_type> {
                let mut result = self.query().collect::<::std::vec::Vec<_>>();
                result.sort_unstable();
                result
            }
        }
    })
}

fn index_storage_type(
    index: &index_requirements::IndexRequirement,
    schema: &Schema,
) -> TokenStream {
    let relation = schema
        .get(&cq_ir::symbol_name(&index.relation))
        .expect("IndexRequirements well-formedness supplies the relation schema");
    let arity = relation.arity();

    if index.key_columns.len() == arity {
        let row_type = tuple_type(&relation.column_types);
        quote! {
            ::std::collections::HashSet<#row_type>
        }
    } else {
        let key_columns = index
            .key_columns
            .iter()
            .map(|column| column.index as usize)
            .collect::<BTreeSet<_>>();
        let key_types = key_columns
            .iter()
            .map(|&column| relation.column_types[column].clone())
            .collect::<Vec<_>>();
        let payload_types = relation
            .column_types
            .iter()
            .enumerate()
            .filter(|(column, _)| !key_columns.contains(column))
            .map(|(_, column_type)| column_type.clone())
            .collect::<Vec<_>>();
        let key_type = tuple_type(&key_types);
        let payload_type = tuple_type(&payload_types);
        quote! {
            ::std::collections::HashMap<
                #key_type,
                ::std::vec::Vec<#payload_type>
            >
        }
    }
}

fn build_index(
    number: usize,
    index: &index_requirements::IndexRequirement,
    schema: &Schema,
    takes_ownership: bool,
) -> TokenStream {
    let relation_schema = schema
        .get(&cq_ir::symbol_name(&index.relation))
        .expect("IndexRequirements well-formedness supplies the relation schema");
    let input_number = relation_schema.input_number;
    let arity = relation_schema.arity();
    let index_name = local_ident("index", number);
    let relation = local_ident("relation", input_number);
    let key_columns = index
        .key_columns
        .iter()
        .map(|column| column.index as usize)
        .collect::<Vec<_>>();

    if key_columns.len() == arity {
        debug_assert!(takes_ownership);
        let row_type = tuple_type(&relation_schema.column_types);
        return quote! {
            let #index_name:
                ::std::collections::HashSet<#row_type> =
                #relation;
        };
    }

    let columns = (0..arity)
        .map(|column| local_ident(&format!("index{number}_column"), column))
        .collect::<Vec<_>>();
    let key_columns_set = key_columns.iter().copied().collect::<BTreeSet<_>>();
    let key = key_columns
        .iter()
        .map(|&column| owned_value(&columns[column], takes_ownership))
        .collect::<Vec<_>>();
    let payload = columns
        .iter()
        .enumerate()
        .filter(|(column, _)| !key_columns_set.contains(column))
        .map(|(_, value)| owned_value(value, takes_ownership))
        .collect::<Vec<_>>();
    let storage_type = index_storage_type(index, schema);
    let rows = if takes_ownership {
        quote!(#relation.into_iter())
    } else {
        quote!(#relation.iter())
    };

    quote! {
        let #index_name: #storage_type = {
                let mut #index_name: #storage_type =
                    ::std::collections::HashMap::new();
                for (#(#columns,)*) in #rows {
                    #index_name
                        .entry((#(#key,)*))
                        .or_default()
                        .push((#(#payload,)*));
                }
                #index_name
            };
    }
}

fn variable_types(logical: &InputAccesses, schema: &Schema) -> BTreeMap<String, syn::Type> {
    let mut types = BTreeMap::new();
    for access in &logical.accesses {
        let relation = schema
            .get(&cq_ir::symbol_name(&access.atom.relation))
            .expect("the logical-plan contract requires every body relation to be declared");
        for (variable, column_type) in access.atom.variables().zip(&relation.column_types) {
            types
                .entry(cq_ir::symbol_name(variable))
                .or_insert_with(|| column_type.clone());
        }
    }
    types
}

fn tuple_type(types: &[syn::Type]) -> TokenStream {
    quote!((#(#types,)*))
}

fn owned_value(value: &syn::Ident, already_owned: bool) -> TokenStream {
    if already_owned {
        quote!(#value)
    } else {
        quote!(::core::clone::Clone::clone(#value))
    }
}

fn index_signature(index: &index_requirements::IndexRequirement) -> IndexSignature {
    (
        cq_ir::symbol_name(&index.relation),
        index
            .key_columns
            .iter()
            .map(|column| column.index as usize)
            .collect(),
    )
}

fn missing_index_error(atom: &cq::Atom, columns: &[usize]) -> syn::Error {
    let columns = columns
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    syn::Error::new_spanned(
        atom,
        format!(
            "missing required index `{}[{columns}]` for this access",
            atom.relation
        ),
    )
}

fn local_ident(prefix: &str, number: usize) -> syn::Ident {
    format_ident!("{prefix}{number}", span = Span::mixed_site())
}
