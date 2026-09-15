//! Independent source-target correspondence.
use cq_ir::{index_requirements, relational_plan, symbol_name as symbol};
use std::collections::{BTreeMap, BTreeSet};
/// Whether the target preserves the plan and has its exact ordered key list.
pub fn indexes_are_exact(
    source: &relational_plan::Module,
    target: &index_requirements::Module,
) -> bool {
    super::check_domain(source).is_ok()
        && index_requirements::contract::well_formed(target)
        && target.relational_plan == *source
        && canonical_indexes(source).is_some_and(|expected| {
            target
                .indexes
                .iter()
                .map(index_requirements::signature)
                .eq(expected
                    .into_iter()
                    .map(|(relation, columns)| (false, relation, columns)))
        })
}

fn canonical_indexes(source: &relational_plan::Module) -> Option<Vec<(String, Vec<usize>)>> {
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
    use super::*;
    use quote::quote;

    fn supported_linear_plan(source: &relational_plan::Module) -> bool {
        super::super::check_domain(source).is_ok()
    }

    fn index_signature(index: &index_requirements::IndexRequirement) -> (String, Vec<usize>) {
        let (result, relation, columns) = index_requirements::signature(index);
        assert!(!result);
        (relation, columns)
    }

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

        let target_with_extra: index_requirements::Module = syn::parse2(quote! {
            #source
            indexes { Edge[0]; r#Edge[1]; }
        })
        .unwrap();
        index_requirements::contract::check(&target_with_extra).unwrap();
        assert!(!indexes_are_exact(&source, &target_with_extra));
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
        let error = crate::input::compile(&source).unwrap_err();
        assert!(error.to_string().contains("input-index selection"));
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
        let indexed = crate::input::compile(&source).unwrap();
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
}
