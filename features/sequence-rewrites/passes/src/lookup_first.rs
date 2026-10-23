//! Lookup-before-Expand scheduling (Birler et al. §3.2–3.4, §4.2), with
//! LargestRoot (Zhao et al. Lemma 3.2) and ATP's deepest-parent tie break
//! (Qiao et al. §3). Cardinality advice chooses plans, never proof premises.
//! Retained join leaves replace their keyed fallbacks; live smallest
//! intersections keep both their candidate and full-key membership adapters.
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
    let facts = checker.check(source)?;
    let physical = &source.structures.physical.program;
    let definitions: BTreeMap<_, _> = physical
        .definitions
        .iter()
        .map(|d| (symbol_name(&d.name), d))
        .collect();
    let barrier = physical
        .definitions
        .iter()
        .rposition(|d| matches!(d.operator, Operator::Bind(_) | Operator::Guard(_)));
    let frozen: BTreeSet<_> = barrier
        .into_iter()
        .flat_map(|i| &physical.definitions[..=i])
        .map(|d| symbol_name(&d.name))
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
    if frozen.contains(&symbol_name(&root))
        || !matches!(
            definitions[&symbol_name(&root)].operator,
            Operator::Probe(..) | Operator::Merge(..)
        )
    {
        return Ok(None);
    }
    fn collect(
        name: &Ident,
        definitions: &BTreeMap<String, &physical::Definition>,
        frozen: &BTreeSet<String>,
        leaves: &mut Vec<Ident>,
        joins: &mut BTreeSet<String>,
    ) {
        let key = symbol_name(name);
        if !frozen.contains(&key) {
            if let Operator::Probe(_, join) | Operator::Merge(_, join) = &definitions[&key].operator
            {
                joins.insert(key);
                collect(&join.left, definitions, frozen, leaves, joins);
                collect(&join.right, definitions, frozen, leaves, joins);
                return;
            }
        }
        if !leaves.contains(name) {
            leaves.push(name.clone());
        }
    }
    let mut leaves = Vec::new();
    let mut joins = BTreeSet::new();
    collect(&root, &definitions, &frozen, &mut leaves, &mut joins);
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
    if leaves
        .iter()
        .all(|leaf| filtered.contains(&symbol_name(leaf)))
    {
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
        headings: facts.structures.physical.headings,
        hosts: facts.structures.physical.hosts,
        renames: facts.structures.physical.renames,
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
        .position(|leaf| !graph.filtered.contains(&symbol_name(leaf)))
        .expect("scan-compatible root checked");
    for i in 0..graph.leaves.len() {
        if !graph.filtered.contains(&symbol_name(&graph.leaves[i]))
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
/// `None` denotes no movable region (including a Rust or shared-result barrier).
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
    // Inspect the actual generated store, not all leaves of the join. A host
    // string leaf must not force its index representation onto integer forwards.
    // Levels constrain reads, not construction. The shared builder checks the
    // rewrite; decomposition checks read placement, and replay uses frozen tokens.
    lookup_first_with(checker, source, &advice, |name, keys, analysis| {
        let result = symbol_name(name);
        let provider = catalog.providers.iter().filter(capable).find(|provider| {
            provider.elements.as_ref().is_none_or(|accepted| {
                analysis.layouts[&result].iter().all(|column| {
                    let actual = &analysis.physical.rows[&result].types[column];
                    accepted.iter().any(|ty| matches!(ty, syn::Type::Infer(_)) || ty == actual)
                })
            })
        }).ok_or_else(|| syn::Error::new_spanned(
            name,
            "lookup_first generated layout requires a catalog provider with Exact and Membership for its row types at a legal read level",
        ))?;
        providers.push((
            name.clone(),
            columns(keys, name.span()),
            provider.ty.clone(),
        ));
        Ok(provider.ty.clone())
    })?;
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
// Formal rule EXT-SeqLookup: Reexpress natural joins and redundant semijoins under checked support equality, pinning every Rust prefix.
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
    let result = lookup_first_with(checker, source, &advice, |name, keys, _| {
        Ok(providers
            .remove(&(symbol_name(name), keys.to_vec()))
            .unwrap_or(&advice.ty)
            .clone())
    })?;
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
    mut choose_provider: impl FnMut(&Ident, &[String], &structures::Analysis) -> syn::Result<syn::Type>,
) -> syn::Result<Program> {
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
    if graph.filtered.contains(&symbol_name(&decisions.order[0])) {
        return Err(syn::Error::new_spanned(
            &decisions.order[0],
            "lookup_first root needs a row scan, not a projected keyed index",
        ));
    }
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
        // A projected index is not a row scan. Its local predicate may still
        // transfer upward, but no unrepresentable full-row bridge is invented.
        if !active[child]
            || graph
                .filtered
                .contains(&symbol_name(&decisions.order[parent]))
        {
            continue;
        }
        let keys = intersection(
            &graph.headings[&symbol_name(&decisions.order[child])],
            &graph.headings[&symbol_name(&decisions.order[parent])],
        );
        let left = &retained[parent];
        let right = &retained[child];
        build(right, &keys);
        let name = fresh(&mut used, "forward", graph.root.span());
        let columns = columns(&keys, graph.root.span());
        generated.push(syn::parse_quote!(#name = let semi eager [#(#columns),*] #left, #right;));
        retained[parent] = name;
        active[parent] = true;
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
                build(&retained[i], &keys);
                let right = &retained[i];
                let name = fresh(&mut used, "lookup", graph.root.span());
                let columns = columns(&keys, graph.root.span());
                generated.push(
                    syn::parse_quote!(#name = pipe semi eager [#(#columns),*] #current, #right;),
                );
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
    let mut definitions = Vec::new();
    for (i, mut definition) in target.physical.program.definitions.into_iter().enumerate() {
        if !graph.joins.contains(&symbol_name(&definition.name)) {
            if decisions.order.contains(&definition.name) {
                definition.mode = physical::Mode::Let(Default::default());
            }
            definitions.push(definition);
        }
        if i == last {
            definitions.append(&mut generated);
        }
    }
    target.physical.program.definitions = definitions;
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
                (name.clone(), "candidates", prefix),
                (name, "membership", full),
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
                physical.keyed_reads.get(&name).is_some_and(|reads| {
                    reads.iter().any(|(_, keys, _)| {
                        structures::prefix_depth(&keyed.level_names(), keys).is_some()
                    })
                })
            }
            structures::Structure::Host { reader, keyed, .. } => {
                let mut keys = keyed.level_names().concat();
                keys.sort();
                let reader_name = symbol_name(reader);
                reader == "scan"
                    || (physical.modes[&name] == physical::ModeKind::Pipe
                        && (physical.keyed_reads.get(&name).into_iter().flatten().any(
                            |(consumer, columns, _)| *consumer == reader_name && *columns == keys,
                        ) || intersection_reads.contains(&(name, reader_name.as_str(), keys))))
            }
            _ => true,
        }
    });
    // Retention also changes existing consumers of a shared leaf, not just the
    // newly emitted Lookup/Expand chain. Replace every displaced host lookup.
    for definition in &target.physical.program.definitions {
        let name = symbol_name(&definition.name);
        if physical.modes[&name] == physical::ModeKind::Pipe {
            continue;
        }
        for (_, keys, _) in physical.keyed_reads.get(&name).into_iter().flatten() {
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
    // Liveness and projected layouts belong to Structures, not to Physical's
    // full relation headings. Freeze types only after every retained store and
    // its consumers are present, before provider aliases rename those entries.
    let layouts = structures::contract::check(&target)?;
    for (index, keys) in generated_providers {
        let entry = &mut target.structures.entries[index];
        let ty = choose_provider(&entry.name, &keys, &layouts)?;
        let structures::Structure::Built { keyed, .. } = &mut entry.structure else {
            unreachable!("generated index entry")
        };
        keyed.ty = ty;
    }
    separate_provider_layouts(&mut target, &physical, &mut used);
    // Every retained atom can be read by both a forward filter and an Expand;
    // generated forward stores already have let mode. Reused host atoms retain
    // their scan adapters while the new provider supplies all keyed reads.
    let mut result = cq_sequence_lowering::compile(&target)?;
    result.evidence = source.evidence.clone();
    result.sequence.output = source.sequence.output.clone();
    checker.check_rewrite_in(source, &result)?;
    Ok(result)
}
