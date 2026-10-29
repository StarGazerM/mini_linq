//! Lookup-before-Expand scheduling (Birler et al. §3.2–3.4, §4.2), with
//! LargestRoot (Zhao et al. Lemma 3.2) and ATP's deepest-parent tie break
//! (Qiao et al. §3). Cardinality advice chooses plans, never proof premises.
//! Retained join leaves replace their keyed fallbacks; live smallest
//! intersections keep both their candidate and full-key membership adapters.
//! Raw membership certificates survive only unchanged complete-key read domains;
//! newly introduced prefix reads use generated exact indexes instead.
//! Distinct full-row index layouts receive separate identity-projection stores;
//! their consumers name a provider explicitly while sharing retained source rows.
use std::collections::{BTreeMap, BTreeSet};

use cq_lang_sequence::{Capability, Catalog, Program, structures};
use cq_syntax::symbol_name;
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use structures::physical::{self, Operator};
use syn::parse::{Parse, ParseStream};
use syn::{Ident, Token};

mod kw {
    syn::custom_keyword!(using);
    syn::custom_keyword!(order);
    syn::custom_keyword!(transfer);
    syn::custom_keyword!(early);
    syn::custom_keyword!(providers);
}

/// Concrete, replayable decisions. `parents[i]` indexes `order`, not Physical
/// definition order. Expands may be postponed until a later key consumer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LookupPlan {
    pub order: Vec<Ident>,
    pub parents: Vec<Option<usize>>,
    pub transfer: Vec<Ident>,
    pub early: Vec<Ident>,
}

struct Graph {
    root: Ident,
    leaves: Vec<Ident>,
    joins: BTreeSet<String>,
    headings: BTreeMap<String, Vec<String>>,
    predicate: BTreeSet<String>,
    filtered: BTreeSet<String>,
    scan_roots: BTreeSet<String>,
    hosts: BTreeMap<String, String>,
    renames: BTreeMap<String, BTreeMap<String, String>>,
}

fn intersection(left: &[String], right: &[String]) -> Vec<String> {
    left.iter()
        .filter(|column| right.contains(column))
        .cloned()
        .collect()
}

fn graph(
    checker: &mut cq_lang_sequence::contract::Checker,
    source: &Program,
) -> syn::Result<Option<Graph>> {
    let facts = checker.check_shared(source)?;
    let physical = &source.structures.physical.program;
    let definitions: BTreeMap<_, _> = physical
        .definitions
        .iter()
        .map(|d| (symbol_name(&d.name), d))
        .collect();
    let mut root = physical.returned.clone();
    // These suffixes keep their exact predicates/projection. In particular,
    // opaque Rust is not pulled through a newly filtered or expanded stream.
    while let Some(definition) = definitions.get(&symbol_name(&root)) {
        match &definition.operator {
            Operator::Project(p) => root = p.input.clone(),
            Operator::Select(s) => root = s.input.clone(),
            _ => break,
        }
    }
    if !matches!(
        definitions[&symbol_name(&root)].operator,
        Operator::Probe(..) | Operator::Merge(..)
    ) {
        return Ok(None);
    }
    fn collect(
        name: &Ident,
        definitions: &BTreeMap<String, &physical::Definition>,
        leaves: &mut Vec<Ident>,
        joins: &mut BTreeSet<String>,
    ) {
        let key = symbol_name(name);
        if let Operator::Probe(_, join) | Operator::Merge(_, join) = &definitions[&key].operator {
            joins.insert(key);
            collect(&join.left, definitions, leaves, joins);
            collect(&join.right, definitions, leaves, joins);
            return;
        }
        if !leaves.contains(name) {
            leaves.push(name.clone());
        }
    }
    let mut leaves = Vec::new();
    let mut joins = BTreeSet::new();
    collect(&root, &definitions, &mut leaves, &mut joins);
    if leaves.len() < 2 {
        return Ok(None);
    }
    // A lazy host read is an authored reusable scan. Retaining its right side
    // would silently replace that execution choice with an eager index build.
    if physical.definitions.iter().any(|definition| {
        matches!(&definition.operator, Operator::Semi(_, keyed) | Operator::Anti(_, keyed)
            if matches!(keyed.choice, Some(physical::Choice::Lazy(_)))
                && leaves.contains(&keyed.right))
    }) {
        return Ok(None);
    }
    // A shared interior result is a separate consumer, not a tree edge that
    // this rewrite may delete. Its region is deliberately left to its author.
    if physical.definitions.iter().any(|d| {
        !joins.contains(&symbol_name(&d.name))
            && d.operator
                .operands()
                .iter()
                .any(|input| joins.contains(&symbol_name(input)) && **input != root)
    }) {
        return Ok(None);
    }
    // Estimates remain author-visible. Do not silently drop facts whose named
    // intermediate would disappear, or retarget them to a different relation.
    use cq_lang_sequence::EvidenceFact;
    let removed = |name: &Ident| joins.contains(&symbol_name(name)) && *name != root;
    if source.evidence.facts.iter().any(|fact| match fact {
        EvidenceFact::Count { subject, .. }
        | EvidenceFact::Key { subject, .. }
        | EvidenceFact::Selectivity { subject, .. }
        | EvidenceFact::Bounds { subject, .. }
        | EvidenceFact::Sorted { subject, .. } => removed(subject),
        EvidenceFact::ForeignKey {
            subject,
            referenced,
            ..
        } => removed(subject) || removed(referenced),
    }) {
        return Ok(None);
    }
    let filtered: BTreeSet<_> = facts.structures.filtered.keys().cloned().collect();
    // A projected index is not itself enumerable. Its original pure producer
    // can nevertheless become the scan root when replacing this join region
    // removes every consumer of that index. Reuse the producer and its scan
    // adapters, not the provider's truncated payload. Shared providers retain
    // their authored representation.
    let scan_roots: BTreeSet<_> = leaves
        .iter()
        .map(symbol_name)
        .filter(|name| {
            !filtered.contains(name)
                || (matches!(
                    definitions[name].operator,
                    Operator::Rename(_)
                        | Operator::Select(_)
                        | Operator::Point(_, _)
                        | Operator::Range(_, _)
                ) && physical.definitions.iter().all(|definition| {
                    joins.contains(&symbol_name(&definition.name))
                        || definition
                            .operator
                            .operands()
                            .iter()
                            .all(|input| symbol_name(input) != *name)
                }))
        })
        .collect();
    if scan_roots.is_empty() {
        return Ok(None);
    }
    let mut predicate = BTreeSet::new();
    for leaf in &leaves {
        let mut at = leaf;
        loop {
            let definition = definitions[&symbol_name(at)];
            match &definition.operator {
                Operator::Select(_)
                | Operator::Point(_, _)
                | Operator::Range(_, _)
                | Operator::Guard(_) => {
                    predicate.insert(symbol_name(leaf));
                    break;
                }
                Operator::Rename(rename) if rename.constants.is_some() => {
                    predicate.insert(symbol_name(leaf));
                    break;
                }
                Operator::Project(project) => at = &project.input,
                _ => break,
            }
        }
    }
    Ok(Some(Graph {
        root,
        leaves,
        joins,
        predicate,
        filtered,
        scan_roots,
        headings: facts.structures.physical.headings.clone(),
        hosts: facts.structures.physical.hosts.clone(),
        renames: facts.structures.physical.renames.clone(),
    }))
}

fn count(source: &Program, name: &Ident) -> Option<usize> {
    let definitions = &source.structures.physical.program.definitions;
    let mut at = name;
    loop {
        if let Some(count) = source.evidence.count(&symbol_name(at)) {
            return Some(count);
        }
        let definition = definitions.iter().find(|d| d.name == *at)?;
        if let Operator::Rename(rename) = &definition.operator {
            return source.evidence.count(&symbol_name(&rename.relation));
        }
        at = definition.operator.input()?;
    }
}

fn key_evidence(source: &Program, graph: &Graph, name: &Ident, keys: &[String]) -> bool {
    let mut at = name;
    loop {
        let name = symbol_name(at);
        if source.evidence.has_key(&name, keys) {
            return true;
        }
        if let Some(host) = graph.hosts.get(&name) {
            let mapped: Option<Vec<_>> = keys
                .iter()
                .map(|key| graph.renames[&name].get(key).cloned())
                .collect();
            return mapped.is_some_and(|keys| source.evidence.has_key(host, &keys));
        }
        let Some(definition) = source
            .structures
            .physical
            .program
            .definitions
            .iter()
            .find(|d| d.name == *at)
        else {
            return false;
        };
        match &definition.operator {
            Operator::Select(s) | Operator::Point(_, s) | Operator::Range(_, s) => at = &s.input,
            Operator::Project(p) => at = &p.input,
            _ => return false,
        }
    }
}

fn plan(source: &Program, graph: &Graph) -> LookupPlan {
    let heading = |i: usize| &graph.headings[&symbol_name(&graph.leaves[i])];
    let mut root = graph
        .leaves
        .iter()
        .position(|leaf| graph.scan_roots.contains(&symbol_name(leaf)))
        .expect("scan-compatible root checked");
    for i in 0..graph.leaves.len() {
        if graph.scan_roots.contains(&symbol_name(&graph.leaves[i]))
            && count(source, &graph.leaves[i]) > count(source, &graph.leaves[root])
        {
            root = i;
        }
    }
    let mut order = vec![root];
    let mut parents = vec![None];
    let mut depths = vec![0usize];
    while order.len() < graph.leaves.len() {
        let mut best: Option<(usize, usize, usize)> = None;
        for child in 0..graph.leaves.len() {
            if order.contains(&child) {
                continue;
            }
            for (parent, &existing) in order.iter().enumerate() {
                let weight = intersection(heading(child), heading(existing)).len();
                if weight == 0 {
                    continue;
                }
                let score = (weight, count(source, &graph.leaves[child]), depths[parent]);
                if best.is_none_or(|(old_child, old_parent, old_weight)| {
                    score
                        > (
                            old_weight,
                            count(source, &graph.leaves[old_child]),
                            depths[old_parent],
                        )
                }) {
                    best = Some((child, parent, weight));
                }
            }
        }
        // Disconnected components are Cartesian products. Their deterministic
        // root is still legal, but no predicate is transferred across them.
        let (child, parent) = best.map_or_else(
            || {
                (
                    (0..graph.leaves.len())
                        .find(|i| !order.contains(i))
                        .expect("remaining node"),
                    None,
                )
            },
            |(child, parent, _)| (child, Some(parent)),
        );
        depths.push(parent.map_or(0, |parent| depths[parent] + 1));
        order.push(child);
        parents.push(parent);
    }
    let order: Vec<_> = order.into_iter().map(|i| graph.leaves[i].clone()).collect();
    let mut transfer = Vec::new();
    let mut early = Vec::new();
    for (i, name) in order.iter().enumerate() {
        let key = symbol_name(name);
        let mut classes = BTreeSet::new();
        if let Some(parent) = parents[i] {
            classes.insert(intersection(
                &graph.headings[&key],
                &graph.headings[&symbol_name(&order[parent])],
            ));
        }
        for (child, parent) in parents.iter().enumerate() {
            if *parent == Some(i) {
                classes.insert(intersection(
                    &graph.headings[&key],
                    &graph.headings[&symbol_name(&order[child])],
                ));
            }
        }
        let worthy = source
            .evidence
            .selectivity(&key)
            .is_none_or(|s| u128::from(s.numerator) * 100 < u128::from(s.denominator) * 35);
        if (graph.predicate.contains(&key) && worthy) || classes.len() > 1 {
            transfer.push(name.clone());
        }
        if parents[i].is_some_and(|parent| {
            let keys = intersection(
                &graph.headings[&key],
                &graph.headings[&symbol_name(&order[parent])],
            );
            key_evidence(source, graph, name, &keys)
        }) {
            early.push(name.clone());
        }
    }
    LookupPlan {
        order,
        parents,
        transfer,
        early,
    }
}

/// Extract the maximal terminal join region and its LargestRoot/ATP decisions.
/// `None` denotes no movable region (including a shared-result boundary).
pub fn lookup_plan(source: &Program) -> syn::Result<Option<LookupPlan>> {
    Ok(graph(&mut Default::default(), source)?
        .as_ref()
        .map(|graph| plan(source, graph)))
}

/// Author-visible, serializable Lookup/Expand decisions. Omitting a list asks
/// the same deterministic evidence consumer used by `lookup_first_advice`.
/// Provider overrides apply only to newly built layouts; authored types remain
/// unchanged. Their order is the deterministic order of generated index builds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LookupFirst {
    pub ty: syn::Type,
    pub providers: Vec<(Ident, Vec<Ident>, syn::Type)>,
    pub order: Option<Vec<Ident>>,
    pub transfer: Option<Vec<Ident>>,
    pub early: Option<Vec<Ident>>,
}

fn identifiers(input: ParseStream<'_>) -> syn::Result<Vec<Ident>> {
    let content;
    syn::bracketed!(content in input);
    Ok(content
        .parse_terminated(Ident::parse, Token![,])?
        .into_iter()
        .collect())
}

impl Parse for LookupFirst {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<kw::using>()?;
        let ty = input.parse()?;
        let mut advice = Self {
            ty,
            providers: Vec::new(),
            order: None,
            transfer: None,
            early: None,
        };
        let mut providers_seen = false;
        while !input.is_empty() {
            input.parse::<Token![,]>()?;
            if input.peek(kw::providers) {
                input.parse::<kw::providers>()?;
                if providers_seen {
                    return Err(input.error("duplicate lookup_first providers"));
                }
                providers_seen = true;
                let content;
                syn::bracketed!(content in input);
                while !content.is_empty() {
                    let name = content.parse()?;
                    let keys = identifiers(&content)?;
                    content.parse::<Token![=]>()?;
                    let ty = content.parse()?;
                    advice.providers.push((name, keys, ty));
                    if !content.is_empty() {
                        content.parse::<Token![,]>()?;
                    }
                }
                continue;
            }
            let slot = if input.peek(kw::order) {
                input.parse::<kw::order>()?;
                &mut advice.order
            } else if input.peek(kw::transfer) {
                input.parse::<kw::transfer>()?;
                &mut advice.transfer
            } else {
                input.parse::<kw::early>()?;
                &mut advice.early
            };
            if slot.is_some() {
                return Err(input.error("duplicate lookup_first decision"));
            }
            *slot = Some(identifiers(input)?);
        }
        Ok(advice)
    }
}

impl ToTokens for LookupFirst {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let ty = &self.ty;
        tokens.extend(quote!(using #ty));
        if !self.providers.is_empty() {
            let providers = self
                .providers
                .iter()
                .map(|(name, keys, ty)| quote!(#name [#(#keys),*] = #ty));
            tokens.extend(quote!(, providers [#(#providers),*]));
        }
        if let Some(order) = &self.order {
            tokens.extend(quote!(, order [#(#order),*]));
        }
        if let Some(transfer) = &self.transfer {
            tokens.extend(quote!(, transfer [#(#transfer),*]));
        }
        if let Some(early) = &self.early {
            tokens.extend(quote!(, early [#(#early),*]));
        }
    }
}

fn plain(source: &Program) -> syn::Result<bool> {
    if source.graded.is_some() {
        return Ok(false);
    }
    let mut lowered = cq_sequence_lowering::compile(&source.structures)?;
    lowered.sequence.output = source.sequence.output.clone();
    lowered.sequence.optional_filters = source.sequence.optional_filters.clone();
    Ok(lowered.sequence == source.sequence)
}

/// Freeze evidence-derived decisions into the exact same tokens authors use.
/// No catalog capability or evidence is interpreted by an emitter.
pub fn lookup_first_advice(
    source: &Program,
    catalog: &Catalog,
) -> syn::Result<Option<LookupFirst>> {
    lookup_first_advice_in(&mut Default::default(), source, catalog)
}

pub fn lookup_first_advice_in(
    checker: &mut cq_lang_sequence::contract::Checker,
    source: &Program,
    catalog: &Catalog,
) -> syn::Result<Option<LookupFirst>> {
    let Some(graph) = graph(checker, source)? else {
        return Ok(None);
    };
    if !plain(source)? {
        return Ok(None);
    }
    let capable = |provider: &&cq_lang_sequence::ProviderToken| {
        [Capability::Exact, Capability::Membership]
            .iter()
            .all(|required| provider.capabilities.contains(required))
            && !provider.levels.is_empty()
    };
    let provider = catalog.providers.iter().find(capable).ok_or_else(|| {
        syn::Error::new_spanned(
            &graph.root,
            "lookup_first requires a catalog provider with Exact and Membership at a legal read level",
        )
    })?;
    let LookupPlan {
        order,
        transfer,
        early,
        ..
    } = plan(source, &graph);
    let mut advice = LookupFirst {
        ty: provider.ty.clone(),
        providers: Vec::new(),
        order: Some(order),
        transfer: Some(transfer),
        early: Some(early),
    };
    let mut providers = Vec::new();
    let needed = std::cell::OnceCell::new();
    // Inspect the actual generated store, not all leaves of the join. A host
    // string leaf must not force its index representation onto integer forwards.
    // Levels constrain reads, not construction. The shared builder checks the
    // rewrite; decomposition checks read placement, and replay uses frozen tokens.
    let applicable = lookup_first_with(
        checker,
        source,
        &advice,
        |name, keys, target, analysis| {
            let result = symbol_name(name);
            let levels = [keys.to_vec()];
            // Retaining an existing keyed host read does not require rebuilding
            // its index. Keep its concrete catalog-admitted representation.
            let native = source
                .structures
                .structures
                .entries
                .iter()
                .find_map(|entry| {
                    if entry.name != *name {
                        return None;
                    }
                    let structures::Structure::Host { reader, keyed, .. } = &entry.structure else {
                        return None;
                    };
                    (!matches!(reader, structures::ReaderRef::Scan)
                        && keyed.level_names().concat().starts_with(keys))
                    .then_some(&keyed.ty)
                });
            let needed = needed
                .get_or_init(|| structures::required_columns(&target.physical, &analysis.physical));
            let provider = catalog.providers.iter().filter(capable).filter(|provider| {
            provider.elements.as_ref().is_none_or(|accepted| {
                analysis.layouts[&result].iter().all(|column| {
                    let actual = &analysis.physical.rows[&result].types[column];
                    accepted.iter().any(|ty| matches!(ty, syn::Type::Infer(_)) || ty == actual)
                })
            }) && crate::rewrites::catalog::accepts_read_levels(provider, target, analysis, &result, &levels, needed)
        }).min_by_key(|provider| !native.is_some_and(|ty| crate::catalog::matches_type(&provider.ty, ty))).ok_or_else(|| syn::Error::new_spanned(
            name,
            "lookup_first generated layout requires a catalog provider with Exact and Membership for its row types at a legal read level",
        ))?;
            let ty = native
                .filter(|ty| crate::catalog::matches_type(&provider.ty, ty))
                .unwrap_or(&provider.ty);
            providers.push((name.clone(), columns(keys, name.span()), ty.clone()));
            Ok(ty.clone())
        },
    )?;
    if applicable.is_none() {
        return Ok(None);
    }
    advice.providers = providers;
    Ok(Some(advice))
}

fn fresh(used: &mut BTreeSet<String>, kind: &str, span: proc_macro2::Span) -> Ident {
    let mut name = format!("__lookup_{kind}");
    while !used.insert(name.clone()) {
        name.push('_');
    }
    Ident::new(&name, span)
}

// A positive left spine preserves its input columns and witnesses every exact
// right-hand lookup it has already performed. Do not publish or probe the same
// membership again. Opaque operators and changed provider names stop this
// deliberately narrow certificate; estimates never establish a witness.
fn witnessed_lookup<'a>(
    source: &'a [physical::Definition],
    generated: &'a [physical::Definition],
    mut left: &'a Ident,
    right: &Ident,
    keys: &[String],
) -> bool {
    while let Some(definition) = generated
        .iter()
        .chain(source)
        .find(|definition| definition.name == *left)
    {
        match &definition.operator {
            Operator::Semi(_, join) | Operator::Probe(_, join) | Operator::Merge(_, join) => {
                if join.right == *right
                    && keys
                        .iter()
                        .all(|key| join.columns.iter().any(|column| column == key.as_str()))
                {
                    return true;
                }
                left = &join.left;
            }
            _ => break,
        }
    }
    false
}

fn columns(keys: &[String], span: proc_macro2::Span) -> Vec<Ident> {
    keys.iter().map(|key| Ident::new(key, span)).collect()
}

fn provider(entries: &mut Vec<structures::Entry>, name: &Ident, keys: &[String], ty: &syn::Type) {
    let keys = columns(keys, name.span());
    entries.push(syn::parse_quote!(#name = #ty [#(#keys),*];));
}

// One retained row source may serve several unrelated keys. Give the additional
// layouts explicit identity-projection stores so every consumer names its own
// provider rather than asking a translator to choose a layout. The source's
// existing row retention is shared; the aliases build indexes, not extra rows.
fn separate_provider_layouts(
    target: &mut structures::Program,
    physical: &physical::Analysis,
    used: &mut BTreeSet<String>,
) {
    let definitions = &target.physical.program.definitions;
    let positions: BTreeMap<_, _> = definitions
        .iter()
        .enumerate()
        .map(|(index, definition)| (symbol_name(&definition.name), index))
        .collect();
    let barrier = definitions.iter().rposition(|definition| {
        matches!(definition.operator, Operator::Bind(_) | Operator::Guard(_))
    });
    let candidates: BTreeSet<_> = physical::candidate_prefixes(&target.physical, physical)
        .into_iter()
        .flat_map(|candidate| candidate.sides.map(|(name, _)| name))
        .collect();
    let mut built = BTreeMap::<String, Vec<usize>>::new();
    for (index, entry) in target.structures.entries.iter().enumerate() {
        if matches!(entry.structure, structures::Structure::Built { .. }) {
            built
                .entry(symbol_name(&entry.name))
                .or_default()
                .push(index);
        }
    }
    let mut aliases = BTreeMap::new();
    let mut insertions = BTreeMap::<usize, Vec<physical::Definition>>::new();
    let mut obsolete = BTreeSet::new();
    for (name, entries) in built {
        if entries.len() < 2
            || candidates.contains(&name)
            || entries.iter().any(|&index| {
                matches!(&target.structures.entries[index].structure,
                    structures::Structure::Built { keyed, .. } if keyed.filtered.is_some())
            })
        {
            // Keep authored candidate-pair identities and projected union
            // representations: neither is an ordinary full-row provider alias.
            continue;
        }
        let layouts: Vec<_> = entries
            .iter()
            .map(|&index| {
                let structures::Structure::Built { keyed, .. } =
                    &target.structures.entries[index].structure
                else {
                    unreachable!()
                };
                (index, keyed.level_names())
            })
            .collect();
        let reads: Vec<_> = physical.keyed_reads[&name]
            .iter()
            .filter_map(|(reader, keys, _)| {
                layouts
                    .iter()
                    .filter_map(|(index, levels)| {
                        structures::prefix_depth(levels, keys)
                            .map(|depth| (*index, levels.len() != depth, levels.len()))
                    })
                    // Match Structures::built_on: full keys precede prefixes.
                    .min_by_key(|(_, prefix, levels)| (*prefix, *levels))
                    .map(|(index, _, _)| (reader, keys, index))
            })
            .collect();
        let mut pinned: BTreeSet<_> = reads
            .iter()
            .filter(|(reader, _, _)| barrier.is_some_and(|end| positions[*reader] <= end))
            .map(|(_, _, index)| *index)
            .collect();
        if pinned.is_empty() {
            pinned.extend(reads.iter().map(|(_, _, index)| *index).min());
        }
        let source = &definitions[positions[&name]].name;
        let columns = columns(&physical.headings[&name], source.span());
        let at = positions[&name].max(barrier.unwrap_or(0));
        for index in entries {
            if !reads.iter().any(|(_, _, selected)| *selected == index) {
                obsolete.insert(index);
                continue;
            }
            if pinned.contains(&index) {
                continue;
            }
            let alias = fresh(used, "provider", source.span());
            insertions.entry(at).or_default().push(syn::parse_quote!(
                #alias = let project #source keep {#(#columns),*};
            ));
            target.structures.entries[index].name = alias.clone();
            for (_, keys, selected) in &reads {
                if *selected == index {
                    aliases.insert((name.clone(), (*keys).clone()), alias.clone());
                }
            }
        }
    }
    let mut index = 0;
    target.structures.entries.retain(|_| {
        let keep = !obsolete.contains(&index);
        index += 1;
        keep
    });
    if aliases.is_empty() {
        return;
    }
    let mut definitions = Vec::new();
    for (index, mut definition) in std::mem::take(&mut target.physical.program.definitions)
        .into_iter()
        .enumerate()
    {
        let ordered = matches!(definition.operator, Operator::Merge(..));
        if let Operator::Probe(_, keyed)
        | Operator::Merge(_, keyed)
        | Operator::Semi(_, keyed)
        | Operator::Anti(_, keyed) = &mut definition.operator
        {
            let mut keys: Vec<_> = keyed.columns.iter().map(symbol_name).collect();
            keys.sort();
            if let Some(alias) = aliases.get(&(symbol_name(&keyed.right), keys.clone())) {
                keyed.right = alias.clone();
            }
            if ordered && let Some(alias) = aliases.get(&(symbol_name(&keyed.left), keys)) {
                keyed.left = alias.clone();
            }
        }
        definitions.push(definition);
        if let Some(mut providers) = insertions.remove(&index) {
            definitions.append(&mut providers);
        }
    }
    target.physical.program.definitions = definitions;
}

/// `using Provider [, order [...]] [, transfer [...]] [, early [...]]`.
/// The explicit family rejects authored schedules rather than silently erasing
/// them; automatic callers receive `None` from `lookup_first_advice` instead.
// Formal rule EXT-SeqLookup: Reexpress relational joins under checked support equality while preserving Rust syntax and input support.
pub fn lookup_first(source: &Program, arguments: TokenStream) -> syn::Result<Program> {
    lookup_first_in(&mut Default::default(), source, arguments)
}

pub fn lookup_first_in(
    checker: &mut cq_lang_sequence::contract::Checker,
    source: &Program,
    arguments: TokenStream,
) -> syn::Result<Program> {
    let advice: LookupFirst = syn::parse2(arguments)?;
    let mut providers = BTreeMap::new();
    for (name, keys, ty) in &advice.providers {
        let key = (
            symbol_name(name),
            keys.iter().map(symbol_name).collect::<Vec<_>>(),
        );
        if providers.insert(key, ty).is_some() {
            return Err(syn::Error::new_spanned(
                name,
                "duplicate lookup_first provider layout",
            ));
        }
    }
    let result = lookup_first_with(checker, source, &advice, |name, keys, _, _| {
        Ok(providers
            .remove(&(symbol_name(name), keys.to_vec()))
            .unwrap_or(&advice.ty)
            .clone())
    })?.ok_or_else(|| syn::Error::new_spanned(
        source,
        "lookup_first would require row scans or additional retention of an authored filtered index",
    ))?;
    if !providers.is_empty() {
        return Err(syn::Error::new_spanned(
            source,
            "lookup_first providers must name newly built layouts, not authored providers",
        ));
    }
    Ok(result)
}

fn lookup_first_with(
    checker: &mut cq_lang_sequence::contract::Checker,
    source: &Program,
    advice: &LookupFirst,
    mut choose_provider: impl FnMut(
        &Ident,
        &[String],
        &structures::Program,
        &structures::Analysis,
    ) -> syn::Result<syn::Type>,
) -> syn::Result<Option<Program>> {
    let Some(graph) = graph(checker, source)? else {
        return Err(syn::Error::new_spanned(
            source,
            "lookup_first has no movable join region",
        ));
    };
    if !plain(source)? {
        return Err(syn::Error::new_spanned(
            source,
            "lookup_first preserves authored schedules; apply ordering before decomposition",
        ));
    }
    let mut decisions = plan(source, &graph);
    if let Some(order) = &advice.order {
        let actual: BTreeSet<_> = order.iter().map(symbol_name).collect();
        let expected: BTreeSet<_> = graph.leaves.iter().map(symbol_name).collect();
        if actual != expected || actual.len() != order.len() {
            return Err(syn::Error::new_spanned(
                source,
                "lookup_first order must name every join leaf exactly once",
            ));
        }
        decisions.order = order.clone();
        let mut depths = vec![0usize; decisions.order.len()];
        decisions.parents = vec![None; decisions.order.len()];
        for child in 1..decisions.order.len() {
            let mut best = None;
            for parent in 0..child {
                let weight = intersection(
                    &graph.headings[&symbol_name(&decisions.order[child])],
                    &graph.headings[&symbol_name(&decisions.order[parent])],
                )
                .len();
                if weight > 0
                    && best
                        .is_none_or(|(old, score)| (weight, depths[parent]) > (score, depths[old]))
                {
                    best = Some((parent, weight));
                }
            }
            if let Some((parent, _)) = best {
                decisions.parents[child] = Some(parent);
                depths[child] = depths[parent] + 1;
            }
        }
    }
    if !graph.scan_roots.contains(&symbol_name(&decisions.order[0])) {
        return Err(syn::Error::new_spanned(
            &decisions.order[0],
            "lookup_first root needs a row scan or an exclusively consumed pure index producer",
        ));
    }
    let recover_root = graph.filtered.contains(&symbol_name(&decisions.order[0]));
    if let Some(transfer) = &advice.transfer {
        decisions.transfer = transfer.clone();
    }
    if let Some(early) = &advice.early {
        decisions.early = early.clone();
    }
    for names in [&decisions.transfer, &decisions.early] {
        let selected: BTreeSet<_> = names.iter().map(symbol_name).collect();
        if selected.len() != names.len() || names.iter().any(|name| !graph.leaves.contains(name)) {
            return Err(syn::Error::new_spanned(
                source,
                "lookup_first decisions must name distinct join leaves",
            ));
        }
    }
    let mut target = source.structures.clone();
    // Optional scan accelerators carry certificates for the original graph.
    // Validate their coverage again after changing joins and provider layouts.
    target
        .structures
        .entries
        .retain(|entry| !matches!(entry.structure, structures::Structure::JoinFilter { .. }));
    let mut used: BTreeSet<_> = target
        .physical
        .program
        .definitions
        .iter()
        .map(|d| symbol_name(&d.name))
        .chain(
            source
                .sequence
                .definitions
                .iter()
                .map(|d| symbol_name(&d.name)),
        )
        .collect();
    let mut generated: Vec<physical::Definition> = Vec::new();
    let mut optional_filters = source.sequence.optional_filters.clone();
    let mut builds = BTreeMap::<(String, Vec<String>), Ident>::new();
    let mut retained: Vec<_> = decisions.order.clone();
    let mut active: Vec<_> = decisions
        .order
        .iter()
        .map(|name| decisions.transfer.contains(name))
        .collect();
    let mut build = |name: &Ident, keys: &[String]| {
        builds
            .entry((symbol_name(name), keys.to_vec()))
            .or_insert_with(|| name.clone());
    };
    // Chaining forward phase: each child is filtered before its parent uses it.
    for child in (1..decisions.order.len()).rev() {
        let Some(parent) = decisions.parents[child] else {
            continue;
        };
        // Projected bridge providers cannot be scanned. The recovered root is
        // different: its original pure producer supplies full rows directly.
        if !active[child]
            || (graph
                .filtered
                .contains(&symbol_name(&decisions.order[parent]))
                && !(parent == 0 && recover_root))
        {
            continue;
        }
        let keys = intersection(
            &graph.headings[&symbol_name(&decisions.order[child])],
            &graph.headings[&symbol_name(&decisions.order[parent])],
        );
        let left = &retained[parent];
        let right = &retained[child];
        active[parent] = true;
        if witnessed_lookup(
            &source.structures.physical.program.definitions,
            &generated,
            left,
            right,
            &keys,
        ) {
            continue;
        }
        build(right, &keys);
        let name = fresh(&mut used, "forward", graph.root.span());
        let columns = columns(&keys, graph.root.span());
        // The root forward has one streaming consumer and is never a keyed
        // child build. Only nonroot forwards need retained rows for providers.
        let mode = if parent == 0 {
            quote!(pipe)
        } else {
            quote!(let)
        };
        generated.push(syn::parse_quote!(#name = #mode semi eager [#(#columns),*] #left, #right;));
        retained[parent] = name;
    }
    let mut current = retained[0].clone();
    let mut heading = graph.headings[&symbol_name(&decisions.order[0])].clone();
    let mut pending: Vec<usize> = (1..decisions.order.len()).collect();
    let mut probed = BTreeSet::new();
    while !pending.is_empty() {
        // Lookups needing a newly bound cycle attribute are not eligible until
        // its Expand has run. Shared key names express transitive equalities.
        let mut ready: Vec<_> = pending
            .iter()
            .copied()
            .filter(|&i| {
                let required =
                    intersection(&heading, &graph.headings[&symbol_name(&decisions.order[i])]);
                !required.is_empty()
            })
            .collect();
        ready.sort_by(|&left, &right| {
            let ratio = |i| {
                source
                    .evidence
                    .selectivity(&symbol_name(&decisions.order[i]))
                    .unwrap_or(cq_lang_sequence::Selectivity {
                        numerator: 1,
                        denominator: 1,
                    })
            };
            let left = ratio(left);
            let right = ratio(right);
            (u128::from(left.numerator) * u128::from(right.denominator))
                .cmp(&(u128::from(right.numerator) * u128::from(left.denominator)))
        });
        let early = ready
            .iter()
            .copied()
            .find(|i| decisions.early.contains(&decisions.order[*i]));
        if early.is_none() {
            for &i in &ready {
                let keys =
                    intersection(&heading, &graph.headings[&symbol_name(&decisions.order[i])]);
                if !probed.insert((i, keys.clone())) {
                    continue;
                }
                if witnessed_lookup(
                    &source.structures.physical.program.definitions,
                    &generated,
                    &current,
                    &retained[i],
                    &keys,
                ) {
                    continue;
                }
                build(&retained[i], &keys);
                let right = &retained[i];
                let name = fresh(&mut used, "lookup", graph.root.span());
                let columns = columns(&keys, graph.root.span());
                generated.push(
                    syn::parse_quote!(#name = pipe semi eager [#(#columns),*] #current, #right;),
                );
                optional_filters.push(name.clone());
                current = name;
            }
        }
        // Expand the first tree relation whose payload supplies a remaining
        // Lookup key; payload-only relations wait until the final consumer.
        let next = early
            .or_else(|| {
                ready.iter().copied().find(|&i| {
                    let added: Vec<_> = graph.headings[&symbol_name(&decisions.order[i])]
                        .iter()
                        .filter(|column| !heading.contains(column))
                        .cloned()
                        .collect();
                    pending.iter().any(|&j| {
                        j != i
                            && !intersection(
                                &added,
                                &graph.headings[&symbol_name(&decisions.order[j])],
                            )
                            .is_empty()
                    })
                })
            })
            .or_else(|| ready.first().copied())
            .unwrap_or(pending[0]);
        let right = &retained[next];
        let keys = intersection(
            &heading,
            &graph.headings[&symbol_name(&decisions.order[next])],
        );
        build(right, &keys);
        let name = if pending.len() == 1 {
            graph.root.clone()
        } else {
            fresh(&mut used, "expand", graph.root.span())
        };
        let columns = columns(&keys, graph.root.span());
        let mut definition: physical::Definition =
            syn::parse_quote!(#name = pipe probe first [#(#columns),*] #current, #right;);
        if pending.len() == 1 {
            let original = source
                .structures
                .physical
                .program
                .definitions
                .iter()
                .find(|d| d.name == graph.root)
                .expect("original root");
            definition.canon = original.canon;
            definition.mode = original.mode.clone();
        }
        generated.push(definition);
        current = name;
        heading.extend(
            graph.headings[&symbol_name(&decisions.order[next])]
                .iter()
                .cloned(),
        );
        heading.sort();
        heading.dedup();
        pending.retain(|&i| i != next);
    }
    let last = target
        .physical
        .program
        .definitions
        .iter()
        .rposition(|d| graph.joins.contains(&symbol_name(&d.name)))
        .expect("join root exists");
    // Preserve a scan-only root's streaming mode unless a keyed build or an
    // authored host adapter requires retention. In particular, replacing a
    // merge under the same consumer name displaces its left ordered reader:
    // record the row-store transition before removing that keyed adapter.
    let root_name = symbol_name(&decisions.order[0]);
    let retain_root = builds.keys().any(|(name, _)| *name == root_name)
        || source.structures.structures.entries.iter().any(|entry| {
            entry.name == decisions.order[0]
                && matches!(&entry.structure, structures::Structure::Host { reader, .. }
                    if reader.result() == Some(&graph.root))
        });
    let mut definitions = Vec::new();
    for (i, mut definition) in target.physical.program.definitions.into_iter().enumerate() {
        if !graph.joins.contains(&symbol_name(&definition.name)) {
            if definition.name == decisions.order[0] && recover_root {
                definition.mode = physical::Mode::Pipe(Default::default());
            } else if decisions.order.contains(&definition.name)
                && (definition.name != decisions.order[0] || retain_root)
            {
                definition.mode = physical::Mode::Let(Default::default());
            }
            definitions.push(definition);
        }
        if i == last {
            definitions.append(&mut generated);
        }
    }
    target.physical.program.definitions = definitions;
    // Witness elimination can leave an already ordered join unchanged. In
    // that case this pass has no relational rewrite to justify replacing its
    // representation: preserve the complete checked source, including any
    // authored provider entries, rather than performing Structures-only cleanup.
    if target.physical == source.structures.physical {
        return Ok(Some(source.clone()));
    }
    // Entries tied to deleted joins no longer describe an executable result.
    // Existing leaf read adapters remain; generated exact indexes supersede
    // keyed fallbacks without changing their opaque implementation laws.
    let physical = physical::contract::check(&target.physical)?;
    let intersection_reads: BTreeSet<_> = physical::candidate_prefixes(&target.physical, &physical)
        .into_iter()
        .flat_map(|candidate| candidate.sides)
        .flat_map(|(name, prefix)| {
            let full = physical.headings[&name].clone();
            [
                (name.clone(), structures::ReaderRef::Candidates, prefix),
                (name, structures::ReaderRef::Membership, full),
            ]
        })
        .collect();
    target.structures.entries.retain(|entry| {
        let name = symbol_name(&entry.name);
        if graph.joins.contains(&name) {
            return entry.name == graph.root
                && matches!(entry.structure, structures::Structure::Dedup { .. });
        }
        match &entry.structure {
            structures::Structure::Built { keyed, .. } => {
                // A raw membership certificate covers the provider's complete
                // read domain, not just the old full-key consumer. New prefix
                // lookups require replacement through the normal index builder.
                if keyed.membership.is_some() && keyed.filtered.is_none() {
                    let key = keyed.level_names().concat();
                    if key != physical.headings[&name]
                        || physical.keyed_reads.get(&name).into_iter().flatten().any(
                            |(_, columns, supply)| {
                                *columns != key || *supply != cq_syntax::SupplyKind::Stream
                            },
                        )
                        || physical
                            .merge_reads
                            .get(&name)
                            .is_some_and(|reads| !reads.is_empty())
                    {
                        return false;
                    }
                }
                physical.keyed_reads.get(&name).is_some_and(|reads| {
                    reads.iter().any(|(_, keys, supply)| {
                        *supply == cq_syntax::SupplyKind::Stream
                            && structures::prefix_depth(&keyed.level_names(), keys).is_some()
                    })
                })
            }
            structures::Structure::Host { reader, keyed, .. } => {
                let mut keys = keyed.level_names().concat();
                keys.sort();
                let reader_key = reader.key();
                matches!(reader, structures::ReaderRef::Scan)
                    || (physical.modes[&name] == physical::ModeKind::Pipe
                        && (physical.keyed_reads.get(&name).into_iter().flatten().any(
                            |(consumer, columns, _)| {
                                reader_key.result() == Some(consumer) && *columns == keys
                            },
                        ) || intersection_reads.contains(&(name, reader_key, keys))))
            }
            _ => true,
        }
    });
    // Retention also changes existing consumers of a shared leaf, not just the
    // newly emitted Lookup/Expand chain. Replace displaced stream lookups only;
    // constant and range reads retain their direct or scan/filter row path.
    for definition in &target.physical.program.definitions {
        let name = symbol_name(&definition.name);
        if physical.modes[&name] == physical::ModeKind::Pipe {
            continue;
        }
        for (_, keys, supply) in physical.keyed_reads.get(&name).into_iter().flatten() {
            if *supply != cq_syntax::SupplyKind::Stream {
                continue;
            }
            builds
                .entry((name.clone(), keys.clone()))
                .or_insert_with(|| definition.name.clone());
        }
    }
    let mut generated_providers = Vec::new();
    for ((name, keys), result) in builds {
        let covered = target.structures.entries.iter().any(|entry| {
            symbol_name(&entry.name) == name
                && matches!(&entry.structure,
                structures::Structure::Built { keyed, .. }
                if structures::prefix_depth(&keyed.level_names(), &keys).is_some())
        });
        if !covered {
            generated_providers.push((target.structures.entries.len(), keys.clone()));
            provider(&mut target.structures.entries, &result, &keys, &advice.ty);
        }
    }
    for definition in &target.physical.program.definitions {
        let name = symbol_name(&definition.name);
        if physical.modes[&name] == physical::ModeKind::Pipe {
            continue;
        }
        let (retained, builds) = target
            .structures
            .entries
            .iter()
            .filter(|entry| entry.name == definition.name)
            .fold((false, 0), |(retained, builds), entry| {
                match entry.structure {
                    structures::Structure::Rows { .. }
                    | structures::Structure::Selection { .. } => (true, builds),
                    structures::Structure::Built { .. } => (retained, builds + 1),
                    _ => (retained, builds),
                }
            });
        if !retained
            && (physical.scanned.contains(&name)
                || (builds > 1 && !graph.filtered.contains(&name))
                || physical.modes[&name] == physical::ModeKind::Exists)
        {
            let name = &definition.name;
            // Row retention is a collection, not a keyed provider: use the
            // same carrier as generic Structures selection.
            target
                .structures
                .entries
                .push(syn::parse_quote!(#name rows = ::std::vec::Vec<_>;));
        }
    }
    // A recovered projected root may still be used as an index by the new
    // chain. Such a layout cannot also supply full row scans or extra retained
    // stores. This is ordering inapplicability, not a relaxed Structures law:
    // automatic advice declines it and an explicit rewrite reports rejection.
    for name in &graph.filtered {
        let entries = target
            .structures
            .entries
            .iter()
            .filter(|entry| symbol_name(&entry.name) == *name);
        if !entries.clone().any(|entry| {
            matches!(&entry.structure,
            structures::Structure::Built { keyed, .. } if keyed.filtered.is_some())
        }) {
            continue;
        }
        let definition = target
            .physical
            .program
            .definitions
            .iter()
            .find(|definition| symbol_name(&definition.name) == *name)
            .expect("filtered provider definition");
        let multiple = matches!(&definition.operator, Operator::Union(union)
            if union.origin.as_ref().is_none_or(|origin| origin.over.is_none()));
        let (builds, nested) = entries.clone().fold((0, false), |(count, nested), entry| {
            match &entry.structure {
                structures::Structure::Built { keyed, .. } => {
                    (count + 1, nested || keyed.levels.len() != 1)
                }
                _ => (count, nested),
            }
        });
        if physical.modes[name] != physical::ModeKind::Let
            || !multiple
                && (physical.scanned.contains(name)
                    || entries.clone().any(|entry| {
                        matches!(
                            entry.structure,
                            structures::Structure::Rows { .. }
                                | structures::Structure::Selection { .. }
                        )
                    })
                    || builds != 1
                    || nested)
        {
            return Ok(None);
        }
    }
    // Liveness and projected layouts belong to Structures, not to Physical's
    // full relation headings. Freeze types only after every retained store and
    // its consumers are present, before provider aliases rename those entries.
    let layouts = structures::contract::check(&target)?;
    for (index, keys) in generated_providers {
        let ty = choose_provider(
            &target.structures.entries[index].name,
            &keys,
            &target,
            &layouts,
        )?;
        let entry = &mut target.structures.entries[index];
        let structures::Structure::Built { keyed, .. } = &mut entry.structure else {
            unreachable!("generated index entry")
        };
        keyed.ty = ty;
    }
    separate_provider_layouts(&mut target, &physical, &mut used);
    if source
        .structures
        .structures
        .entries
        .iter()
        .any(|entry| matches!(entry.structure, structures::Structure::JoinFilter { .. }))
    {
        let facts = structures::contract::check(&target)?;
        for entry in &source.structures.structures.entries {
            let structures::Structure::JoinFilter { filter, .. } = &entry.structure else {
                continue;
            };
            let covering = symbol_name(&filter.covering);
            let scan = symbol_name(&entry.name);
            let build = symbol_name(&filter.build);
            let key = symbol_name(&filter.key.columns[0]);
            if structures::join_filter_candidates(&target, &facts, &covering)
                .iter()
                .any(|candidate| {
                    candidate.scan == scan && candidate.build == build && candidate.key == key
                })
            {
                target.structures.entries.push(entry.clone());
            }
        }
    }
    // Nonroot forwards and keyed atoms retain rows for their provider reads.
    // The scan root and its forwards remain a single-consumer pipeline;
    // decomposition may introduce a checked publication when resources require it.
    let mut result = cq_sequence_lowering::compile(&target)?;
    result.evidence = source.evidence.clone();
    result.representation = source.representation.clone();
    result.sequence.output = source.sequence.output.clone();
    result.sequence.optional_filters = optional_filters;
    checker.check_rewrite_in(source, &result)?;
    Ok(Some(result))
}
