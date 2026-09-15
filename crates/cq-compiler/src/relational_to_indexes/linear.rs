//! Storage-free facts for the input-index policy's linear RA domain.
//! The physical input-indexed route consumes the same occurrence/key analysis.

use cq_ir::{cq, relational_plan};
use quote::quote;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AccessKind {
    Enumerate,
    Membership,
}

#[derive(Clone, Debug)]
pub(crate) struct InputAccess {
    pub(crate) atom: cq::Atom,
    pub(crate) key_columns: Vec<usize>,
    pub(crate) kind: AccessKind,
}

#[derive(Clone, Debug)]
pub(crate) struct InputAccesses {
    pub(crate) accesses: Vec<InputAccess>,
    pub(crate) head: cq::Atom,
}

type Heading = BTreeSet<String>;

/// Recover the left-deep access sequence from named relational dataflow.
///
/// The basic physical backend accepts one root Rename followed by NaturalJoins
/// whose right operand is a direct Rename occurrence, then one final Project.
/// NaturalJoin becomes enumeration when the right heading adds attributes and
/// membership when the right heading is already fully bound.
pub(crate) fn input_accesses(source: &relational_plan::Module) -> syn::Result<InputAccesses> {
    let declarations = source
        .program
        .inputs()
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
            relational_plan::Operator::Extend(_)
            | relational_plan::Operator::Semijoin(_)
            | relational_plan::Operator::Select(_) => {
                return Err(syn::Error::new_spanned(
                    definition,
                    "the input-indexed route does not support semijoin, selection or extension",
                ));
            }
            relational_plan::Operator::Unit(unit) => {
                return Err(unsupported_unit_error(unit));
            }
            relational_plan::Operator::Rename(rename) => {
                if projected {
                    return Err(syn::Error::new_spanned(
                        rename,
                        "the input-indexed backend requires Rename inputs before the final Project",
                    ));
                }
                let declaration = declarations
                    .get(&cq_ir::symbol_name(&rename.relation))
                    .expect("RelationalPlan well-formedness requires a declared Rename input");
                let heading = rename_heading(rename);
                if current.is_none() {
                    current = Some(result.clone());
                    accesses.push(InputAccess {
                        atom: renamed_atom(rename, declaration),
                        key_columns: Vec::new(),
                        kind: AccessKind::Enumerate,
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
                        "the input-indexed backend requires NaturalJoins before the final Project",
                    ));
                }
                require_current_left("natural join", &join.left, current.as_deref())?;
                let right_name = cq_ir::symbol_name(&join.right);
                if !unconsumed_renames.remove(&right_name) {
                    return Err(syn::Error::new_spanned(
                        &join.right,
                        "the input-indexed backend must consume each non-root Rename exactly once as a right operand",
                    ));
                }
                let (right_definition, right_heading) = earlier
                    .get(&right_name)
                    .expect("RelationalPlan well-formedness requires an earlier right operand");
                let relational_plan::Operator::Rename(rename) = &right_definition.operator else {
                    return Err(syn::Error::new_spanned(
                        &join.right,
                        "index selection and the input-indexed backend require a direct Rename as the right operand",
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
                accesses.push(InputAccess {
                    atom: renamed_atom(rename, declaration),
                    key_columns: relational_key_columns(rename, declaration, &shared),
                    kind: if right_heading.is_subset(left_heading) {
                        AccessKind::Membership
                    } else {
                        AccessKind::Enumerate
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
                        "the input-indexed backend accepts exactly one final Project",
                    ));
                }
                require_current_left("project", &project.input, current.as_deref())?;
                if !unconsumed_renames.is_empty() {
                    return Err(syn::Error::new_spanned(
                        project,
                        "the input-indexed backend requires every non-root Rename to be the right operand of exactly one NaturalJoin",
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
            "the input-indexed backend requires one final Project selected by output metadata",
        ));
    }

    Ok(InputAccesses {
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
            "the input-indexed backend requires the {operator} left operand to be the preceding operator result"
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
        "the input-indexed route requires a Rename root; select the result-index route for `unit`",
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
