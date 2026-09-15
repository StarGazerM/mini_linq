//! Domain judgment for the expression-preserving lazy realization.
use cq_ir::{relational_plan, symbol_name};
use std::collections::BTreeSet;
/// Logical dataflow domain of the default index edge; independent of storage.
pub fn is_linear_prefix(source: &relational_plan::Module) -> bool {
    if !relational_plan::contract::well_formed(source) {
        return false;
    }
    let mut current = None::<String>;
    let mut pending = BTreeSet::new();
    let mut projected = false;
    for d in &source.plan.definitions {
        if projected {
            return false;
        }
        let name = symbol_name(&d.result);
        match &d.operator {
            relational_plan::Operator::Unit(_) if current.is_none() => current = Some(name),
            relational_plan::Operator::Rename(_) if current.is_none() => current = Some(name),
            relational_plan::Operator::Rename(_) => {
                pending.insert(name);
            }
            relational_plan::Operator::NaturalJoin(j)
                if current.as_deref() == Some(&symbol_name(&j.left))
                    && pending.remove(&symbol_name(&j.right)) =>
            {
                current = Some(name)
            }
            relational_plan::Operator::Select(s)
                if current.as_deref() == Some(&symbol_name(&s.input)) =>
            {
                current = Some(name)
            }
            relational_plan::Operator::Extend(e)
                if current.as_deref() == Some(&symbol_name(&e.input)) =>
            {
                current = Some(name)
            }
            relational_plan::Operator::Project(p)
                if current.as_deref() == Some(&symbol_name(&p.input)) && pending.is_empty() =>
            {
                projected = true;
                current = Some(name);
            }
            _ => return false,
        }
    }
    projected && current.as_deref() == Some(&symbol_name(&source.plan.output.input))
}
