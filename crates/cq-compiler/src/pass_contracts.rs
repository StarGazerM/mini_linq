//! Small semantic relations used by the typed compiler-pass contracts.

pub(crate) mod planned_join;

/// Result-index selection preserves the entire algebra and adds exactly the
/// first-use whole shared keys. This relation does not call the lowering.
pub(crate) fn result_indexes_are_exact(
    source: &cq_ir::relational_plan::Module,
    target: &cq_ir::index_requirements::Module,
) -> bool {
    use cq_ir::{relational_plan::Operator, symbol_name};
    if target.relational_plan != *source
        || !cq_ir::index_requirements::contract::well_formed(target)
    {
        return false;
    }
    let Ok(info) = cq_ir::relational_plan::check_in(&source.program.inputs, &source.plan) else {
        return false;
    };
    let mut expected = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for d in &source.plan.definitions {
        let (l, r) = match &d.operator {
            Operator::NaturalJoin(j) => (&j.left, &j.right),
            Operator::Semijoin(j) => (&j.left, &j.right),
            _ => continue,
        };
        let shared = info[&symbol_name(l)]
            .heading
            .intersection(&info[&symbol_name(r)].heading)
            .collect::<std::collections::BTreeSet<_>>();
        let key = info[&symbol_name(r)]
            .heading
            .iter()
            .enumerate()
            .filter_map(|(i, x)| shared.contains(x).then_some(i))
            .collect::<Vec<_>>();
        let signature = (true, symbol_name(r), key);
        if !signature.2.is_empty() && seen.insert(signature.clone()) {
            expected.push(signature);
        }
    }
    target
        .indexes
        .iter()
        .map(crate::relational_to_indexes::signature)
        .eq(expected)
}

use std::collections::{BTreeMap, BTreeSet};

use cq_ir::{cq, index_requirements, iterator_pipeline, relational_plan, rust_access_plan};
use quote::{format_ident, quote};

type IndexSignature = (String, Vec<usize>);

/// Whether `target` is exactly the canonical named algebra for `source`.
pub(crate) fn relational_plan_is_exact(
    source: &cq::Module,
    target: &relational_plan::Module,
) -> bool {
    // OPTIONAL HW4 — STUDENT 3/6 (LOGICAL): extend this independent exact
    // relation as specified in doc/HW4-OPTIONAL.md.
    if !relational_plan::contract::well_formed(target) || target.program != source.program {
        return false;
    }

    let mut definitions = target.plan.definitions.iter();
    let mut current = None::<String>;
    let mut next_result = 0usize;

    for item in &source.program.query.body {
        let Ok(atom) = item.positive_atom() else {
            return false;
        };
        let Some(rename_definition) = definitions.next() else {
            return false;
        };
        let rename_name = format!("r{next_result}");
        next_result += 1;
        if symbol(&rename_definition.result) != rename_name
            || !matches!(
                &rename_definition.operator,
                relational_plan::Operator::Rename(rename)
                    if rename_is_exact(&source.program, rename, atom)
            )
        {
            return false;
        }

        if let Some(left) = current {
            let Some(join_definition) = definitions.next() else {
                return false;
            };
            let result = format!("r{next_result}");
            next_result += 1;
            if symbol(&join_definition.result) != result
                || !matches!(
                    &join_definition.operator,
                    relational_plan::Operator::NaturalJoin(join)
                        if symbol(&join.left) == left && symbol(&join.right) == rename_name
                )
            {
                return false;
            }
            current = Some(result);
        } else {
            current = Some(rename_name);
        }
    }

    let Some(project_definition) = definitions.next() else {
        return false;
    };
    let project_name = format!("r{next_result}");
    let Some(input) = current else {
        return false;
    };
    symbol(&project_definition.result) == project_name
        && matches!(
            &project_definition.operator,
            relational_plan::Operator::Project(project)
                if symbol(&project.input) == input
                    && symbols(&project.attributes).into_iter().collect::<BTreeSet<_>>()
                        == symbols(source.program.query.head.variables())
                            .into_iter()
                            .collect::<BTreeSet<_>>()
        )
        && definitions.next().is_none()
        && symbol(&target.plan.output.input) == project_name
        && same_atom(&target.plan.output.head, &source.program.query.head)
}

fn rename_is_exact(
    program: &cq::Program,
    rename: &relational_plan::Rename,
    atom: &cq::Atom,
) -> bool {
    if symbol(&rename.relation) != symbol(&atom.relation) {
        return false;
    }
    let Some(declaration) = program
        .inputs
        .iter()
        .find(|input| symbol(&input.name) == symbol(&atom.relation))
    else {
        return false;
    };
    rename.mapping.len() == declaration.columns.len()
        && rename.mapping.len() == atom.args.len()
        && rename
            .mapping
            .iter()
            .zip(&declaration.columns)
            .zip(atom.variables())
            .all(|((mapping, column), variable)| {
                symbol(&mapping.source) == symbol(&column.name)
                    && symbol(&mapping.target) == symbol(variable)
            })
}

/// Whether the basic index and sequential backends can consume this plan.
///
/// RelationalPlan itself deliberately permits general named binary algebra.
/// The basic backend has a smaller, explicit domain: a root Rename, followed
/// by earlier right-hand Renames consumed one-by-one by left-deep
/// NaturalJoins, then a final Project.
pub(crate) fn basic_left_deep_plan(source: &relational_plan::Module) -> bool {
    if !relational_plan::contract::well_formed(source) {
        return false;
    }

    let mut earlier = BTreeMap::<String, (BTreeSet<String>, bool)>::new();
    let mut current = None::<String>;
    let mut unconsumed_renames = BTreeSet::<String>::new();
    let mut projected = false;

    for definition in &source.plan.definitions {
        let result = symbol(&definition.result);
        let heading = match &definition.operator {
            // OPTIONAL HW4 — STUDENT 3/6 (LOGICAL): admit the supplied Unit
            // only when the extended global-clause lowering consumes it.
            relational_plan::Operator::Unit(_)
            | relational_plan::Operator::Semijoin(_)
            | relational_plan::Operator::Select(_) => return false,
            relational_plan::Operator::Rename(rename) => {
                if projected {
                    return false;
                }
                if current.is_none() {
                    current = Some(result.clone());
                } else {
                    unconsumed_renames.insert(result.clone());
                }
                let heading = rename
                    .mapping
                    .iter()
                    .map(|mapping| symbol(&mapping.target))
                    .collect();
                earlier.insert(result, (heading, true));
                continue;
            }
            relational_plan::Operator::NaturalJoin(operator) => {
                let right = symbol(&operator.right);
                let Some(left) = current.as_deref() else {
                    return false;
                };
                let Some((left_heading, _)) = earlier.get(left) else {
                    return false;
                };
                let Some((right_heading, true)) = earlier.get(&right) else {
                    return false;
                };
                if projected || symbol(&operator.left) != left || !unconsumed_renames.remove(&right)
                {
                    return false;
                }
                let mut heading = left_heading.clone();
                heading.extend(right_heading.iter().cloned());
                current = Some(result.clone());
                heading
            }
            relational_plan::Operator::Project(operator) => {
                if projected
                    || !unconsumed_renames.is_empty()
                    || current.as_deref() != Some(symbol(&operator.input).as_str())
                {
                    return false;
                }
                projected = true;
                current = Some(result.clone());
                operator.attributes.iter().map(symbol).collect()
            }
        };
        earlier.insert(result, (heading, false));
    }

    projected
        && unconsumed_renames.is_empty()
        && current
            .as_deref()
            .is_some_and(|current| current == symbol(&source.plan.output.input))
}

/// Whether the target preserves the plan and has its exact ordered key list.
pub(crate) fn indexes_are_exact(
    source: &relational_plan::Module,
    target: &index_requirements::Module,
) -> bool {
    index_requirements::contract::well_formed(target)
        && target.relational_plan == *source
        && canonical_indexes(source)
            .is_some_and(|expected| target.indexes.iter().map(index_signature).eq(expected))
}

/// Whether a locally valid module contains every canonical required index.
///
/// Local IndexRequirements validation deliberately permits extra indexes and
/// does not prove that the written list covers the relational plan's named
/// right-hand input occurrences. This predicate is used only by a debug pass
/// contract; the homework lowering reports a missing key at the corresponding
/// occurrence.
pub(crate) fn covers_required_indexes(source: &index_requirements::Module) -> bool {
    let available = source
        .indexes
        .iter()
        .map(index_signature)
        .collect::<BTreeSet<_>>();
    canonical_indexes(&source.relational_plan).is_some_and(|required| {
        required
            .into_iter()
            .all(|required| available.contains(&required))
    })
}

/// Whether the basic physical pass is defined and has all required indexes.
pub(crate) fn physical_input_is_supported(source: &index_requirements::Module) -> bool {
    source.indexes.iter().all(|i| i.result_token.is_none())
        && basic_left_deep_plan(&source.relational_plan)
        && covers_required_indexes(source)
}

/// Whether `target` is exactly the explicit iterator pipeline denoted by
/// `source`.
///
/// This is an independent pass relation: it compares the two typed values
/// directly and never invokes the lowering under test. Besides requiring the
/// target's local contract, it preserves source order, every Rust pattern and
/// leaf expression, the complete intermediate binding at each boundary, and
/// the head output expression.
pub(crate) fn iterator_pipeline_is_exact(
    source: &rust_access_plan::Plan,
    target: &iterator_pipeline::Pipeline,
) -> bool {
    if !iterator_pipeline::contract::well_formed(target) {
        return false;
    }

    let mut binding = Vec::new();
    let mut definitions = target.definitions.iter();
    let Some(first_definition) = definitions.next() else {
        return false;
    };
    let mut expected_stream_number = 0usize;
    if first_definition.stream != generated_stream(expected_stream_number) {
        return false;
    }
    expected_stream_number += 1;
    let first = source.body.first();
    let access_offset = match (first.map(|clause| &clause.access), first_definition) {
        (
            Some(rust_access_plan::RustAccess::For {
                pattern,
                source: rust_source,
                ..
            }),
            iterator_pipeline::Definition {
                operator: iterator_pipeline::Operator::Scan(scan),
                ..
            },
        ) => {
            let Some(introduced) = enumerating_variables(pattern) else {
                return false;
            };
            binding.extend(introduced);
            if scan.item_pattern != *pattern
                || scan.source != *rust_source
                || scan.binding != binding_expression(&binding)
            {
                return false;
            }
            1
        }
        (
            None | Some(rust_access_plan::RustAccess::If { .. }),
            iterator_pipeline::Definition {
                stream,
                operator: iterator_pipeline::Operator::Unit(unit),
                ..
            },
        ) if stream == &generated_stream(0) && unit.binding == syn::parse_quote!(()) => 0,
        _ => return false,
    };

    let mut previous = first_definition.stream.clone();
    for clause in source.body.iter().skip(access_offset) {
        let Some(definition) = definitions.next() else {
            return false;
        };
        if definition.stream != generated_stream(expected_stream_number) {
            return false;
        }
        expected_stream_number += 1;
        match (&clause.access, &definition.operator) {
            (
                rust_access_plan::RustAccess::For {
                    pattern,
                    source: rust_source,
                    ..
                },
                iterator_pipeline::Operator::Join(join),
            ) => {
                if join.input_stream != previous
                    || join.input_pattern != binding_pattern(&binding)
                    || join.item_pattern != *pattern
                    || join.source != *rust_source
                {
                    return false;
                }
                let Some(introduced) = enumerating_variables(pattern) else {
                    return false;
                };
                binding.extend(introduced);
                if join.binding != binding_expression(&binding) {
                    return false;
                }
            }
            (
                rust_access_plan::RustAccess::If { condition, .. },
                iterator_pipeline::Operator::Filter(filter),
            ) => {
                if filter.input_stream != previous
                    || filter.input_pattern != binding_pattern(&binding)
                    || filter.condition != *condition
                {
                    return false;
                }
            }
            _ => return false,
        }
        previous = definition.stream.clone();
    }

    let Some(project_definition) = definitions.next() else {
        return false;
    };
    if project_definition.stream != generated_stream(expected_stream_number) {
        return false;
    }
    expected_stream_number += 1;
    let iterator_pipeline::Operator::Project(project) = &project_definition.operator else {
        return false;
    };
    if project.input_stream != previous
        || project.input_pattern != binding_pattern(&binding)
        || project.output != source.head.output
    {
        return false;
    }

    let Some(distinct_definition) = definitions.next() else {
        return false;
    };
    if distinct_definition.stream != generated_stream(expected_stream_number) {
        return false;
    }
    let iterator_pipeline::Operator::Distinct(distinct) = &distinct_definition.operator else {
        return false;
    };
    distinct.input_stream == project_definition.stream
        && definitions.next().is_none()
        && target.return_stream.stream == distinct_definition.stream
}

/// Whether `target` is the exact ordinary-Rust implementation denoted by the
/// named operator plan.
///
/// This relation independently constructs the required target shape. It does
/// not call the lowering under contract: it checks the fixed deferred wrapper,
/// one local per definition in source order, every exact adapter initializer,
/// and the recorded final stream.
pub(crate) fn iterator_pipeline_rust_is_exact(
    source: &iterator_pipeline::Pipeline,
    target: &syn::Expr,
) -> bool {
    if !iterator_pipeline::contract::well_formed(source) {
        return false;
    }
    let locals = source
        .definitions
        .iter()
        .map(expected_rust_local)
        .collect::<Vec<_>>();
    let return_stream = &source.return_stream.stream;
    let Ok(expected) = syn::parse2::<syn::Expr>(quote! {
        ::core::iter::Iterator::flatten(
            ::core::iter::once_with(move || {
                #(#locals)*
                #return_stream
            })
        )
    }) else {
        return false;
    };
    target == &expected
}

fn expected_rust_local(definition: &iterator_pipeline::Definition) -> proc_macro2::TokenStream {
    let stream = &definition.stream;
    match &definition.operator {
        iterator_pipeline::Operator::Unit(operator) => {
            let binding = &operator.binding;
            quote!(let #stream = ::core::iter::once(#binding);)
        }
        iterator_pipeline::Operator::Scan(operator) => {
            let pattern = &operator.item_pattern;
            let source = &operator.source;
            let binding = &operator.binding;
            quote! {
                let #stream = ::core::iter::Iterator::map(
                    ::core::iter::IntoIterator::into_iter(#source),
                    move |#pattern| #binding,
                );
            }
        }
        iterator_pipeline::Operator::Join(operator) => {
            let input_stream = &operator.input_stream;
            let input_pattern = &operator.input_pattern;
            let item_pattern = &operator.item_pattern;
            let source = &operator.source;
            let binding = &operator.binding;
            quote! {
                let #stream = ::core::iter::Iterator::flat_map(
                    #input_stream,
                    move |#input_pattern| {
                        ::core::iter::Iterator::map(
                            ::core::iter::IntoIterator::into_iter(#source),
                            move |#item_pattern| #binding,
                        )
                    },
                );
            }
        }
        iterator_pipeline::Operator::Filter(operator) => {
            let input_stream = &operator.input_stream;
            let input_pattern = &operator.input_pattern;
            let borrowed_pattern: syn::Pat = syn::parse_quote!(&#input_pattern);
            let condition = &operator.condition;
            quote! {
                let #stream = ::core::iter::Iterator::filter(
                    #input_stream,
                    move |#[allow(unused_variables)] #borrowed_pattern| #condition,
                );
            }
        }
        iterator_pipeline::Operator::Project(operator) => {
            let input_stream = &operator.input_stream;
            let input_pattern = &operator.input_pattern;
            let output = &operator.output;
            quote! {
                let #stream = ::core::iter::Iterator::map(
                    #input_stream,
                    move |#[allow(unused_variables)] #input_pattern| #output,
                );
            }
        }
        iterator_pipeline::Operator::Distinct(operator) => {
            let input_stream = &operator.input_stream;
            quote! {
                let #stream = {
                    let mut seen = ::std::collections::HashSet::new();
                    ::core::iter::Iterator::filter(
                        #input_stream,
                        move |row| seen.insert(::core::clone::Clone::clone(row)),
                    )
                };
            }
        }
    }
}

fn generated_stream(number: usize) -> syn::Ident {
    format_ident!("iter{number}", span = proc_macro2::Span::mixed_site())
}

fn enumerating_variables(pattern: &syn::Pat) -> Option<Vec<syn::Ident>> {
    match pattern {
        syn::Pat::Tuple(pattern) => pattern.elems.iter().map(simple_pattern_ident).collect(),
        syn::Pat::Ident(_) => simple_pattern_ident(pattern).map(|ident| vec![ident]),
        _ => None,
    }
}

fn simple_pattern_ident(pattern: &syn::Pat) -> Option<syn::Ident> {
    let syn::Pat::Ident(pattern) = pattern else {
        return None;
    };
    (pattern.attrs.is_empty()
        && pattern.by_ref.is_none()
        && pattern.mutability.is_none()
        && pattern.subpat.is_none())
    .then(|| pattern.ident.clone())
}

fn binding_pattern(binding: &[syn::Ident]) -> syn::Pat {
    syn::parse_quote!((#(#binding,)*))
}

fn binding_expression(binding: &[syn::Ident]) -> syn::Expr {
    syn::parse_quote!((#(#binding,)*))
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

fn canonical_indexes(source: &relational_plan::Module) -> Option<Vec<IndexSignature>> {
    // OPTIONAL HW4 — STUDENT 4/6 (INDEX): extend this independent canonical
    // key walk as specified in doc/HW4-OPTIONAL.md.
    let declarations = source
        .program
        .inputs
        .iter()
        .map(|input| (symbol(&input.name), input))
        .collect::<BTreeMap<_, _>>();
    let mut earlier = BTreeMap::<String, (&relational_plan::Definition, BTreeSet<String>)>::new();
    let mut seen = BTreeSet::new();
    let mut requirements = Vec::new();

    for definition in &source.plan.definitions {
        let heading = match &definition.operator {
            relational_plan::Operator::Unit(_)
            | relational_plan::Operator::Semijoin(_)
            | relational_plan::Operator::Select(_) => return None,
            relational_plan::Operator::Rename(rename) => rename
                .mapping
                .iter()
                .map(|mapping| symbol(&mapping.target))
                .collect(),
            relational_plan::Operator::NaturalJoin(join) => {
                let (_, left_heading) = earlier.get(&symbol(&join.left))?;
                let (right_definition, right_heading) = earlier.get(&symbol(&join.right))?;
                let relational_plan::Operator::Rename(rename) = &right_definition.operator else {
                    return None;
                };
                let declaration = declarations.get(&symbol(&rename.relation))?;
                let shared = left_heading
                    .intersection(right_heading)
                    .cloned()
                    .collect::<BTreeSet<_>>();
                let mapping = rename
                    .mapping
                    .iter()
                    .map(|attribute| (symbol(&attribute.source), symbol(&attribute.target)))
                    .collect::<BTreeMap<_, _>>();
                let columns = declaration
                    .columns
                    .iter()
                    .enumerate()
                    .filter_map(|(column, declaration)| {
                        mapping
                            .get(&symbol(&declaration.name))
                            .is_some_and(|target| shared.contains(target))
                            .then_some(column)
                    })
                    .collect::<Vec<_>>();
                let signature = (symbol(&rename.relation), columns);
                if !signature.1.is_empty() && seen.insert(signature.clone()) {
                    requirements.push(signature);
                }

                let mut heading = left_heading.clone();
                heading.extend(right_heading.iter().cloned());
                heading
            }
            relational_plan::Operator::Project(project) => {
                project.attributes.iter().map(symbol).collect()
            }
        };
        earlier.insert(symbol(&definition.result), (definition, heading));
    }

    Some(requirements)
}

fn symbol(identifier: &syn::Ident) -> String {
    cq_ir::symbol_name(identifier)
}

fn symbols<'a>(identifiers: impl IntoIterator<Item = &'a syn::Ident>) -> Vec<String> {
    identifiers.into_iter().map(symbol).collect()
}

fn same_atom(left: &cq::Atom, right: &cq::Atom) -> bool {
    symbol(&left.relation) == symbol(&right.relation)
        && symbols(left.variables()) == symbols(right.variables())
}

/// Domain of the unchanged R1 lowering.
pub(crate) fn r1_cq(source: &cq::Module) -> bool {
    source
        .program
        .query
        .body
        .iter()
        .all(|item| matches!(item, cq::BodyItem::Positive { .. }))
}

#[cfg(test)]
mod tests {
    use quote::quote;

    use super::*;

    #[test]
    fn cq_to_relational_contract_requires_the_complete_canonical_named_plan() {
        let source: cq::Module = syn::parse2(quote! {
            struct P;
            relation R(c0: i32, c1: i32);
            relation S(c0: i32, c1: i32);
            answer(x, z) :- R(x, y), S(y, z).
        })
        .unwrap();
        let exact: relational_plan::Module = syn::parse2(quote! {
            struct P;
            relation R(c0: i32, c1: i32);
            relation S(c0: i32, c1: i32);
            answer(x, z) :- R(x, y), S(y, z).
            relational {
                r0 = rename R {c0 -> x, c1 -> y};
                r1 = rename S {c0 -> y, c1 -> z};
                r2 = natural_join r0 with r1;
                r3 = project r2 keep {z, x};
                output r3 as answer(x, z).
            }
        })
        .unwrap();
        assert!(relational_plan_is_exact(&source, &exact));

        let renamed_but_locally_valid: relational_plan::Module = syn::parse2(quote! {
            struct P;
            relation R(c0: i32, c1: i32);
            relation S(c0: i32, c1: i32);
            answer(x, z) :- R(x, y), S(y, z).
            relational {
                left = rename R {c0 -> x, c1 -> y};
                right = rename S {c0 -> y, c1 -> z};
                joined = natural_join left with right;
                answer_rows = project joined keep {x, z};
                output answer_rows as answer(x, z).
            }
        })
        .unwrap();
        relational_plan::contract::check(&renamed_but_locally_valid).unwrap();
        assert!(!relational_plan_is_exact(
            &source,
            &renamed_but_locally_valid
        ));
    }

    #[test]
    fn composed_index_contract_clauses_compare_relation_symbols_after_unraw() {
        let source: relational_plan::Module = syn::parse2(quote! {
            struct RawIndexedRelation;
            relation Seed(c0: i32);
            relation r#Edge(c0: i32, c1: i32);
            answer(x, z) :- Seed(x), Edge(z, x).
            relational {
                r0 = rename Seed {c0 -> x};
                r1 = rename r#Edge {c0 -> z, c1 -> x};
                r2 = natural_join r0 with r1;
                r3 = project r2 keep {x, z};
                output r3 as answer(x, z).
            }
        })
        .unwrap();
        relational_plan::contract::check(&source).unwrap();

        let target: index_requirements::Module = syn::parse2(quote! {
            #source
            indexes { r#Edge[1]; }
        })
        .unwrap();
        index_requirements::contract::check(&target).unwrap();

        assert!(index_requirements::contract::well_formed(&target));
        assert_eq!(target.relational_plan, source);
        assert!(indexes_are_exact(&source, &target));
        assert!(covers_required_indexes(&target));

        let target_with_extra: index_requirements::Module = syn::parse2(quote! {
            #source
            indexes { Edge[0]; r#Edge[1]; }
        })
        .unwrap();
        index_requirements::contract::check(&target_with_extra).unwrap();
        assert!(!indexes_are_exact(&source, &target_with_extra));
        assert!(covers_required_indexes(&target_with_extra));
    }

    #[test]
    fn physical_coverage_rejects_a_missing_required_key_but_allows_extras() {
        let missing: index_requirements::Module = syn::parse2(quote! {
            struct MissingRequiredIndex;
            relation Seed(c0: i32);
            relation Edge(c0: i32, c1: i32);
            answer(x, z) :- Seed(x), Edge(z, x).
            relational {
                r0 = rename Seed {c0 -> x};
                r1 = rename Edge {c0 -> z, c1 -> x};
                r2 = natural_join r0 with r1;
                r3 = project r2 keep {x, z};
                output r3 as answer(x, z).
            }
            indexes {}
        })
        .unwrap();
        index_requirements::contract::check(&missing).unwrap();
        assert!(!covers_required_indexes(&missing));

        let with_extra: index_requirements::Module = syn::parse2(quote! {
            struct ExtraIndex;
            relation Seed(c0: i32);
            relation Edge(c0: i32, c1: i32);
            answer(x, z) :- Seed(x), Edge(z, x).
            relational {
                r0 = rename Seed {c0 -> x};
                r1 = rename Edge {c0 -> z, c1 -> x};
                r2 = natural_join r0 with r1;
                r3 = project r2 keep {x, z};
                output r3 as answer(x, z).
            }
            indexes { Edge[0]; Edge[1]; }
        })
        .unwrap();
        index_requirements::contract::check(&with_extra).unwrap();
        assert!(covers_required_indexes(&with_extra));

        let row_only: index_requirements::Module = syn::parse2(quote! {
            struct RowOnly;
            relation R(c0: i32, c1: i32);
            answer(x) :- R(x, y).
            relational {
                r0 = rename R {c0 -> x, c1 -> y};
                r1 = project r0 keep {x};
                output r1 as answer(x).
            }
            indexes {}
        })
        .unwrap();
        index_requirements::contract::check(&row_only).unwrap();
        assert!(covers_required_indexes(&row_only));
    }

    #[test]
    fn bushy_right_intermediate_is_locally_relational_but_outside_basic_backend() {
        let source: relational_plan::Module = syn::parse2(quote! {
            struct Bushy;
            relation R(c0: i32, c1: i32);
            relation S(c0: i32, c1: i32);
            relation T(c0: i32, c1: i32);
            answer(x, w) :- R(x, y), S(y, z), T(z, w).
            relational {
                r0 = rename R {c0 -> x, c1 -> y};
                r1 = rename S {c0 -> y, c1 -> z};
                r2 = rename T {c0 -> z, c1 -> w};
                r3 = natural_join r1 with r2;
                r4 = natural_join r0 with r3;
                r5 = project r4 keep {x, w};
                output r5 as answer(x, w).
            }
        })
        .unwrap();
        relational_plan::contract::check(&source).unwrap();

        assert!(!basic_left_deep_plan(&source));
        let error = crate::compile_relational_plan(&source).unwrap_err();
        assert!(error.to_string().contains("basic left-deep backend"));
    }

    #[test]
    fn basic_left_deep_plan_accepts_renames_declared_before_their_joins() {
        let source: relational_plan::Module = syn::parse2(quote! {
            struct ScansFirst;
            relation R(c0: i32, c1: i32);
            relation S(c0: i32, c1: i32);
            relation T(c0: i32, c1: i32);
            answer(x, w) :- R(x, y), S(y, z), T(z, w).
            relational {
                r0 = rename R {c0 -> x, c1 -> y};
                r1 = rename S {c0 -> y, c1 -> z};
                r2 = rename T {c0 -> z, c1 -> w};
                r3 = natural_join r0 with r1;
                r4 = natural_join r3 with r2;
                r5 = project r4 keep {x, w};
                output r5 as answer(x, w).
            }
        })
        .unwrap();
        relational_plan::contract::check(&source).unwrap();

        assert!(basic_left_deep_plan(&source));
        let indexed = crate::compile_relational_plan(&source).unwrap();
        assert!(indexes_are_exact(&source, &indexed));
        assert_eq!(
            indexed
                .indexes
                .iter()
                .map(index_signature)
                .collect::<Vec<_>>(),
            [("S".to_owned(), vec![0]), ("T".to_owned(), vec![0])]
        );
    }

    #[test]
    fn supplied_unit_is_locally_valid_but_reserved_for_hw4_lowering() {
        let source: relational_plan::Module = syn::parse2(quote! {
            struct GlobalSeed;
            relation R(c0: i32);
            answer(x) :- R(x).
            relational {
                r0 = unit;
                r1 = rename R {c0 -> x};
                r2 = natural_join r0 with r1;
                r3 = project r2 keep {x};
                output r3 as answer(x).
            }
        })
        .unwrap();
        relational_plan::contract::check(&source).unwrap();

        assert!(!basic_left_deep_plan(&source));
        assert!(canonical_indexes(&source).is_none());
        let logical_error = crate::compile_relational_plan(&source).unwrap_err();
        assert!(logical_error.to_string().contains("OPTIONAL HW4"));

        let indexed: index_requirements::Module = syn::parse2(quote! {
            #source
            indexes {}
        })
        .unwrap();
        index_requirements::contract::check(&indexed).unwrap();
        assert!(!physical_input_is_supported(&indexed));
        let physical_error = crate::compile_index_requirements(&indexed).unwrap_err();
        assert!(physical_error.to_string().contains("OPTIONAL HW4"));
    }

    #[test]
    fn iterator_pipeline_relations_reject_leaf_order_and_rust_mutations() {
        let source: rust_access_plan::Plan = syn::parse2(quote! {
            answer(x, y) => ((x.clone(), y.clone(),)) :-
                Left(x) => for (x,) in (left.iter()),
                Right(x, y) => for (y,) in
                    (right.get(&(x.clone(),)).into_iter().flatten()),
                Keep(x, y) => if (keep(*x, *y)).
        })
        .unwrap();
        rust_access_plan::contract::check(&source).unwrap();

        let exact: iterator_pipeline::Pipeline = syn::parse2(quote! {
            iter0 = scan (x,) in (left.iter()) yield (x,);
            iter1 = join iter0 as (x,) with (y,) in
                (right.get(&(x.clone(),)).into_iter().flatten()) yield (x, y,);
            iter2 = filter iter1 as (x, y,) if (keep(*x, *y));
            iter3 = project iter2 as (x, y,) yield ((x.clone(), y.clone(),));
            iter4 = distinct iter3;
            return iter4.
        })
        .unwrap();
        iterator_pipeline::contract::check(&exact).unwrap();
        assert!(iterator_pipeline_is_exact(&source, &exact));

        let mut changed_source = exact.clone();
        let iterator_pipeline::Operator::Join(join) = &mut changed_source.definitions[1].operator
        else {
            unreachable!()
        };
        join.source = syn::parse_quote!(other.get(&(x.clone(),)).into_iter().flatten());

        let mut changed_condition = exact.clone();
        let iterator_pipeline::Operator::Filter(filter) =
            &mut changed_condition.definitions[2].operator
        else {
            unreachable!()
        };
        filter.condition = syn::parse_quote!(!keep(*x, *y));

        let mut changed_output = exact.clone();
        let iterator_pipeline::Operator::Project(project) =
            &mut changed_output.definitions[3].operator
        else {
            unreachable!()
        };
        project.output = syn::parse_quote!((y.clone(), x.clone(),));

        let reordered: iterator_pipeline::Pipeline = syn::parse2(quote! {
            iter0 = scan (x,) in (left.iter()) yield (x,);
            iter1 = filter iter0 as (x,) if (ready(*x));
            iter2 = join iter1 as (x,) with (y,) in
                (right.get(&(x.clone(),)).into_iter().flatten()) yield (x, y,);
            iter3 = project iter2 as (x, y,) yield ((x.clone(), y.clone(),));
            iter4 = distinct iter3;
            return iter4.
        })
        .unwrap();

        for different_but_well_formed in
            [changed_source, changed_condition, changed_output, reordered]
        {
            iterator_pipeline::contract::check(&different_but_well_formed).unwrap();
            assert!(!iterator_pipeline_is_exact(
                &source,
                &different_but_well_formed
            ));
        }

        let exact_rust: syn::Expr = syn::parse2(quote! {
            ::core::iter::Iterator::flatten(
                ::core::iter::once_with(move || {
                    let iter0 = ::core::iter::Iterator::map(
                        ::core::iter::IntoIterator::into_iter(left.iter()),
                        move |(x,)| (x,),
                    );
                    let iter1 = ::core::iter::Iterator::flat_map(
                        iter0,
                        move |(x,)| {
                            ::core::iter::Iterator::map(
                                ::core::iter::IntoIterator::into_iter(
                                    right.get(&(x.clone(),)).into_iter().flatten()
                                ),
                                move |(y,)| (x, y,),
                            )
                        },
                    );
                    let iter2 = ::core::iter::Iterator::filter(
                        iter1,
                        move |#[allow(unused_variables)] &(x, y,)| keep(*x, *y),
                    );
                    let iter3 = ::core::iter::Iterator::map(
                        iter2,
                        move |#[allow(unused_variables)] (x, y,)| (x.clone(), y.clone(),),
                    );
                    let iter4 = {
                        let mut seen = ::std::collections::HashSet::new();
                        ::core::iter::Iterator::filter(
                            iter3,
                            move |row| seen.insert(::core::clone::Clone::clone(row)),
                        )
                    };
                    iter4
                })
            )
        })
        .unwrap();
        assert!(iterator_pipeline_rust_is_exact(&exact, &exact_rust));

        let wrong_rust: syn::Expr = syn::parse2(quote! {
            ::core::iter::Iterator::flatten(
                ::core::iter::once_with(move || {
                    let iter0 = ::core::iter::Iterator::map(
                        ::core::iter::IntoIterator::into_iter(other.iter()),
                        move |(x,)| (x,),
                    );
                    iter0
                })
            )
        })
        .unwrap();
        assert!(!iterator_pipeline_rust_is_exact(&exact, &wrong_rust));
    }
}
