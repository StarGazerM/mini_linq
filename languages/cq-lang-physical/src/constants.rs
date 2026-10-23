//! Literal-only disjointness through the shared algebra fold.
use super::*;
use cq_syntax::algebra::LiteralStep;

pub(super) fn disjoint(program: &Program) -> BTreeSet<String> {
    cq_syntax::algebra::disjoint_unions(program.program.definitions.iter().map(|definition| {
        let step = match &definition.operator {
            Operator::Unit(_) | Operator::Rename(_) => LiteralStep::Empty,
            Operator::Probe(_, join) | Operator::Merge(_, join) => {
                LiteralStep::Join(&join.left, &join.right)
            }
            Operator::Semi(_, join) | Operator::Anti(_, join) => LiteralStep::Input(&join.left),
            Operator::Select(select) | Operator::Point(_, select) | Operator::Range(_, select) => {
                LiteralStep::Select(select)
            }
            Operator::Guard(guard) => LiteralStep::Input(&guard.input),
            Operator::Bind(bind) => LiteralStep::Bind(bind),
            Operator::Project(project) => LiteralStep::Project(project),
            Operator::Group(_) => LiteralStep::Empty,
            Operator::Union(union) => LiteralStep::Union(&union.operands, union.origin.as_ref()),
        };
        (&definition.name, step)
    }))
}
