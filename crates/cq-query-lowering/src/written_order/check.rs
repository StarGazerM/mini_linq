//! Exact CQ-to-canonical-RA correspondence, checked independently of lowering.

use std::collections::BTreeSet;

use cq_ir::{cq, relational_plan, symbol_name as symbol};

/// Check source validity and the exact canonical CQ -> RA correspondence.
pub fn check(source: &cq::Module, target: &relational_plan::Module) -> syn::Result<()> {
    cq::contract::check(source)?;
    if relational_plan_is_exact(source, target) {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(
            target,
            "target is not the canonical relational plan for this CQ",
        ))
    }
}

/// Whether `target` is exactly the canonical named algebra for `source`.
pub(super) fn relational_plan_is_exact(
    source: &cq::Module,
    target: &relational_plan::Module,
) -> bool {
    if !source.program.query.is_basic() {
        return false;
    }
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
                        == source.program.query.head.variables().map(symbol).collect::<Vec<_>>()
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
        .inputs()
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

fn symbols<'a>(identifiers: impl IntoIterator<Item = &'a syn::Ident>) -> Vec<String> {
    identifiers.into_iter().map(symbol).collect()
}

fn same_atom(left: &cq::Atom, right: &cq::Atom) -> bool {
    symbol(&left.relation) == symbol(&right.relation) && left.args == right.args
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
            answer(x, z) :- R(x, y), S(y, z);
        })
        .unwrap();
        let exact: relational_plan::Module = syn::parse2(quote! {
            struct P;
            relation R(c0: i32, c1: i32);
            relation S(c0: i32, c1: i32);
            answer(x, z) :- R(x, y), S(y, z);
            relational {
                r0 = rename R {c0 -> x, c1 -> y};
                r1 = rename S {c0 -> y, c1 -> z};
                r2 = natural_join r0 with r1;
                r3 = project r2 keep {z, x};
                output r3 as answer(x, z);
            }
        })
        .unwrap();
        assert!(relational_plan_is_exact(&source, &exact));

        let renamed_but_locally_valid: relational_plan::Module = syn::parse2(quote! {
            struct P;
            relation R(c0: i32, c1: i32);
            relation S(c0: i32, c1: i32);
            answer(x, z) :- R(x, y), S(y, z);
            relational {
                left = rename R {c0 -> x, c1 -> y};
                right = rename S {c0 -> y, c1 -> z};
                joined = natural_join left with right;
                answer_rows = project joined keep {x, z};
                output answer_rows as answer(x, z);
            }
        })
        .unwrap();
        relational_plan::contract::check(&renamed_but_locally_valid).unwrap();
        assert!(!relational_plan_is_exact(
            &source,
            &renamed_but_locally_valid
        ));
    }
}
