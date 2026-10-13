//! Join filters are conservative accelerators, never independent relational reads.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum FilterKind {
    #[parse(peek = kw::exact)]
    Exact(kw::exact),
    #[parse(peek = kw::bloom)]
    Bloom(kw::bloom),
}
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct JoinFilter {
    pub kind: FilterKind,
    pub ty: syn::Type,
    pub build_token: kw::build,
    pub build: Ident,
    pub key: Level,
    pub covering_token: kw::covering,
    pub covering: Ident,
    pub marker_token: kw::marker,
    pub marker: syn::Type,
}

/// Static candidate: a single-use declarative stream and a filtered, total build.
/// The bool selects exact membership for an occurrence-preserving bounded build;
/// joined build branches use a fixed-budget Bloom and keep the exact probe.
pub fn join_filter_candidate(program: &Program, facts: &Analysis, probe: &str) -> Option<(String, String, String, bool)> {
    let defs: BTreeMap<_, _> = program.physical.program.definitions.iter().map(|d| (symbol_name(&d.name), d)).collect();
    let d = *defs.get(probe)?;
    let Operator::Probe(_, k) = &d.operator else { return None };
    if k.columns.len() != 1 || facts.physical.fused.values().any(|g| g.members.contains(probe)) { return None; }
    let key = symbol_name(k.columns.first()?);
    let build = symbol_name(&k.right);
    if facts.physical.modes[&build] != ModeKind::Let
        || ungroup_type(&facts.physical.rows[&build].types[&key]) != &syn::parse_quote!(i64)
        || !facts.physical.rows[&build].computed.is_empty()
        || facts.filtered.get(&build).is_some_and(|f| f.constants.iter().any(|c| symbol_name(&c.column) == key))
    { return None; }
    let (ty, levels) = facts.built_on(&build, std::slice::from_ref(&key))?;
    if levels != 1 || *ty != syn::parse_quote!(::cq_storage::HashIndex<'_, _, _>) { return None; }
    fn total_filtered(name: &str, defs: &BTreeMap<String, &physical::Definition>) -> Option<bool> {
        let d = defs.get(name)?;
        match &d.operator {
            Operator::Rename(_) | Operator::Unit(_) => Some(false),
            Operator::Select(s) | Operator::Point(_, s) | Operator::Range(_, s) => { total_filtered(&symbol_name(&s.input), defs)?; Some(true) }
            Operator::Project(p) => total_filtered(&symbol_name(&p.input), defs),
            Operator::Probe(_, k) | Operator::Semi(_, k) if !matches!(k.choice, Some(physical::Choice::Lazy(_))) => Some(total_filtered(&symbol_name(&k.left), defs)? | total_filtered(&symbol_name(&k.right), defs)?),
            _ => None,
        }
    }
    if !total_filtered(&build, &defs)? { return None; }
    let exact = facts.lineage.contains_key(&build);
    let mut current = symbol_name(&k.left);
    let mut consumer = probe.to_owned();
    loop {
        let d = *defs.get(&current)?;
        if d.canon.is_some() || facts.physical.modes[&current] != ModeKind::Pipe
            || facts.physical.fused.values().any(|g| g.members.contains(&current))
            || !facts.lineage.contains_key(&current)
        { return None; }
        let uses: Vec<_> = defs.values().filter(|d| d.operator.operands().iter().any(|v| symbol_name(v) == current)).collect();
        if uses.len() != 1 || symbol_name(&uses[0].name) != consumer { return None; }
        consumer = current.clone();
        match &d.operator {
            Operator::Rename(_) => return Some((current, build, key, exact)),
            Operator::Project(p) => current = symbol_name(&p.input),
            Operator::Select(s) | Operator::Point(_, s) | Operator::Range(_, s) => current = symbol_name(&s.input),
            _ => return None,
        }
    }
}

/// Formal rule STR-JoinFilter: A conservative scan filter is covered by the same-key exact probe along an exclusive total spine.
pub(crate) fn check_join_filters(program: &Program, facts: &Analysis) -> syn::Result<()> {
    for (scan, filter) in &facts.join_filters {
        let probe = symbol_name(&filter.covering);
        let Some((expected_scan, build, key, exact)) = join_filter_candidate(program, facts, &probe) else {
            return error(&filter.covering, "join filter requires a covering exact probe, total filtered build and exclusive declarative scan spine");
        };
        if *scan != expected_scan || symbol_name(&filter.build) != build || names(&filter.key.columns) != [key.clone()]
            || matches!(filter.kind, FilterKind::Exact(_)) != exact {
            return error(&filter.covering, "join filter build/key/representation must match its exact covering probe and static bound");
        }
        let expected_type: syn::Type = if exact {
            syn::parse_quote!(::cq_storage::AdaptiveKeySet<i64, ::cq_storage::I64Codec, 67108864>)
        } else {
            syn::parse_quote!(::cq_storage::BloomFilter<16384>)
        };
        if filter.ty != expected_type {
            return error(&filter.ty, "join filter must record its scalar provider, codec and fixed budget");
        }
        let origin = &facts.lineage[scan];
        let column = &origin.columns[&key];
        let index = syn::Index::from(facts.host_layouts[&origin.base].iter().position(|c| c == column).unwrap());
        if filter.marker != syn::parse_quote!(::cq_load::ColumnAt<#index>) {
            return error(&filter.marker, "join filter marker must select the covered scalar key");
        }
    }
    Ok(())
}
