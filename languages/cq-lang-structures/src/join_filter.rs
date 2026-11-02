//! Join filters are conservative accelerators, never independent relational reads.
//! A filter tests one scalar key column of an exact covering probe or semijoin
//! at the scan its left spine reaches; the covering operator still runs. For a
//! build B and covering key K, the scan keeps every row whose column k ∈ K lies
//! in π_k(B), a superset of the rows the covering operator can keep.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct JoinFilter {
    pub provider: Provider,
    pub build_token: kw::build,
    pub build: Ident,
    pub key: Level,
    pub covering_token: kw::covering,
    pub covering: Ident,
    pub marker_token: kw::marker,
    pub marker: syn::Type,
    #[parse(parse_abandon)]
    pub abandon: Option<Abandon>,
}

/// Formal rule IF-JoinFilterAbandon, recorded: construct no filter from a
/// build keeping more than `percent`% of the rows of `base`, the positional
/// base of the build's occurrence lineage (its population). Absent, the
/// filter is always built.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Abandon {
    pub abandon_token: kw::abandon,
    pub percent: syn::LitInt,
    pub of_token: kw::of,
    pub base: Ident,
}

impl Abandon {
    pub fn percent(&self) -> usize {
        self.percent.base10_parse().expect("checked percent")
    }
}

fn parse_abandon(input: syn::parse::ParseStream<'_>) -> syn::Result<Option<Abandon>> {
    if input.peek(kw::abandon) {
        input.parse().map(Some)
    } else {
        Ok(None)
    }
}

/// The abandon threshold join-filter selection records (RPT+'s creation
/// threshold): a build keeping more than this percent of its population
/// rarely rejects probe-side rows.
pub const JOIN_FILTER_ABANDON_PERCENT: usize = 35;

/// Formal rule SEL-JoinFilterAbandon: record every join filter's abandon
/// policy from the final positional bases. A filter whose build has a
/// positional lineage base abandons above [`JOIN_FILTER_ABANDON_PERCENT`] of
/// that base's rows; otherwise its population is unknown and it is always
/// built. Selecting passes call this after their last join-filter or
/// positional decision.
pub fn record_join_filter_abandon(program: &mut Program) -> syn::Result<()> {
    for entry in &mut program.structures.entries {
        if let Structure::JoinFilter { filter, .. } = &mut entry.structure {
            filter.abandon = None;
        }
    }
    let facts = contract::check(program)?;
    let positional = positional_bases(&facts);
    let percent = proc_macro2::Literal::usize_unsuffixed(JOIN_FILTER_ABANDON_PERCENT);
    for entry in &mut program.structures.entries {
        let Structure::JoinFilter { filter, .. } = &mut entry.structure else {
            continue;
        };
        let Some(origin) = facts.lineage.get(&symbol_name(&filter.build)) else {
            continue;
        };
        if positional.contains(&origin.base) {
            let base = Ident::new(&origin.base, proc_macro2::Span::call_site());
            filter.abandon = Some(syn::parse_quote!(abandon #percent of #base));
        }
    }
    Ok(())
}

/// Bases read positionally: the bases of recorded reads, selections, key
/// sets, join-filtered scans and position representations.
pub fn positional_bases(analysis: &Analysis) -> BTreeSet<String> {
    analysis
        .reads
        .values()
        .map(|r| symbol_name(&r.origin.base))
        .chain(
            analysis
                .selections
                .values()
                .map(|s| symbol_name(&s.origin.base)),
        )
        .chain(
            analysis
                .keysets
                .values()
                .map(|s| symbol_name(&s.origin.base)),
        )
        .chain(
            analysis
                .join_filters
                .keys()
                .map(|scan| analysis.lineage[scan].base.clone()),
        )
        .chain(
            analysis
                .repr
                .values()
                .filter_map(|repr| match repr {
                    Repr::Positions { bases, .. } => Some(bases),
                    Repr::Tuples(_) => None,
                })
                .flat_map(|bases| bases.iter().map(|base| symbol_name(&base.base))),
        )
        .collect()
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

/// Every admissible filter of `covering`, one per scalar key column:
/// an eager probe or semijoin into a total single-level exact provider with
/// declared distinct-key enumeration. Key enumeration is the provider's
/// `cq_storage::Keys` obligation, checked by rustc. Automatic selection may
/// apply a narrower cost policy; explicit providers retain this
/// representation-independent contract. The left operand reaches a scan through an
/// exclusive pipe spine of selections, projections and eager probes/semijoins,
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
        || facts
            .membership
            .contains(&(build.clone(), names(&k.columns)))
        || !facts.provider_support(&build, &columns, None, ProviderCapability::Keys)
        || facts
            .selected_built(&build, &columns)
            .is_none_or(|((levels, _), depth)| depth != 1 || levels.len() != 1)
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
        if facts
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
            concrete_type(&filter.marker)?;
            if let Some(abandon) = &filter.abandon {
                let percent: usize = abandon.percent.base10_parse()?;
                let population = facts
                    .lineage
                    .get(&symbol_name(&filter.build))
                    .map(|origin| &origin.base);
                if !(1..=100).contains(&percent)
                    || population != Some(&symbol_name(&abandon.base))
                    || !positional_bases(facts).contains(&symbol_name(&abandon.base))
                {
                    return error(
                        abandon,
                        "a join filter abandons at a percent of its build's positional lineage base",
                    );
                }
            }
        }
    }
    Ok(())
}

/// Covering operators are proof witnesses, not runtime filter identities.
/// Validate every recorded witness before sharing its runtime work. A build
/// name identifies one Physical version; its complete covering key identifies
/// the source index layout (and therefore the projected key component).
pub(crate) fn coalesce_join_filters(program: &Program, facts: &mut Analysis) {
    if facts.join_filters.is_empty() {
        return;
    }
    let layouts: BTreeMap<_, _> = program
        .physical
        .program
        .definitions
        .iter()
        .filter_map(|d| {
            let (Operator::Probe(_, k) | Operator::Semi(_, k)) = &d.operator else {
                return None;
            };
            Some((symbol_name(&d.name), names(&k.columns)))
        })
        .collect();
    for filters in facts.join_filters.values_mut() {
        let witnesses = std::mem::take(filters);
        for filter in witnesses {
            if filters.iter().any(|previous| {
                symbol_name(&previous.build) == symbol_name(&filter.build)
                    && names(&previous.key.columns) == names(&filter.key.columns)
                    && previous.provider.same_token(&filter.provider)
                    && ungroup_type(&previous.provider.ty) == ungroup_type(&filter.provider.ty)
                    && ungroup_type(&previous.marker) == ungroup_type(&filter.marker)
                    && previous.abandon == filter.abandon
                    && layouts[&symbol_name(&previous.covering)]
                        == layouts[&symbol_name(&filter.covering)]
            }) {
                continue;
            }
            filters.push(filter);
        }
    }
}
