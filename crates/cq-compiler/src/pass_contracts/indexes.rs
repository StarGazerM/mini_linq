//! RA-to-index correspondence and the indexed linear backend's input domain.

use std::collections::{BTreeMap, BTreeSet};

use cq_ir::{index_requirements, relational_plan, symbol_name as symbol};

type IndexSignature = (String, Vec<usize>);

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
    let Ok(info) = cq_ir::relational_plan::check_in(source.program.inputs(), &source.plan) else {
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

/// Whether the indexed linear routes can consume this relational plan.
///
/// The base-index route accepts a root Rename, earlier right-hand Renames
/// consumed one-by-one by left-deep NaturalJoins, and a final Project.
/// Plans needing result indexes instead use the linear-prefix domain, which
/// includes Unit, Extend and Select. General named binary algebra is broader
/// than either route.
pub(crate) fn supported_linear_plan(source: &relational_plan::Module) -> bool {
    if crate::relational_to_indexes::uses_result_index_route(source) {
        return crate::relational_to_indexes::is_linear_prefix(source);
    }
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
            relational_plan::Operator::Unit(_)
            | relational_plan::Operator::Semijoin(_)
            | relational_plan::Operator::Extend(_)
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
    if crate::relational_to_indexes::uses_result_index_route(source) {
        return result_indexes_are_exact(source, target);
    }
    index_requirements::contract::well_formed(target)
        && target.relational_plan == *source
        && canonical_indexes(source)
            .is_some_and(|expected| target.indexes.iter().map(index_signature).eq(expected))
}

/// Whether a locally valid module contains every canonical required index.
///
/// Local IndexRequirements validation deliberately permits extra indexes and
/// does not prove that the written list covers the relational plan's named
/// right-hand input occurrences. The physical entry also checks coverage while
/// lowering and reports a missing key at its corresponding occurrence.
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
    let declarations = source
        .program
        .inputs()
        .map(|input| (symbol(&input.name), input))
        .collect::<BTreeMap<_, _>>();
    let mut earlier = BTreeMap::<String, (&relational_plan::Definition, BTreeSet<String>)>::new();
    let mut seen = BTreeSet::new();
    let mut requirements = Vec::new();

    for definition in &source.plan.definitions {
        let heading = match &definition.operator {
            relational_plan::Operator::Unit(_)
            | relational_plan::Operator::Semijoin(_)
            | relational_plan::Operator::Extend(_)
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

#[cfg(test)]
mod tests {
    use quote::quote;

    use super::*;

    #[test]
    fn composed_index_contract_clauses_compare_relation_symbols_after_unraw() {
        let source: relational_plan::Module = syn::parse2(quote! {
            struct RawIndexedRelation;
            relation Seed(c0: i32);
            relation r#Edge(c0: i32, c1: i32);
            answer(x, z) :- Seed(x), Edge(z, x);
            relational {
                r0 = rename Seed {c0 -> x};
                r1 = rename r#Edge {c0 -> z, c1 -> x};
                r2 = natural_join r0 with r1;
                r3 = project r2 keep {x, z};
                output r3 as answer(x, z);
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
            answer(x, z) :- Seed(x), Edge(z, x);
            relational {
                r0 = rename Seed {c0 -> x};
                r1 = rename Edge {c0 -> z, c1 -> x};
                r2 = natural_join r0 with r1;
                r3 = project r2 keep {x, z};
                output r3 as answer(x, z);
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
            answer(x, z) :- Seed(x), Edge(z, x);
            relational {
                r0 = rename Seed {c0 -> x};
                r1 = rename Edge {c0 -> z, c1 -> x};
                r2 = natural_join r0 with r1;
                r3 = project r2 keep {x, z};
                output r3 as answer(x, z);
            }
            indexes { Edge[0]; Edge[1]; }
        })
        .unwrap();
        index_requirements::contract::check(&with_extra).unwrap();
        assert!(covers_required_indexes(&with_extra));

        let row_only: index_requirements::Module = syn::parse2(quote! {
            struct RowOnly;
            relation R(c0: i32, c1: i32);
            answer(x) :- R(x, y);
            relational {
                r0 = rename R {c0 -> x, c1 -> y};
                r1 = project r0 keep {x};
                output r1 as answer(x);
            }
            indexes {}
        })
        .unwrap();
        index_requirements::contract::check(&row_only).unwrap();
        assert!(covers_required_indexes(&row_only));
    }

    #[test]
    fn bushy_right_intermediate_is_locally_relational_but_outside_linear_backend() {
        let source: relational_plan::Module = syn::parse2(quote! {
            struct Bushy;
            relation R(c0: i32, c1: i32);
            relation S(c0: i32, c1: i32);
            relation T(c0: i32, c1: i32);
            answer(x, w) :- R(x, y), S(y, z), T(z, w);
            relational {
                r0 = rename R {c0 -> x, c1 -> y};
                r1 = rename S {c0 -> y, c1 -> z};
                r2 = rename T {c0 -> z, c1 -> w};
                r3 = natural_join r1 with r2;
                r4 = natural_join r0 with r3;
                r5 = project r4 keep {x, w};
                output r5 as answer(x, w);
            }
        })
        .unwrap();
        relational_plan::contract::check(&source).unwrap();

        assert!(!supported_linear_plan(&source));
        let error = crate::compile_relational_plan(&source).unwrap_err();
        assert!(error.to_string().contains("input-indexed backend"));
    }

    #[test]
    fn supported_linear_plan_accepts_renames_declared_before_their_joins() {
        let source: relational_plan::Module = syn::parse2(quote! {
            struct ScansFirst;
            relation R(c0: i32, c1: i32);
            relation S(c0: i32, c1: i32);
            relation T(c0: i32, c1: i32);
            answer(x, w) :- R(x, y), S(y, z), T(z, w);
            relational {
                r0 = rename R {c0 -> x, c1 -> y};
                r1 = rename S {c0 -> y, c1 -> z};
                r2 = rename T {c0 -> z, c1 -> w};
                r3 = natural_join r0 with r1;
                r4 = natural_join r3 with r2;
                r5 = project r4 keep {x, w};
                output r5 as answer(x, w);
            }
        })
        .unwrap();
        relational_plan::contract::check(&source).unwrap();

        assert!(supported_linear_plan(&source));
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
    fn unit_seed_is_supported_by_linear_lowering() {
        let source: relational_plan::Module = syn::parse2(quote! {
            struct GlobalSeed;
            relation R(c0: i32);
            answer(x) :- R(x);
            relational {
                r0 = unit;
                r1 = rename R {c0 -> x};
                r2 = natural_join r0 with r1;
                r3 = project r2 keep {x};
                output r3 as answer(x);
            }
        })
        .unwrap();
        relational_plan::contract::check(&source).unwrap();

        assert!(supported_linear_plan(&source));
        assert!(canonical_indexes(&source).is_none());
        let requirements = crate::compile_relational_plan(&source).unwrap();
        assert!(indexes_are_exact(&source, &requirements));

        let indexed: index_requirements::Module = syn::parse2(quote! {
            #source
            indexes {}
        })
        .unwrap();
        index_requirements::contract::check(&indexed).unwrap();
        assert!(crate::pass_contracts::physical::indexed_input_is_supported(
            &indexed
        ));
        crate::compile_index_requirements(&indexed).unwrap();
    }
}
