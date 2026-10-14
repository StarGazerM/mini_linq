//! Literal-only disjointness through the shared algebra fold.
use super::*;
use cq_syntax::algebra::LiteralStep;

pub fn disjoint_unions(program: &Program) -> syn::Result<BTreeSet<String>> {
    contract::check(program)?;
    Ok(cq_syntax::algebra::disjoint_unions(
        program.plan.definitions.iter().map(|definition| {
            let step = match &definition.operator {
                Operator::Unit(_) | Operator::Rename(_) => LiteralStep::Empty,
                Operator::NaturalJoin(join) => LiteralStep::Join(&join.left, &join.right),
                Operator::Semijoin(join) => LiteralStep::Input(&join.left),
                Operator::Select(select) => LiteralStep::Select(select),
                Operator::Guard(guard) => LiteralStep::Input(&guard.input),
                Operator::Bind(bind) => LiteralStep::Bind(bind),
                Operator::Project(project) => LiteralStep::Project(project),
                Operator::Union(union) => {
                    LiteralStep::Union(&union.operands, union.origin.as_ref())
                }
                Operator::Anti(anti) => LiteralStep::Input(&anti.left),
            };
            (&definition.result, step)
        }),
    ))
}
