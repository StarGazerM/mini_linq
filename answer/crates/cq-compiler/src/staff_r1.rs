//! Staff solution for the three basic homework lowerings.
//!
//! This module deliberately carries only information that is not yet Rust.
//! The third lowering emits concrete collection types, builders, names, and
//! the query API as ordinary Rust syntax. Its one remaining staged fragment is
//! the occurrence-annotated Rust access plan consumed by the supplied pull
//! strategy. That strategy returns one lazy iterator for the whole result set;
//! only the generated materializer drains it into an output collection.

use std::collections::{BTreeMap, BTreeSet};

use cq_ir::{cq, index_requirements, relational_plan};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};

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

/// Lower one positive CQ to the canonical named left-deep relational plan.
pub fn compile_cq(source: &cq::Module) -> syn::Result<relational_plan::Module> {
    let declarations = source
        .program
        .inputs
        .iter()
        .map(|input| (cq_ir::symbol_name(&input.name), input))
        .collect::<BTreeMap<_, _>>();
    let mut definitions = Vec::<TokenStream>::new();
    let mut next_result = 0usize;
    let mut current = None::<syn::Ident>;

    for item in &source.program.query.body {
        let cq::BodyItem::Positive { atom } = item;
        let declaration = declarations
            .get(&cq_ir::symbol_name(&atom.relation))
            .expect("the CQ contract requires every body relation to be declared");
        let rename = local_ident("r", next_result);
        next_result += 1;
        let mapping = declaration
            .columns
            .iter()
            .zip(&atom.variables)
            .map(|(column, variable)| {
                let source = &column.name;
                quote!(#source -> #variable)
            })
            .collect::<Vec<_>>();
        let relation = &atom.relation;
        definitions.push(quote! {
            #rename = rename #relation { #(#mapping),* };
        });

        if let Some(left) = current {
            let result = local_ident("r", next_result);
            next_result += 1;
            definitions.push(quote! {
                #result = natural_join #left with #rename;
            });
            current = Some(result);
        } else {
            current = Some(rename);
        }
    }

    let input = current.expect("a CQ contract requires a nonempty body");
    let project = local_ident("r", next_result);
    let head = &source.program.query.head;
    let attributes = head.variables.iter();
    definitions.push(quote! {
        #project = project #input keep { #(#attributes),* };
    });

    let program = &source.program;
    syn::parse2(quote! {
        #program
        relational {
            #(#definitions)*
            output #project as #head.
        }
    })
}

/// Preserve a relational plan and derive its canonical right-Rename keys.
pub fn compile_relational_plan(
    source: &relational_plan::Module,
) -> syn::Result<index_requirements::Module> {
    let logical = lowerable_access_sequence(source)?;
    let mut seen = BTreeSet::<IndexSignature>::new();
    let requirements = logical
        .accesses
        .iter()
        .skip(1)
        .filter_map(|access| {
            let signature = (
                cq_ir::symbol_name(&access.atom.relation),
                access.key_columns.clone(),
            );
            (!signature.1.is_empty() && seen.insert(signature))
                .then_some((&access.atom.relation, &access.key_columns))
        })
        .map(|(relation, key_columns)| {
            let columns = key_columns.iter().copied().map(syn::Index::from);
            quote!(#relation[#(#columns),*])
        })
        .collect::<Vec<_>>();

    syn::parse2(quote! {
        #source
        indexes { #(#requirements;)* }
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LogicalAccessKind {
    Enumerate,
    Membership,
}

#[derive(Clone, Debug)]
struct LogicalAccess {
    atom: cq::Atom,
    key_columns: Vec<usize>,
    kind: LogicalAccessKind,
}

#[derive(Clone, Debug)]
struct LowerableLogicalPlan {
    accesses: Vec<LogicalAccess>,
    head: cq::Atom,
}

type Heading = BTreeSet<String>;

/// Recover the left-deep access sequence from named relational dataflow.
///
/// The basic physical backend accepts one root Rename followed by NaturalJoins
/// whose right operand is a direct Rename occurrence, then one final Project.
/// NaturalJoin becomes enumeration when the right heading adds attributes and
/// membership when the right heading is already fully bound.
fn lowerable_access_sequence(
    source: &relational_plan::Module,
) -> syn::Result<LowerableLogicalPlan> {
    let declarations = source
        .program
        .inputs
        .iter()
        .map(|input| (cq_ir::symbol_name(&input.name), input))
        .collect::<BTreeMap<_, _>>();
    let mut earlier = BTreeMap::<String, (&relational_plan::Definition, Heading)>::new();
    let mut current = None::<String>;
    let mut unconsumed_renames = BTreeSet::<String>::new();
    let mut accesses = Vec::new();
    let mut projected = false;

    for definition in &source.plan.definitions {
        let result = cq_ir::symbol_name(&definition.result);
        let heading = match &definition.operator {
            // OPTIONAL HW4 — STUDENT 3/6 (LOGICAL): lower the supplied Unit
            // only as specified in doc/HW4-OPTIONAL.md.
            relational_plan::Operator::Unit(unit) => {
                return Err(unsupported_unit_error(unit));
            }
            relational_plan::Operator::Rename(rename) => {
                if projected {
                    return Err(syn::Error::new_spanned(
                        rename,
                        "the basic left-deep backend requires Rename inputs before the final Project",
                    ));
                }
                let declaration = declarations
                    .get(&cq_ir::symbol_name(&rename.relation))
                    .expect("RelationalPlan well-formedness requires a declared Rename input");
                let heading = rename_heading(rename);
                if current.is_none() {
                    current = Some(result.clone());
                    accesses.push(LogicalAccess {
                        atom: renamed_atom(rename, declaration),
                        key_columns: Vec::new(),
                        kind: LogicalAccessKind::Enumerate,
                    });
                } else {
                    unconsumed_renames.insert(result.clone());
                }
                heading
            }
            relational_plan::Operator::NaturalJoin(join) => {
                if projected {
                    return Err(syn::Error::new_spanned(
                        join,
                        "the basic left-deep backend requires NaturalJoins before the final Project",
                    ));
                }
                require_current_left("natural join", &join.left, current.as_deref())?;
                let right_name = cq_ir::symbol_name(&join.right);
                if !unconsumed_renames.remove(&right_name) {
                    return Err(syn::Error::new_spanned(
                        &join.right,
                        "the basic left-deep backend must consume each non-root Rename exactly once as a right operand",
                    ));
                }
                let (right_definition, right_heading) = earlier
                    .get(&right_name)
                    .expect("RelationalPlan well-formedness requires an earlier right operand");
                let relational_plan::Operator::Rename(rename) = &right_definition.operator else {
                    return Err(syn::Error::new_spanned(
                        &join.right,
                        "index selection and the basic left-deep backend require a direct Rename as the right operand",
                    ));
                };
                let (_, left_heading) = earlier
                    .get(&cq_ir::symbol_name(&join.left))
                    .expect("RelationalPlan well-formedness requires an earlier left operand");
                let declaration = declarations
                    .get(&cq_ir::symbol_name(&rename.relation))
                    .expect("RelationalPlan well-formedness requires a declared Rename input");
                let shared = left_heading
                    .intersection(right_heading)
                    .cloned()
                    .collect::<Heading>();
                accesses.push(LogicalAccess {
                    atom: renamed_atom(rename, declaration),
                    key_columns: relational_key_columns(rename, declaration, &shared),
                    kind: if right_heading.is_subset(left_heading) {
                        LogicalAccessKind::Membership
                    } else {
                        LogicalAccessKind::Enumerate
                    },
                });
                let mut heading = left_heading.clone();
                heading.extend(right_heading.iter().cloned());
                current = Some(result.clone());
                heading
            }
            relational_plan::Operator::Project(project) => {
                if projected {
                    return Err(syn::Error::new_spanned(
                        project,
                        "the basic left-deep backend accepts exactly one final Project",
                    ));
                }
                require_current_left("project", &project.input, current.as_deref())?;
                if !unconsumed_renames.is_empty() {
                    return Err(syn::Error::new_spanned(
                        project,
                        "the basic left-deep backend requires every non-root Rename to be the right operand of exactly one NaturalJoin",
                    ));
                }
                projected = true;
                current = Some(result.clone());
                project.attributes.iter().map(cq_ir::symbol_name).collect()
            }
        };

        earlier.insert(result, (definition, heading));
    }

    if !projected
        || current.as_deref() != Some(cq_ir::symbol_name(&source.plan.output.input).as_str())
    {
        return Err(syn::Error::new_spanned(
            &source.plan.output,
            "the basic left-deep backend requires one final Project selected by output metadata",
        ));
    }

    Ok(LowerableLogicalPlan {
        accesses,
        head: source.plan.output.head.clone(),
    })
}

fn require_current_left(
    operator: &str,
    left: &syn::Ident,
    current: Option<&str>,
) -> syn::Result<()> {
    if current.is_some_and(|current| current == cq_ir::symbol_name(left)) {
        return Ok(());
    }
    Err(syn::Error::new_spanned(
        left,
        format!(
            "the basic left-deep backend requires the {operator} left operand to be the preceding operator result"
        ),
    ))
}

fn rename_heading(rename: &relational_plan::Rename) -> Heading {
    rename
        .mapping
        .iter()
        .map(|attribute| cq_ir::symbol_name(&attribute.target))
        .collect()
}

fn renamed_atom(rename: &relational_plan::Rename, declaration: &cq::RelationDecl) -> cq::Atom {
    let targets = rename
        .mapping
        .iter()
        .map(|attribute| (cq_ir::symbol_name(&attribute.source), &attribute.target))
        .collect::<BTreeMap<_, _>>();
    let relation = &rename.relation;
    let variables = declaration.columns.iter().map(|column| {
        targets
            .get(&cq_ir::symbol_name(&column.name))
            .expect("RelationalPlan well-formedness requires a complete Rename mapping")
    });
    syn::parse2(quote!(#relation(#(#variables),*)))
        .expect("a checked Rename must reconstruct one CQ atom")
}

fn unsupported_unit_error(unit: &relational_plan::Unit) -> syn::Error {
    syn::Error::new_spanned(
        unit,
        "logical `unit` is reserved for the OPTIONAL HW4 global-clause lowering",
    )
}

fn relational_key_columns(
    rename: &relational_plan::Rename,
    declaration: &cq::RelationDecl,
    shared: &Heading,
) -> Vec<usize> {
    let targets = rename
        .mapping
        .iter()
        .map(|attribute| {
            (
                cq_ir::symbol_name(&attribute.source),
                cq_ir::symbol_name(&attribute.target),
            )
        })
        .collect::<BTreeMap<_, _>>();
    declaration
        .columns
        .iter()
        .enumerate()
        .filter_map(|(column, declaration)| {
            targets
                .get(&cq_ir::symbol_name(&declaration.name))
                .is_some_and(|target| shared.contains(target))
                .then_some(column)
        })
        .collect()
}

/// Lower IndexRequirements to ordinary staged Rust with one local `pull!`.
pub fn compile_index_requirements(source: &index_requirements::Module) -> syn::Result<syn::File> {
    let logical = lowerable_access_sequence(&source.relational_plan)?;
    let program = &source.relational_plan.program;
    let visibility = &program.visibility;
    let program_name = &program.name;
    let storage_name = format_ident!("{}Storage", program_name, span = Span::mixed_site());

    let schema = program
        .inputs
        .iter()
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
        .variables
        .iter()
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
        .inputs
        .iter()
        .enumerate()
        .map(|(number, _)| local_ident("input", number))
        .collect::<Vec<_>>();
    let parameters = program
        .inputs
        .iter()
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

        let access =
            if matches!(occurrence.kind, LogicalAccessKind::Enumerate) && key_columns.is_empty() {
                let variables = atom.variables.iter();
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
                        let variable = &atom.variables[column];
                        quote!(::core::clone::Clone::clone(#variable))
                    })
                    .collect::<Vec<_>>();

                if matches!(occurrence.kind, LogicalAccessKind::Membership) {
                    debug_assert_eq!(key_columns.len(), arity);
                    quote! {
                        if (self.#index.contains(&(#(#key,)*)))
                    }
                } else {
                    let key_set = key_columns.iter().copied().collect::<BTreeSet<_>>();
                    let payload = atom
                        .variables
                        .iter()
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
        .inputs
        .iter()
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
                &program.inputs[input_number]
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
                &program.inputs[input_number]
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
        .variables
        .iter()
        .map(|variable| quote!(::core::clone::Clone::clone(#variable)))
        .collect::<Vec<_>>();
    let access_plan = quote! {
        #output_atom => ((#(#output_variables,)*)) :-
            #(#accesses),*.
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

fn variable_types(logical: &LowerableLogicalPlan, schema: &Schema) -> BTreeMap<String, syn::Type> {
    let mut types = BTreeMap::new();
    for access in &logical.accesses {
        let relation = schema
            .get(&cq_ir::symbol_name(&access.atom.relation))
            .expect("the logical-plan contract requires every body relation to be declared");
        for (variable, column_type) in access.atom.variables.iter().zip(&relation.column_types) {
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

#[cfg(test)]
mod tests {
    use syn::visit::Visit;

    use super::*;

    #[test]
    fn triangle_relational_plan_and_index_overlay_are_locally_valid() {
        let source: cq::Module = syn::parse2(quote! {
            pub struct TriangleProgram;
            relation R(src: i32, dst: i32);
            relation S(src: i32, dst: i32);
            relation T(src: i32, dst: i32);
            triangle(x, y, z) :- R(x, y), S(y, z), T(z, x).
        })
        .unwrap();
        cq::contract::check(&source).unwrap();

        let logical = compile_cq(&source).unwrap();
        relational_plan::contract::check(&logical).unwrap();
        let indexed = compile_relational_plan(&logical).unwrap();
        index_requirements::contract::check(&indexed).unwrap();
    }

    #[test]
    fn indexes_and_physical_labels_follow_relational_order_not_cq_text_order() {
        let logical: relational_plan::Module = syn::parse2(quote! {
            struct Reordered;
            relation R(src: i32, dst: i32);
            relation S(src: i32, dst: i32);
            relation T(src: i32, dst: i32);
            answer(x, w) :- R(x, y), S(y, z), T(z, w).
            relational {
                r0 = rename T {src -> z, dst -> w};
                r1 = rename S {src -> y, dst -> z};
                r2 = natural_join r0 with r1;
                r3 = rename R {src -> x, dst -> y};
                r4 = natural_join r2 with r3;
                r5 = project r4 keep {x, w};
                output r5 as answer(x, w).
            }
        })
        .unwrap();
        relational_plan::contract::check(&logical).unwrap();

        let indexed = compile_relational_plan(&logical).unwrap();
        let signatures = indexed
            .indexes
            .iter()
            .map(index_signature)
            .collect::<Vec<_>>();
        assert_eq!(
            signatures,
            vec![("S".to_owned(), vec![1]), ("R".to_owned(), vec![1])]
        );

        let file = compile_index_requirements(&indexed).unwrap();
        #[derive(Default)]
        struct PullFinder {
            tokens: Option<TokenStream>,
        }
        impl<'ast> Visit<'ast> for PullFinder {
            fn visit_macro(&mut self, node: &'ast syn::Macro) {
                if node
                    .path
                    .segments
                    .last()
                    .is_some_and(|segment| segment.ident == "pull")
                {
                    self.tokens = Some(node.tokens.clone());
                }
                syn::visit::visit_macro(self, node);
            }
        }
        let mut finder = PullFinder::default();
        finder.visit_file(&file);
        let access: cq_ir::rust_access_plan::Plan = syn::parse2(finder.tokens.unwrap()).unwrap();
        let labels = access
            .body
            .iter()
            .map(|clause| match &clause.item {
                cq::BodyItem::Positive { atom } => cq_ir::symbol_name(&atom.relation),
            })
            .collect::<Vec<_>>();
        assert_eq!(labels, ["T", "S", "R"]);
        assert_eq!(access.head.atom, logical.program.query.head);
    }

    #[test]
    fn basic_backend_rejects_missing_or_nonfinal_project() {
        let unsupported = [
            quote! {
                struct MissingProject;
                relation R(src: i32, dst: i32);
                answer(x, y) :- R(x, y).
                relational {
                    r0 = rename R {src -> x, dst -> y};
                    output r0 as answer(x, y).
                }
            },
            quote! {
                struct NonfinalProject;
                relation R(src: i32, dst: i32);
                answer(x) :- R(x, y).
                relational {
                    r0 = rename R {src -> x, dst -> y};
                    r1 = project r0 keep {x, y};
                    r2 = project r1 keep {x};
                    output r2 as answer(x).
                }
            },
        ];

        for source in unsupported {
            let logical: relational_plan::Module = syn::parse2(source).unwrap();
            relational_plan::contract::check(&logical).unwrap();
            assert!(compile_relational_plan(&logical).is_err());
        }
    }
}
