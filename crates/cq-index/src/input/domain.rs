//! Admitted source shape of the input-column index policy.
//! This domain is a policy premise, not part of relational well-formedness.
use cq_ir::{relational_plan, symbol_name as symbol};
use std::collections::{BTreeMap, BTreeSet};

pub fn check(source: &relational_plan::Module) -> syn::Result<()> {
    relational_plan::contract::check(source)?;
    if supported_linear_plan(source) {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(
            source,
            "input-index selection requires a Rename root, left-deep joins with distinct right Rename occurrences, and one final Project",
        ))
    }
}

fn supported_linear_plan(source: &relational_plan::Module) -> bool {
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
