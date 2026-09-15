//! Independent source facts and local names for physical correspondence.
use super::*;

pub(super) type Information = BTreeMap<String, ra::RelationInfo>;
pub(super) type Names = BTreeMap<String, Ident>;

/// Derived relation/name environments used only by the independent judgments.
pub(super) struct RuleContext<'a> {
    pub(super) source: &'a ir::Module,
    pub(super) info: Information,
    pub(super) variables: Names,
    pub(super) inputs: Names,
    pub(super) results: Names,
}

impl<'a> RuleContext<'a> {
    pub(super) fn new(
        source: &'a ir::Module,
        result_name: impl Fn(usize) -> Ident,
    ) -> Option<Self> {
        let module = &source.relational_plan;
        let info = ra::check_in(module.program.inputs(), &module.plan).ok()?;
        let variables = info
            .values()
            .flat_map(|r| r.heading.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .enumerate()
            .map(|(i, name)| (name, format_ident!("v{i}")))
            .collect();
        let inputs = module
            .program
            .inputs()
            .enumerate()
            .map(|(i, d)| (symbol_name(&d.name), format_ident!("input{i}")))
            .collect();
        let results = module
            .plan
            .definitions
            .iter()
            .enumerate()
            .map(|(i, d)| (symbol_name(&d.result), result_name(i)))
            .collect();
        Some(Self {
            source,
            info,
            variables,
            inputs,
            results,
        })
    }
}
pub(super) fn join_key(info: &Information, left: &Ident, right: &Ident) -> Vec<usize> {
    info[&symbol_name(right)]
        .heading
        .iter()
        .enumerate()
        .filter_map(|(i, v)| info[&symbol_name(left)].heading.contains(v).then_some(i))
        .collect()
}
