//! Independent RA-to-result-index correspondence.

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
        .map(crate::analysis::indexes::signature)
        .eq(expected)
}
