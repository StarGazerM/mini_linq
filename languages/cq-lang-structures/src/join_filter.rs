//! Join filters are conservative accelerators, never independent relational reads.
//! A filter tests one scalar key column of an exact covering probe or semijoin
//! at the scan its left spine reaches; the covering operator still runs. For a
//! build B and covering key K, the scan keeps every row whose column k ∈ K lies
//! in π_k(B), a superset of the rows the covering operator can keep.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct JoinFilter {
    pub ty: syn::Type,
    pub build_token: kw::build,
    pub build: Ident,
    pub key: Level,
    pub covering_token: kw::covering,
    pub covering: Ident,
    pub marker_token: kw::marker,
    pub marker: syn::Type,
}

/// One admissible filter of a covering operator. `filtered` reports a
/// statically filtered build, which default selection requires; explicit
/// advice may add a filter on an unfiltered total build as well.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinFilterCandidate {
    pub scan: String,
    pub build: String,
    pub key: String,
    pub filtered: bool,
}

/// Every admissible filter of `covering`, one per scalar `i64` key column:
/// an eager probe or semijoin into a total single-level exact hash index or
/// sorted run, whose unique keys supply the conservative filter, and whose left
/// operand reaches a scan through an exclusive pipe spine of declarative
/// selections, projections and the left operands of eager probes/semijoins,
/// carrying the key column from the scan's lineage.
pub fn join_filter_candidates(
    program: &Program,
    facts: &Analysis,
    covering: &str,
) -> Vec<JoinFilterCandidate> {
    let defs: BTreeMap<_, _> = program
        .physical
        .program
        .definitions
        .iter()
        .map(|d| (symbol_name(&d.name), d))
        .collect();
    let fused = |name: &str| {
        facts
            .physical
            .fused
            .values()
            .any(|g| g.members.contains(name))
    };
    let eager = |k: &physical::Keyed| !matches!(k.choice, Some(physical::Choice::Lazy(_)));
    let Some(d) = defs.get(covering) else {
        return Vec::new();
    };
    let (Operator::Probe(_, k) | Operator::Semi(_, k)) = &d.operator else {
        return Vec::new();
    };
    let build = symbol_name(&k.right);
    let columns: Vec<_> = k.columns.iter().map(symbol_name).collect();
    if !eager(k)
        || fused(covering)
        || facts.physical.modes[&build] != ModeKind::Let
        || !facts.physical.rows[&build].computed.is_empty()
        || facts.built_on(&build, &columns).is_none_or(|(ty, levels)| {
            let sorted_run = matches!(ty, syn::Type::Path(path)
                if path.qself.is_none()
                    && path.path.segments.len() == 2
                    && path.path.segments[0].ident == "cq_storage"
                    && path.path.segments[1].ident == "SortedRun");
            levels != 1
                || !(*ty == syn::parse_quote!(::cq_storage::HashIndex<'_, _, _>) || sorted_run)
        })
    {
        return Vec::new();
    }
    fn total_filtered(name: &str, defs: &BTreeMap<String, &physical::Definition>) -> Option<bool> {
        let d = defs.get(name)?;
        match &d.operator {
            Operator::Rename(_) | Operator::Unit(_) => Some(false),
            Operator::Select(s) | Operator::Point(_, s) | Operator::Range(_, s) => {
                total_filtered(&symbol_name(&s.input), defs)?;
                Some(true)
            }
            Operator::Project(p) => total_filtered(&symbol_name(&p.input), defs),
            Operator::Probe(_, k) | Operator::Semi(_, k)
                if !matches!(k.choice, Some(physical::Choice::Lazy(_))) =>
            {
                Some(
                    total_filtered(&symbol_name(&k.left), defs)?
                        | total_filtered(&symbol_name(&k.right), defs)?,
                )
            }
            // A union keeps no more rows than its arms; it is filtered only
            // when every arm is.
            Operator::Union(u) => u.operands.iter().try_fold(true, |all, arm| {
                Some(all & total_filtered(&symbol_name(arm), defs)?)
            }),
            _ => None,
        }
    }
    let Some(filtered) = total_filtered(&build, &defs) else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for key in columns {
        if ungroup_type(&facts.physical.rows[&build].types[&key]) != &syn::parse_quote!(i64)
            || facts
                .filtered
                .get(&build)
                .is_some_and(|f| f.constants.iter().any(|c| symbol_name(&c.column) == key))
        {
            continue;
        }
        let mut current = symbol_name(&k.left);
        let mut consumer = covering.to_owned();
        let scan = loop {
            let Some(d) = defs.get(&current) else {
                break None;
            };
            if d.canon.is_some()
                || facts.physical.modes[&current] != ModeKind::Pipe
                || fused(&current)
                || !facts.physical.headings[&current].contains(&key)
            {
                break None;
            }
            let uses: Vec<_> = defs
                .values()
                .filter(|d| {
                    d.operator
                        .operands()
                        .iter()
                        .any(|v| symbol_name(v) == current)
                })
                .collect();
            if uses.len() != 1 || symbol_name(&uses[0].name) != consumer {
                break None;
            }
            consumer = current.clone();
            current = match &d.operator {
                Operator::Rename(_) => {
                    break facts
                        .lineage
                        .get(&current)
                        .is_some_and(|l| l.columns.contains_key(&key))
                        .then(|| current.clone());
                }
                Operator::Project(p) => symbol_name(&p.input),
                Operator::Select(s) | Operator::Point(_, s) | Operator::Range(_, s) => {
                    symbol_name(&s.input)
                }
                Operator::Probe(_, k) | Operator::Semi(_, k) if eager(k) => symbol_name(&k.left),
                _ => break None,
            };
        };
        if let Some(scan) = scan {
            result.push(JoinFilterCandidate {
                scan,
                build: build.clone(),
                key,
                filtered,
            });
        }
    }
    result
}

/// Formal rule STR-JoinFilter: A conservative scan filter is covered by an exact probe or semijoin on a key containing it, along an exclusive pipe spine.
pub(crate) fn check_join_filters(program: &Program, facts: &Analysis) -> syn::Result<()> {
    for (scan, filters) in &facts.join_filters {
        for filter in filters {
            let covering = symbol_name(&filter.covering);
            let [key] = &names(&filter.key.columns)[..] else {
                return error(&filter.covering, "join filter tests one key column");
            };
            let expected = JoinFilterCandidate {
                scan: scan.clone(),
                build: symbol_name(&filter.build),
                key: key.clone(),
                filtered: false,
            };
            if !join_filter_candidates(program, facts, &covering)
                .iter()
                .any(|c| {
                    JoinFilterCandidate {
                        filtered: false,
                        ..c.clone()
                    } == expected
                })
            {
                return error(
                    &filter.covering,
                    "join filter requires an exact covering probe or semijoin on its key, a total build and an exclusive pipe spine to its scan",
                );
            }
            concrete_type(&filter.ty)?;
            let origin = &facts.lineage[scan];
            let column = &origin.columns[key];
            let index = syn::Index::from(
                facts.host_layouts[&origin.base]
                    .iter()
                    .position(|c| c == column)
                    .unwrap(),
            );
            if filter.marker != syn::parse_quote!(::cq_load::ColumnAt<#index>) {
                return error(
                    &filter.marker,
                    "join filter marker must select the covered scalar key",
                );
            }
        }
    }
    Ok(())
}
