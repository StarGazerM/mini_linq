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
    pub policy: Policy,
}

/// The one recorded activation decision of a join filter (STR-FilterPolicy).
/// `policy always` builds the conservative filter unconditionally;
/// `policy abandon P of B keys S` activates it only while the covering build's
/// exact key count S is at most P% of the positional base B. `keys key_count`
/// reads `cq_storage::Keys::key_count` (constant cost; it completes a deferred
/// build); `keys ACCESSOR` reads a declared optional exact statistic whose
/// absence selects identity. Representation stays a provider-token fact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Policy {
    pub policy_token: kw::policy,
    pub activation: Activation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Activation {
    Always(kw::always),
    Abandon(Abandon),
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Abandon {
    pub abandon_token: kw::abandon,
    pub percent: syn::LitInt,
    pub of_token: kw::of,
    pub base: Ident,
    pub keys_token: kw::keys,
    pub keys: KeySource,
}

/// The exact build key count an abandon policy compares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeySource {
    KeyCount(kw::key_count),
    Statistic(Ident),
}

impl Policy {
    pub fn always() -> Self {
        syn::parse_quote!(policy always)
    }

    pub fn abandon(&self) -> Option<&Abandon> {
        match &self.activation {
            Activation::Abandon(abandon) => Some(abandon),
            Activation::Always(_) => None,
        }
    }

    /// The optional statistic accessor this policy reads, if any.
    pub fn statistic(&self) -> Option<&Ident> {
        match self.abandon().map(|abandon| &abandon.keys) {
            Some(KeySource::Statistic(accessor)) => Some(accessor),
            _ => None,
        }
    }
}

impl Abandon {
    pub fn percent(&self) -> usize {
        self.percent.base10_parse().expect("checked percent")
    }
}

impl syn::parse::Parse for Policy {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let policy_token = input.parse()?;
        let activation = if input.peek(kw::always) {
            Activation::Always(input.parse()?)
        } else {
            Activation::Abandon(input.parse()?)
        };
        Ok(Self { policy_token, activation })
    }
}

impl ToTokens for Policy {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.policy_token.to_tokens(tokens);
        match &self.activation {
            Activation::Always(always) => always.to_tokens(tokens),
            Activation::Abandon(abandon) => abandon.to_tokens(tokens),
        }
    }
}

impl syn::parse::Parse for KeySource {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        if input.peek(kw::key_count) {
            input.parse().map(Self::KeyCount)
        } else {
            input.parse().map(Self::Statistic)
        }
    }
}

impl ToTokens for KeySource {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        match self {
            Self::KeyCount(key_count) => key_count.to_tokens(tokens),
            Self::Statistic(accessor) => accessor.to_tokens(tokens),
        }
    }
}

/// The abandon threshold join-filter selection records (RPT+'s creation
/// threshold): a build keeping more than this percent of its population
/// rarely rejects probe-side rows.
pub const JOIN_FILTER_ABANDON_PERCENT: usize = 35;

/// Formal rule SEL-FilterPolicy: record every join filter's one policy from
/// the final positional bases. A filter whose build has a positional lineage
/// base abandons above [`JOIN_FILTER_ABANDON_PERCENT`] of that base's rows,
/// comparing `Keys::key_count`, or the provider's declared exact Keys statistic
/// when authored advice selected a borrowed or attached realization; otherwise
/// its population is unknown and the policy is `always`. This measured
/// incumbent is the only automatic policy: P7.3's cost rule may only veto it,
/// and recorded evidence supports no veto (`doc/PLANNING-REDESIGN.md` P7).
/// Selecting passes call this after their last join-filter or positional
/// decision.
pub fn record_join_filter_policies(physical: &crate::physical::Program, program: &mut Program) -> syn::Result<()> {
    for entry in &mut program.structures.entries {
        if let Structure::JoinFilter { filter, .. } = &mut entry.structure {
            filter.policy = Policy::always();
        }
    }
    let facts = contract::check(physical, program)?;
    let positional = positional_bases(&facts);
    let percent = proc_macro2::Literal::usize_unsuffixed(JOIN_FILTER_ABANDON_PERCENT);
    for entry in &mut program.structures.entries {
        let Structure::JoinFilter { filter, .. } = &mut entry.structure else {
            continue;
        };
        let Some(origin) = facts.lineage.get(&symbol_name(&filter.build)) else {
            continue;
        };
        if !positional.contains(&origin.base) {
            continue;
        }
        let base = Ident::new(&origin.base, proc_macro2::Span::call_site());
        let realization = facts.facts(&filter.provider);
        let statistic = (realization.borrowed_filter || realization.attached_filter)
            .then(|| {
                let definition = physical.program.definitions.iter()
                    .find(|definition| definition.name == filter.covering)?;
                let (Operator::Probe(_, keyed) | Operator::Semi(_, keyed)) = &definition.operator else {
                    return None;
                };
                let (provider, _) = facts.built_on(&symbol_name(&filter.build), &names(&keyed.columns))?;
                facts.facts(provider).statistics.iter().find(|statistic|
                    statistic.field == stats::Field::Keys
                        && statistic.precision == stats::Precision::Exact
                        && statistic.placement == stats::Placement::Host
                        && statistic.computation == stats::Computation::Build)
                    .map(|statistic| statistic.accessor.clone())
            })
            .flatten();
        filter.policy = match statistic {
            Some(accessor) => syn::parse_quote!(policy abandon #percent of #base keys #accessor),
            None => syn::parse_quote!(policy abandon #percent of #base keys key_count),
        };
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
/// `cq_storage::Keys` obligation, checked by rustc. The default projects these
/// keys only after activation; explicit attached advice observes exact construction,
/// and explicit borrowed advice requires a complete scalar run layout without
/// key allocation. These recorded facts, never provider names, select realization.
/// The left operand reaches a scan through an
/// exclusive pipe spine of selections, projections and eager probes/semijoins,
/// carrying the key column from the scan's lineage.
pub fn join_filter_candidates(
    physical: &crate::physical::Program,
    _program: &Program,
    facts: &Analysis,
    covering: &str,
) -> Vec<JoinFilterCandidate> {
    let defs: BTreeMap<_, _> = physical
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

/// Formal rule STR-JoinFilter: A conservative scan filter is covered by an exact probe or semijoin on a key containing it, along an exclusive pipe spine; projected construction is the default, while explicit borrowed and attached realizations are mutually exclusive and borrowing requires a complete scalar run key.
pub(crate) fn check_join_filters(physical: &crate::physical::Program, program: &Program, facts: &Analysis) -> syn::Result<()> {
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
            if !join_filter_candidates(physical, program, facts, &covering)
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
            if facts.facts(&filter.provider).borrowed_filter {
                let build = symbol_name(&filter.build);
                let (Operator::Probe(_, keyed) | Operator::Semi(_, keyed)) = &physical.program.definitions.iter()
                    .find(|definition| symbol_name(&definition.name) == covering)
                    .expect("checked covering operator").operator else { unreachable!() };
                let columns = names(&keyed.columns);
                let Some((provider, _)) = facts.built_on(&build, &columns) else {
                    return error(filter, "STR-JoinFilter: borrowed filter requires its selected run");
                };
                if columns != [key.clone()] || !facts.facts(provider).run_filter {
                    return error(filter, "STR-JoinFilter: borrowed filter requires a declared run view on its complete scalar key");
                }
            }
            check_policy(physical, program, facts, filter)?;
        }
    }
    Ok(())
}

/// Formal rule STR-FilterPolicy: every join filter records exactly one policy;
/// `abandon P of B` needs 1 ≤ P ≤ 100 and B the build's positional lineage base,
/// and an optional statistic accessor must be the covering build provider's
/// declared exact host Keys statistic. Legality only: no selection is rerun.
fn check_policy(physical: &crate::physical::Program, _program: &Program, facts: &Analysis, filter: &JoinFilter) -> syn::Result<()> {
    let Some(abandon) = filter.policy.abandon() else {
        return Ok(());
    };
    let percent: usize = abandon.percent.base10_parse()?;
    let build = symbol_name(&filter.build);
    let definition = physical.program.definitions.iter()
        .find(|definition| definition.name == filter.covering).expect("checked cover");
    let (Operator::Probe(_, keyed) | Operator::Semi(_, keyed)) = &definition.operator else { unreachable!() };
    let Some((provider, _)) = facts.built_on(&build, &names(&keyed.columns)) else {
        return error(abandon, "STR-FilterPolicy: activation requires selected build");
    };
    if let Some(accessor) = filter.policy.statistic()
        && !facts.facts(provider).statistics.iter().any(|statistic|
            statistic.field == stats::Field::Keys
                && statistic.precision == stats::Precision::Exact
                && statistic.placement == stats::Placement::Host
                && statistic.computation == stats::Computation::Build
                && statistic.accessor == *accessor)
    {
        return error(abandon, "STR-FilterPolicy: activation requires declared exact optional key statistic");
    }
    let population = facts.lineage.get(&build).map(|origin| &origin.base);
    if !(1..=100).contains(&percent)
        || population != Some(&symbol_name(&abandon.base))
        || !positional_bases(facts).contains(&symbol_name(&abandon.base))
    {
        return error(
            abandon,
            "STR-FilterPolicy: a join filter abandons at a percent of its build's positional lineage base",
        );
    }
    Ok(())
}

/// Covering operators are proof witnesses, not runtime filter identities.
/// Validate every recorded witness before sharing its runtime work. A build
/// name identifies one Physical version; its complete covering key identifies
/// the source index layout (and therefore the projected key component).
pub(crate) fn coalesce_join_filters(physical: &crate::physical::Program, _program: &Program, facts: &mut Analysis) {
    if facts.join_filters.is_empty() {
        return;
    }
    let layouts: BTreeMap<_, _> = physical
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
                    && previous.policy == filter.policy
                    && layouts[&symbol_name(&previous.covering)]
                        == layouts[&symbol_name(&filter.covering)]
            }) {
                continue;
            }
            filters.push(filter);
        }
    }
}
