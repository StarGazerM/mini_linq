//! Checked key-disjoint decomposition; routing choices remain explicit advice.
//! A single-use named Canon feeding Group moves inside the same key-disjoint
//! parts as an inline Canon. Shared producers and explicit stores stay intact.
use cq_lang_sequence::{Definition, Guard, Len, Program, Term};
use cq_syntax::symbol_name;
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::parse::{Parse, ParseStream};
use syn::{Ident, LitInt, Token, bracketed};

/// Replayable routing advice. Empty keys select contiguous partial folds of a
/// globally unique Group: total spines stream; opaque spines publish host parts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyPartition {
    pub name: Ident,
    /// Fresh routed/host-parts publication name; unused for total streaming folds.
    pub input: Ident,
    pub keys: Vec<Ident>,
    pub parts: usize,
    /// Adaptive provider row-count cutoff. Absent means fixed routing.
    pub threshold: Option<usize>,
    /// Optional single-worker cache-locality cutoff, effective at least `threshold`.
    /// Omitted advice retains the legacy worker/count policy.
    pub cache_threshold: Option<usize>,
}

impl Parse for KeyPartition {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let name = input.parse()?;
        input.parse::<Token![,]>()?;
        let producer = input.parse()?;
        input.parse::<Token![,]>()?;
        let keys;
        bracketed!(keys in input);
        let keys = keys
            .parse_terminated(Ident::parse, Token![,])?
            .into_iter()
            .collect();
        input.parse::<Token![,]>()?;
        let parts = input.parse::<LitInt>()?.base10_parse()?;
        let mut threshold = None;
        let mut cache_threshold = None;
        while input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
            let keyword: Ident = input.parse()?;
            let slot = if keyword == "threshold" {
                &mut threshold
            } else if keyword == "cache" {
                &mut cache_threshold
            } else {
                return Err(syn::Error::new_spanned(
                    keyword,
                    "expected threshold or cache",
                ));
            };
            if slot.is_some() {
                return Err(syn::Error::new_spanned(
                    keyword,
                    "duplicate partition cutoff",
                ));
            }
            *slot = Some(input.parse::<LitInt>()?.base10_parse()?);
        }
        Ok(Self {
            name,
            input: producer,
            keys,
            parts,
            threshold,
            cache_threshold,
        })
    }
}

impl ToTokens for KeyPartition {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let Self {
            name,
            input,
            keys,
            parts,
            threshold,
            cache_threshold,
        } = self;
        let parts = LitInt::new(&parts.to_string(), name.span());
        let threshold = threshold.map(|n| {
            let n = LitInt::new(&n.to_string(), name.span());
            quote!(, threshold #n)
        });
        let cache = cache_threshold.map(|n| {
            let n = LitInt::new(&n.to_string(), name.span());
            quote!(, cache #n)
        });
        tokens.extend(quote!(#name, #input, [#(#keys),*], #parts #threshold #cache));
    }
}
fn references(term: &Term, counts: &mut std::collections::BTreeMap<String, usize>) {
    fn length(len: &Len, counts: &mut std::collections::BTreeMap<String, usize>) {
        match len {
            Len::Input(name) => *counts.entry(name.clone()).or_default() += 1,
            Len::Add(left, right) => {
                length(left, counts);
                length(right, counts);
            }
            _ => {}
        }
    }
    match term {
        Term::Ref(name) => *counts.entry(symbol_name(name)).or_default() += 1,
        Term::Map(_, inner)
        | Term::Compact(_, inner)
        | Term::Flatten(inner)
        | Term::Fold(_, inner)
        | Term::Group(_, inner)
        | Term::Canon(inner)
        | Term::Tile { inner, .. }
        | Term::PartitionK { inner, .. }
        | Term::Store { inner, .. }
        | Term::Lookup { inner, .. } => references(inner, counts),
        Term::Union(arms) => {
            for term in arms {
                references(term, counts);
            }
        }
        Term::Choose(arms) => {
            for (guard, term) in arms {
                if let Guard::Lt(len, _) | Guard::Ge(len, _) = guard {
                    length(len, counts);
                }
                references(term, counts);
            }
        }
        Term::Rows(_) => {}
    }
}

// The raw publication preserves the old flat carrier and ancestor scope.
// Ordinary row/length consumers reuse it; Lookup provider identifiers retain
// the retyped routed token. Never expose the new cover level to old row users.
fn redirect_references(term: &mut Term, name: &Ident, publication: &Ident) {
    fn length(len: &mut Len, name: &str, publication: &Ident) {
        match len {
            Len::Input(reference) if reference == name => *reference = symbol_name(publication),
            Len::Add(left, right) => {
                length(left, name, publication);
                length(right, name, publication);
            }
            _ => {}
        }
    }
    match term {
        Term::Ref(reference) if reference == name => *reference = publication.clone(),
        Term::Map(_, inner)
        | Term::Compact(_, inner)
        | Term::Flatten(inner)
        | Term::Fold(_, inner)
        | Term::Group(_, inner)
        | Term::Canon(inner)
        | Term::Tile { inner, .. }
        | Term::PartitionK { inner, .. }
        | Term::Store { inner, .. }
        | Term::Lookup { inner, .. } => {
            redirect_references(inner, name, publication);
        }
        Term::Union(arms) => {
            for arm in arms {
                redirect_references(arm, name, publication);
            }
        }
        Term::Choose(arms) => {
            for (guard, arm) in arms {
                if let Guard::Lt(len, _) | Guard::Ge(len, _) = guard {
                    length(len, &symbol_name(name), publication);
                }
                redirect_references(arm, name, publication);
            }
        }
        Term::Rows(_) | Term::Ref(_) => {}
    }
}

/// Move, rather than duplicate, a pure single-use canonicalizer. Keeping its
/// named raw producer avoids changing other pipeline names or their grades.
/// A retained or shared canonical result is an existing reuse boundary.
fn canonical_input(
    term: &Term,
    source: &Program,
    counts: &std::collections::BTreeMap<String, usize>,
    updates: &mut Vec<(usize, Term)>,
) -> Option<Term> {
    match term {
        Term::Canon(inner) => Some(*inner.clone()),
        Term::Ref(name) if counts.get(&symbol_name(name)) == Some(&1) => {
            let index = super::definition(source, name).ok()?;
            let definition = &source.sequence.definitions[index];
            let raw = canonical_input(&definition.term, source, counts, updates)?;
            if raw != definition.term {
                updates.push((index, raw));
            }
            Some(term.clone())
        }
        _ => None,
    }
}

fn tile_group_spine(
    term: &Term,
    source: &Program,
    counts: &std::collections::BTreeMap<String, usize>,
    parts: usize,
    updates: &mut Vec<(usize, Term)>,
) -> syn::Result<Term> {
    let tile = |inner| Term::Tile {
        n: parts,
        items: None,
        inner: Box::new(inner),
    };
    match term {
        Term::Rows(_) => Ok(tile(term.clone())),
        Term::Ref(name) => {
            let index = super::definition(source, name)?;
            let producer = &source.sequence.definitions[index].term;
            if matches!(producer, Term::Store { .. }) {
                return Ok(tile(term.clone()));
            }
            if counts.get(&symbol_name(name)) != Some(&1) {
                return Err(syn::Error::new_spanned(
                    name,
                    "BE-GroupPartial: streaming tiles require a single-use pipe or named store boundary",
                ));
            }
            let changed = tile_group_spine(producer, source, counts, parts, updates)?;
            updates.push((index, changed));
            Ok(term.clone())
        }
        Term::Map(_, _) | Term::Compact(_, _) | Term::Flatten(_) | Term::Lookup { .. } => {
            let mut changed = term.clone();
            let (Term::Map(_, inner)
            | Term::Compact(_, inner)
            | Term::Flatten(inner)
            | Term::Lookup { inner, .. }) = &mut changed
            else {
                unreachable!()
            };
            **inner = tile_group_spine(inner, source, counts, parts, updates)?;
            Ok(changed)
        }
        _ => Err(syn::Error::new_spanned(
            term,
            "BE-GroupPartial: streaming tiles require a map/compact/lookup spine ending at rows or a named store",
        )),
    }
}

fn streaming_partial_group(
    term: &Term,
    source: &Program,
    counts: &std::collections::BTreeMap<String, usize>,
    parts: usize,
    host_input: Option<&Ident>,
    updates: &mut Vec<(usize, Term)>,
) -> syn::Result<Term> {
    match term {
        Term::Group(group, input) => Ok(Term::Group(
            group.clone(),
            Box::new(if let Some(name) = host_input {
                Term::Ref(name.clone())
            } else {
                tile_group_spine(input, source, counts, parts, updates)?
            }),
        )),
        Term::Store { capacity, inner } => Ok(Term::Store {
            capacity: *capacity,
            inner: Box::new(streaming_partial_group(
                inner, source, counts, parts, host_input, updates,
            )?),
        }),
        _ => Err(syn::Error::new_spanned(
            term,
            "BE-GroupPartial: empty routing keys require a globally unique Group",
        )),
    }
}

/// Bound actual Sequence occurrences, not merely their Physical set support.
/// Every traversed map must also have a nonexpanding Physical driver path.
fn nonexpanding_host(
    source: &Program,
    analysis: &cq_lang_sequence::Analysis,
    term: &Term,
    before: usize,
) -> Option<String> {
    use cq_lang_sequence::structures::physical::Operator;
    fn extends<'a>(
        source: &'a Program,
        analysis: &cq_lang_sequence::Analysis,
        mut result: &'a Ident,
        input: &Ident,
    ) -> bool {
        loop {
            if result == input {
                return true;
            }
            let Some(definition) = source
                .structures
                .physical
                .program
                .definitions
                .iter()
                .find(|definition| definition.name == *result)
            else {
                return false;
            };
            result = match &definition.operator {
                Operator::Rename(_) => {
                    return analysis
                        .structures
                        .physical
                        .hosts
                        .get(&symbol_name(result))
                        .is_some_and(|host| input == host.as_str());
                }
                Operator::Select(select)
                | Operator::Point(_, select)
                | Operator::Range(_, select) => &select.input,
                Operator::Guard(guard) => &guard.input,
                Operator::Bind(bind) => &bind.input,
                Operator::Project(project) => &project.input,
                Operator::Semi(_, keyed) | Operator::Anti(_, keyed) => &keyed.left,
                Operator::Probe(..)
                | Operator::Merge(..)
                | Operator::Union(_)
                | Operator::Group(_)
                | Operator::Unit(_) => return false,
            };
        }
    }
    fn bound<'a>(
        source: &'a Program,
        analysis: &'a cq_lang_sequence::Analysis,
        term: &'a Term,
        before: usize,
    ) -> Option<(&'a str, &'a Ident)> {
        match term {
            Term::Rows(name) => {
                let result = symbol_name(name);
                // Rows may read an earlier publication rather than the host.
                // Support-equal duplicate buffers are not host cardinality bounds.
                if source.sequence.definitions[..before]
                    .iter()
                    .any(|definition| {
                        let name = symbol_name(&definition.name);
                        analysis.results.get(&name) == Some(&result)
                            && analysis.publications.get(&name).is_some_and(|publication| {
                                !matches!(
                                    publication,
                                    cq_lang_sequence::Publication::BorrowHost { .. }
                                )
                            })
                    })
                {
                    return None;
                }
                let host = analysis
                    .structures
                    .physical
                    .hosts
                    .get(&result)
                    .map(String::as_str)
                    .or_else(|| {
                        analysis
                            .structures
                            .host_layouts
                            .get_key_value(&result)
                            .map(|(host, _)| host.as_str())
                    })?;
                analysis
                    .structures
                    .host_layouts
                    .contains_key(host)
                    .then_some((host, name))
            }
            Term::Ref(name) => {
                let index = source.sequence.definitions[..before]
                    .iter()
                    .position(|definition| definition.name == *name)?;
                bound(
                    source,
                    analysis,
                    &source.sequence.definitions[index].term,
                    index,
                )
            }
            Term::Store { inner, .. }
            | Term::Tile { inner, .. }
            | Term::Flatten(inner)
            | Term::Canon(inner) => bound(source, analysis, inner, before),
            Term::Map(name, inner) | Term::Compact(name, inner) => {
                let child = if matches!(term, Term::Compact(_, _)) {
                    match inner.as_ref() {
                        Term::Lookup { inner, .. } => inner.as_ref(),
                        _ => inner.as_ref(),
                    }
                } else {
                    inner.as_ref()
                };
                let (host, input) = bound(source, analysis, child, before)?;
                extends(source, analysis, name, input).then_some((host, name))
            }
            Term::Choose(arms) => {
                let (_, first) = arms.first()?;
                let first = bound(source, analysis, first, before)?;
                arms.iter()
                    .skip(1)
                    .all(|(_, arm)| bound(source, analysis, arm, before) == Some(first))
                    .then_some(first)
            }
            Term::Union(_)
            | Term::Lookup { .. }
            | Term::Fold(_, _)
            | Term::Group(_, _)
            | Term::PartitionK { .. } => None,
        }
    }
    bound(source, analysis, term, before).map(|(host, _)| host.to_owned())
}

pub fn key_partition(source: &Program, arguments: TokenStream) -> syn::Result<Program> {
    key_partition_in(&mut Default::default(), source, &syn::parse2(arguments)?)
}

/// Introduce a named publication before routing. Grading separately proves that
/// publication is at an ancestor of the fixed-width separated pool.
pub fn key_partition_in(
    checker: &mut cq_lang_sequence::contract::Checker,
    source: &Program,
    advice: &KeyPartition,
) -> syn::Result<Program> {
    let analysis = checker.check_shared(source)?;
    let index = super::definition(source, &advice.name)?;
    if advice.parts == 0 {
        return Err(syn::Error::new_spanned(
            &advice.name,
            "BE-KeyPart: positive partition geometry required",
        ));
    }
    if advice.threshold == Some(0) {
        return Err(syn::Error::new_spanned(
            &advice.name,
            "BE-KeyPartProvider: runtime threshold must be positive",
        ));
    }
    if advice.cache_threshold == Some(0) {
        return Err(syn::Error::new_spanned(
            &advice.name,
            "BE-KeyPartProvider: runtime cache threshold must be positive",
        ));
    }
    if advice.cache_threshold.is_some() && advice.threshold.is_none() {
        return Err(syn::Error::new_spanned(
            &advice.name,
            "BE-KeyPartProvider: cache cutoff requires a provider threshold",
        ));
    }
    if advice.threshold.is_some()
        && !matches!(
            &source.sequence.definitions[index].term,
            Term::Store { inner, .. } if !matches!(inner.as_ref(), Term::Canon(_) | Term::Group(_, _))
        )
    {
        return Err(syn::Error::new_spanned(
            &advice.name,
            "BE-KeyPartProvider: runtime threshold requires a provider Store",
        ));
    }
    if !advice.keys.is_empty()
        && source
            .sequence
            .definitions
            .iter()
            .any(|d| d.name == advice.input)
    {
        return Err(syn::Error::new_spanned(
            &advice.input,
            "key partition publication name already exists",
        ));
    }
    let mut counts = std::collections::BTreeMap::new();
    for definition in &source.sequence.definitions {
        references(&definition.term, &mut counts);
    }
    if advice.keys.is_empty() {
        fn group(term: &Term) -> Option<(&Ident, &Term)> {
            match term {
                Term::Group(name, input) => Some((name, input)),
                Term::Store { inner, .. } => group(inner),
                _ => None,
            }
        }
        let (group_name, group_input) = group(&source.sequence.definitions[index].term)
            .ok_or_else(|| {
                syn::Error::new_spanned(
                    &advice.name,
                    "BE-GroupPartial: empty routing keys require a globally unique Group",
                )
            })?;
        let physical = &source.structures.physical.program.definitions;
        let group_definition = physical
            .iter()
            .find(|d| d.name == *group_name)
            .expect("checked group");
        let cq_lang_sequence::structures::physical::Operator::Group(group) =
            &group_definition.operator
        else {
            unreachable!("checked group")
        };
        // Match BE-Partial's materialized boundaries. Existing stored providers
        // and row results have already evaluated their opaque dependencies.
        let stored: std::collections::BTreeSet<_> = source
            .sequence
            .definitions
            .iter()
            .take(index)
            .filter(|d| matches!(d.term, Term::Store { .. }))
            .filter_map(|d| analysis.results.get(&symbol_name(&d.name)))
            .collect();
        let mut pending = vec![symbol_name(&group.input)];
        let mut seen = std::collections::BTreeSet::new();
        let mut host_owned = false;
        while let Some(name) = pending.pop() {
            if stored.contains(&name) || !seen.insert(name.clone()) {
                continue;
            }
            let Some(definition) = physical.iter().find(|d| symbol_name(&d.name) == name) else {
                continue;
            };
            if analysis.structures.physical.opaque(&definition.operator) {
                host_owned = true;
                break;
            }
            pending.extend(definition.operator.operands().into_iter().map(symbol_name));
        }
        if host_owned
            && source
                .sequence
                .definitions
                .iter()
                .any(|d| d.name == advice.input)
        {
            return Err(syn::Error::new_spanned(
                &advice.input,
                "key partition publication name already exists",
            ));
        }
        let mut updates = Vec::new();
        let output = streaming_partial_group(
            &source.sequence.definitions[index].term,
            source,
            &counts,
            advice.parts,
            host_owned.then_some(&advice.input),
            &mut updates,
        )?;
        let mut target = source.clone();
        for (index, term) in updates {
            target.sequence.definitions[index].term = term;
        }
        target.sequence.definitions[index].term = if matches!(output, Term::Store { .. }) {
            output
        } else {
            Term::Store {
                capacity: None,
                inner: Box::new(output),
            }
        };
        if host_owned {
            target.sequence.definitions.insert(
                index,
                Definition {
                    name: advice.input.clone(),
                    term: Term::Store {
                        capacity: None,
                        inner: Box::new(Term::Tile {
                            n: advice.parts,
                            items: None,
                            inner: Box::new(group_input.clone()),
                        }),
                    },
                },
            );
        }
        return super::checked_in(checker, source, target);
    }
    fn split(
        term: &Term,
        advice: &KeyPartition,
        source: &Program,
        host_bound: Option<&str>,
        counts: &std::collections::BTreeMap<String, usize>,
        updates: &mut Vec<(usize, Term)>,
    ) -> syn::Result<(Term, Term)> {
        let routed = || Term::PartitionK {
            keys: advice.keys.clone(),
            parts: advice.parts,
            inner: Box::new(Term::Ref(advice.input.clone())),
        };
        let singleton = || Term::PartitionK {
            keys: advice.keys.clone(),
            parts: 1,
            inner: Box::new(Term::Ref(advice.input.clone())),
        };
        match term {
            Term::Canon(inner) => Ok((
                *inner.clone(),
                Term::Flatten(Box::new(Term::Canon(Box::new(routed())))),
            )),
            Term::Group(group, inner) => {
                let global = source.structures.physical.program.definitions.iter()
                    .find(|definition| definition.name == *group)
                    .is_some_and(|definition| matches!(&definition.operator,
                        cq_lang_sequence::structures::physical::Operator::Group(group) if group.keys.is_empty()));
                let (input, routed) =
                    if let Some(input) = canonical_input(inner, source, counts, updates) {
                        (input, Term::Canon(Box::new(routed())))
                    } else {
                        (
                            *inner.clone(),
                            if global {
                                Term::Canon(Box::new(routed()))
                            } else {
                                routed()
                            },
                        )
                    };
                let grouped = Term::Group(group.clone(), Box::new(routed));
                Ok((
                    input,
                    if global {
                        grouped
                    } else {
                        Term::Flatten(Box::new(grouped))
                    },
                ))
            }
            Term::Store { capacity, inner } => {
                let (input, output) =
                    if matches!(inner.as_ref(), Term::Canon(_) | Term::Group(_, _)) {
                        split(inner, advice, source, host_bound, counts, updates)?
                    } else {
                        let output = if let Some(threshold) = advice.threshold {
                            let single_worker = if let Some(cache) = advice.cache_threshold {
                                let cache = cache.max(threshold);
                                let mut arms =
                                    Vec::with_capacity(if host_bound.is_some() { 3 } else { 2 });
                                if let Some(host) = host_bound {
                                    arms.push((
                                        Guard::Lt(Len::Input(host.into()), cache),
                                        singleton(),
                                    ));
                                }
                                arms.push((
                                    Guard::Lt(Len::Input(symbol_name(&advice.input)), cache),
                                    singleton(),
                                ));
                                arms.push((Guard::Else, routed()));
                                Term::Choose(arms)
                            } else {
                                singleton()
                            };
                            Term::Choose(vec![
                                (Guard::WorkersLt(2), single_worker),
                                (
                                    Guard::Lt(Len::Input(symbol_name(&advice.input)), threshold),
                                    singleton(),
                                ),
                                (Guard::Else, routed()),
                            ])
                        } else {
                            routed()
                        };
                        (*inner.clone(), output)
                    };
                Ok((
                    input,
                    Term::Store {
                        capacity: *capacity,
                        inner: Box::new(output),
                    },
                ))
            }
            _ => Err(syn::Error::new_spanned(
                &advice.name,
                "key_partition requires Canon with routing keys, Group, or provider Store",
            )),
        }
    }
    let host_bound = advice
        .cache_threshold
        .and_then(|_| {
            nonexpanding_host(
                source,
                &analysis,
                &source.sequence.definitions[index].term,
                index,
            )
        })
        .filter(|host| {
            advice.input != host.as_str()
                && !source
                    .sequence
                    .definitions
                    .iter()
                    .any(|definition| definition.name == host.as_str())
        });
    let mut updates = Vec::new();
    let (input, output) = split(
        &source.sequence.definitions[index].term,
        advice,
        source,
        host_bound.as_deref(),
        &counts,
        &mut updates,
    )?;
    let output = if matches!(output, Term::Store { .. }) {
        output
    } else {
        // Pool publication evaluates the collective at its task child.
        Term::Store {
            capacity: None,
            inner: Box::new(output),
        }
    };
    // A bare Store is only a provider candidate when the physical result is
    // consumed by exact lookup or membership. The checker records every read.
    let mut provider = None;
    if matches!(&source.sequence.definitions[index].term, Term::Store { inner, .. }
        if !matches!(inner.as_ref(), Term::Canon(_) | Term::Group(_, _)))
    {
        let analysis = checker.check_shared(source)?;
        let result = analysis.results.get(&symbol_name(&advice.name));
        let keys: Vec<_> = advice.keys.iter().map(symbol_name).collect();
        let reads: Vec<_> = analysis
            .obligations
            .iter()
            .filter_map(|obligation| match obligation {
                cq_lang_sequence::Obligation::Provider {
                    provider,
                    keys,
                    capability,
                    ..
                } if Some(provider) == result => Some((keys, capability)),
                _ => None,
            })
            .collect();
        if reads.is_empty()
            || reads.iter().any(|(lookup, capability)| {
                **lookup != keys
                    || !matches!(
                        capability,
                        cq_lang_sequence::Capability::Exact
                            | cq_lang_sequence::Capability::Membership
                    )
            })
        {
            return Err(syn::Error::new_spanned(
                &advice.name,
                "BE-KeyPartProvider: Store requires complete-key exact lookup or membership reads",
            ));
        }
        if result.is_some_and(|result| {
            analysis
                .structures
                .filtered
                .get(result)
                .is_some_and(|filtered| {
                    filtered
                        .constants
                        .iter()
                        .any(|binding| keys.contains(&symbol_name(&binding.column)))
                })
        }) {
            return Err(syn::Error::new_spanned(
                &advice.name,
                "BE-KeyPartProvider: routing keys must be present in the stored key",
            ));
        }
        if let Some(result) = result
            && source
                .sequence
                .definitions
                .iter()
                .take(index + 1)
                .any(|definition| {
                    analysis
                        .provider_reads
                        .get(&symbol_name(&definition.name))
                        .is_some_and(|reads| reads.contains(result))
                })
        {
            return Err(syn::Error::new_spanned(
                &advice.name,
                "BE-KeyPartProvider: publication must precede every keyed read of its physical provider",
            ));
        }
        provider = result.cloned();
    }
    let mut target = source.clone();
    for (index, term) in updates {
        target.sequence.definitions[index].term = term;
    }
    if let Some(provider) = provider {
        let mut found = false;
        for (entry_index, entry) in source.structures.structures.entries.iter().enumerate() {
            if symbol_name(&entry.name) != provider {
                continue;
            }
            let cq_lang_sequence::structures::Structure::Built { keyed, .. } = &entry.structure
            else {
                continue;
            };
            let keys: Vec<_> = advice.keys.iter().map(symbol_name).collect();
            if keyed.level_names().first() != Some(&keys) {
                return Err(syn::Error::new_spanned(
                    &advice.name,
                    "BE-KeyPartProvider: routing keys must equal the complete outer provider key",
                ));
            }
            let ty = &keyed.ty;
            if matches!(ty, syn::Type::Path(path) if path.qself.is_none()
                && path.path.segments.len() == 3
                && path.path.segments[0].ident == "cq_storage"
                && path.path.segments[1].ident == "sharded"
                && path.path.segments[2].ident == "Sharded")
            {
                return Err(syn::Error::new_spanned(
                    &advice.name,
                    "BE-KeyPartProvider: physical provider is already routed",
                ));
            }
            let entry_index = LitInt::new(&entry_index.to_string(), advice.name.span());
            target.structures = cq_rewrite_passes::retype(
                &target.structures,
                quote!(entry(#entry_index) => ::cq_storage::sharded::Sharded<#ty>),
            )?;
            found = true;
        }
        if !found {
            return Err(syn::Error::new_spanned(
                &advice.name,
                "BE-KeyPartProvider: no built provider layout",
            ));
        }
        for definition in target.sequence.definitions.iter_mut().skip(index + 1) {
            redirect_references(&mut definition.term, &advice.name, &advice.input);
        }
    }
    target.sequence.definitions[index].term = output;
    target.sequence.definitions.insert(
        index,
        Definition {
            name: advice.input.clone(),
            term: Term::Store {
                capacity: None,
                inner: Box::new(input),
            },
        },
    );
    super::checked_in(checker, source, target)
}
