//! Replayable preparation of raw/filter alternatives from Lookup & Expand.
use std::collections::{BTreeMap, BTreeSet};

use cq_lang_sequence::{Program, Term, structures};
use cq_syntax::symbol_name;
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use structures::physical::{ModeKind, Operator};
use syn::parse::{Parse, ParseStream};
use syn::{Ident, LitBool, LitInt, Token};

use super::{adaptive_choose_in, fresh, has_filter, replace_producer, select_filter};
use crate::rewrites::{checked_in, definition};

mod kw {
    syn::custom_keyword!(select);
}

/// Frozen performance decisions, independent of evidence when replayed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdaptiveCandidate {
    pub filter: Ident,
    pub published: Ident,
    pub threshold: usize,
    /// `None` reads the published count; a fact selects either arm statically.
    pub select: Option<bool>,
}

impl Parse for AdaptiveCandidate {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let filter = input.parse()?;
        input.parse::<Token![,]>()?;
        let published = input.parse()?;
        input.parse::<Token![,]>()?;
        let threshold = input.parse::<LitInt>()?.base10_parse()?;
        let select = if input.is_empty() {
            None
        } else {
            input.parse::<Token![,]>()?;
            input.parse::<kw::select>()?;
            Some(input.parse::<LitBool>()?.value)
        };
        Ok(Self {
            filter,
            published,
            threshold,
            select,
        })
    }
}

impl ToTokens for AdaptiveCandidate {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let Self {
            filter,
            published,
            threshold,
            select,
        } = self;
        let threshold = LitInt::new(&threshold.to_string(), filter.span());
        let select = select.map(|value| {
            let value = LitBool::new(value, filter.span());
            quote!(, select #value)
        });
        tokens.extend(quote!(#filter, #published, #threshold #select));
    }
}

/// Authored spelling of the same replayable adaptive preparation and choice.
pub fn adaptive(source: &Program, arguments: TokenStream) -> syn::Result<Program> {
    apply_adaptive(source, &syn::parse2(arguments)?)
}
/// Enumerate optional provider/key classes in deterministic Physical order.
/// Apply the returned decisions in order before decomposition. Each application
/// preserves previous choices and covers every first exact-join consumer.
pub fn adaptive_candidates(source: &Program) -> syn::Result<Vec<AdaptiveCandidate>> {
    adaptive_candidates_in(&mut Default::default(), source)
}

pub fn adaptive_candidates_in(
    checker: &mut cq_lang_sequence::contract::Checker,
    source: &Program,
) -> syn::Result<Vec<AdaptiveCandidate>> {
    let analysis = checker.check_shared(source)?;
    if !plain(source)? {
        return Ok(Vec::new());
    }
    let mut candidates = Vec::new();
    let mut seen = BTreeSet::new();
    for filter in &source.structures.physical.program.definitions {
        let Operator::Semi(_, keyed) = &filter.operator else {
            continue;
        };
        if !symbol_name(&filter.name).starts_with("__lookup_lookup")
            || ModeKind::from(&filter.mode) != ModeKind::Pipe
            || !has_filter(source, &symbol_name(&keyed.right))
        {
            continue;
        }
        let mut keys: Vec<_> = keyed.columns.iter().map(symbol_name).collect();
        keys.sort();
        if !seen.insert((symbol_name(&keyed.right), keys)) {
            continue;
        }
        let Some(published) = source.sequence.definitions.iter().find(|definition| {
            matches!(definition.term, Term::Store { .. })
                && analysis.results.get(&symbol_name(&definition.name))
                    == Some(&symbol_name(&keyed.right))
        }) else {
            continue;
        };
        let threshold = base_count(source, &keyed.right)
            .map_or(100_000, |count| ((count as u128 * 7) / 20 + 1) as usize);
        let candidate = AdaptiveCandidate {
            filter: filter.name.clone(),
            published: published.name.clone(),
            threshold,
            select: select_filter(&source.evidence, &symbol_name(&keyed.right)),
        };
        // Candidate construction is itself the certificate: unsupported barriers
        // or a filter not absorbed by an exact join produce no candidate.
        if apply_adaptive_in(checker, source, &candidate).is_ok() {
            candidates.push(candidate);
        }
    }
    Ok(candidates)
}

/// Replay one explicit adaptive decision, checking graph preparation and final
/// selection separately. No cardinality hint is consulted by this entry point.
pub fn apply_adaptive(source: &Program, candidate: &AdaptiveCandidate) -> syn::Result<Program> {
    apply_adaptive_in(&mut Default::default(), source, candidate)
}

pub fn apply_adaptive_in(
    checker: &mut cq_lang_sequence::contract::Checker,
    source: &Program,
    candidate: &AdaptiveCandidate,
) -> syn::Result<Program> {
    checker.check_shared(source)?;
    let endpoints = covering_joins(source, candidate)?;
    let mut target = source.clone();
    for endpoint in endpoints {
        let (prepared, name, filtered, raw) = prepare(checker, &target, candidate, &endpoint)?;
        target = if let Some(select) = candidate.select {
            let index = definition(&prepared, &name)?;
            // Validate even the discarded arm: advice is never a proof premise.
            let yes = replace_producer(checker, &prepared, index, filtered)?;
            let no = replace_producer(checker, &prepared, index, raw)?;
            if select { yes } else { no }
        } else {
            adaptive_choose_in(
                checker,
                &prepared,
                &name,
                &candidate.published,
                candidate.threshold,
                filtered,
                raw,
            )?
        };
    }
    checked_in(checker, source, target)
}

fn plain(source: &Program) -> syn::Result<bool> {
    if source.graded.is_some() {
        return Ok(false);
    }
    let mut lowered = cq_sequence_lowering::compile(&source.structures)?;
    lowered.sequence.output = source.sequence.output.clone();
    Ok(lowered.sequence == source.sequence)
}

fn filter_group(
    source: &Program,
    candidate: &AdaptiveCandidate,
) -> syn::Result<BTreeMap<String, Ident>> {
    let definitions = &source.structures.physical.program.definitions;
    let selected = definitions
        .iter()
        .find(|definition| definition.name == candidate.filter)
        .ok_or_else(|| syn::Error::new_spanned(&candidate.filter, "unknown optional lookup"))?;
    let Operator::Semi(_, selected) = &selected.operator else {
        return Err(syn::Error::new_spanned(
            &candidate.filter,
            "adaptive advice requires a positive semi lookup",
        ));
    };
    let mut group = BTreeMap::new();
    for definition in definitions {
        let Operator::Semi(_, keyed) = &definition.operator else {
            continue;
        };
        if ModeKind::from(&definition.mode) == ModeKind::Pipe
            && keyed.right == selected.right
            && keyed.columns == selected.columns
        {
            group.insert(symbol_name(&definition.name), keyed.left.clone());
        }
    }
    Ok(group)
}

fn covering_joins(source: &Program, candidate: &AdaptiveCandidate) -> syn::Result<Vec<Ident>> {
    let mut active: BTreeSet<_> = filter_group(source, candidate)?.into_keys().collect();
    let definitions = &source.structures.physical.program.definitions;
    let filter = definitions
        .iter()
        .find(|definition| definition.name == candidate.filter)
        .expect("filter_group checked the selected definition");
    let Operator::Semi(_, selected) = &filter.operator else {
        unreachable!()
    };
    let mut endpoints = Vec::new();
    for definition in definitions {
        if active.contains(&symbol_name(&definition.name)) {
            continue;
        }
        if !definition
            .operator
            .operands()
            .iter()
            .any(|name| active.contains(&symbol_name(name)))
        {
            continue;
        }
        if let Operator::Probe(_, join) | Operator::Merge(_, join) = &definition.operator
            && join.right == selected.right
            && join.columns == selected.columns
        {
            endpoints.push(definition.name.clone());
        } else {
            active.insert(symbol_name(&definition.name));
        }
    }
    if endpoints.is_empty() {
        return Err(syn::Error::new_spanned(
            &candidate.filter,
            "optional lookup lacks a covering mandatory join",
        ));
    }
    Ok(endpoints)
}

fn physical_ancestors(source: &Program, result: &Ident) -> BTreeSet<String> {
    let mut names = BTreeSet::from([symbol_name(result)]);
    for definition in source.structures.physical.program.definitions.iter().rev() {
        if names.contains(&symbol_name(&definition.name)) {
            names.extend(definition.operator.operands().into_iter().map(symbol_name));
        }
    }
    names
}

fn sequence_ancestors(source: &Program, result: &Ident) -> BTreeSet<String> {
    fn references(term: &Term, names: &mut BTreeSet<String>) {
        match term {
            Term::Ref(name) => {
                names.insert(symbol_name(name));
            }
            Term::Map(_, inner)
            | Term::Compact(_, inner)
            | Term::Flatten(inner)
            | Term::Lookup { inner, .. }
            | Term::Fold(_, inner)
            | Term::Group(_, inner)
            | Term::Canon(inner)
            | Term::Tile { inner, .. }
            | Term::Store { inner, .. } => references(inner, names),
            Term::Union(arms) => {
                for arm in arms {
                    references(arm, names);
                }
            }
            Term::Choose(arms) => {
                for (_, arm) in arms {
                    references(arm, names);
                }
            }
            Term::Rows(_) => {}
        }
    }
    let mut names = BTreeSet::from([symbol_name(result)]);
    for definition in source.sequence.definitions.iter().rev() {
        if names.contains(&symbol_name(&definition.name)) {
            references(&definition.term, &mut names);
        }
    }
    names
}

fn prepare(
    checker: &mut cq_lang_sequence::contract::Checker,
    source: &Program,
    candidate: &AdaptiveCandidate,
    requested: &Ident,
) -> syn::Result<(Program, Ident, Term, Term)> {
    let analysis = checker.check_shared(source)?;
    if source.graded.is_some() {
        return Err(syn::Error::new_spanned(
            &candidate.filter,
            "adaptive preparation precedes grading",
        ));
    }
    let physical = &source.structures.physical.program;
    let filter = physical
        .definitions
        .iter()
        .find(|definition| definition.name == candidate.filter)
        .ok_or_else(|| syn::Error::new_spanned(&candidate.filter, "unknown optional lookup"))?;
    let Operator::Semi(_, keyed) = &filter.operator else {
        return Err(syn::Error::new_spanned(
            &candidate.filter,
            "adaptive preparation requires a positive semi lookup",
        ));
    };
    if ModeKind::from(&filter.mode) != ModeKind::Pipe
        || analysis.stores.contains_key(&symbol_name(&filter.name))
    {
        return Err(syn::Error::new_spanned(
            &candidate.filter,
            "adaptive preparation cannot bypass an authored publication",
        ));
    }
    if analysis.results.get(&symbol_name(&candidate.published)) != Some(&symbol_name(&keyed.right))
        || !analysis
            .stores
            .contains_key(&symbol_name(&candidate.published))
    {
        return Err(syn::Error::new_spanned(
            &candidate.published,
            "adaptive count must publish this lookup's provider",
        ));
    }
    let mut occupied: BTreeSet<_> = physical
        .definitions
        .iter()
        .map(|definition| symbol_name(&definition.name))
        .chain(
            source
                .sequence
                .definitions
                .iter()
                .map(|definition| symbol_name(&definition.name)),
        )
        .collect();
    let mut replacements = filter_group(source, candidate)?;
    let filters: BTreeSet<_> = replacements.keys().cloned().collect();
    let ancestors = physical_ancestors(source, requested);
    for name in &filters {
        if ancestors.contains(name) && analysis.stores.contains_key(name) {
            return Err(syn::Error::new_spanned(
                requested,
                "adaptive alternatives preserve authored filter publications",
            ));
        }
    }
    // The unfiltered arm must own its pure streaming prefix. Reusing a pipe
    // consumed by the filtered arm would violate Physical's single-use law.
    // Publications and host scans can be shared; never duplicate opaque Rust
    // or a provider build.
    let mut prefix = BTreeSet::new();
    let mut pending: Vec<_> = replacements
        .iter()
        .filter(|(name, _)| ancestors.contains(*name))
        .map(|(_, input)| input.clone())
        .collect();
    while let Some(name) = pending.pop() {
        let definition = physical
            .definitions
            .iter()
            .find(|definition| definition.name == name)
            .expect("checked Physical operand");
        if ModeKind::from(&definition.mode) != ModeKind::Pipe
            || matches!(definition.operator, Operator::Rename(_))
        {
            continue;
        }
        if analysis.stores.contains_key(&symbol_name(&name)) {
            return Err(syn::Error::new_spanned(
                &name,
                "adaptive alternatives preserve existing publications",
            ));
        }
        if !prefix.insert(symbol_name(&name)) {
            continue;
        }
        match &definition.operator {
            Operator::Probe(_, join) | Operator::Merge(_, join) | Operator::Semi(_, join) => {
                pending.push(join.left.clone());
            }
            Operator::Union(union) => pending.extend(union.operands.iter().cloned()),
            Operator::Project(project) => pending.push(project.input.clone()),
            _ => {
                return Err(syn::Error::new_spanned(
                    &name,
                    "adaptive alternatives cannot duplicate an opaque prefix",
                ));
            }
        }
    }
    let mut copies = Vec::new();
    let mut copied_names = Vec::new();
    let mut endpoint = None;
    // Close the choice at the first exact join that absorbs this semijoin,
    // rather than rebuilding the final suffix. Other choices remain intact.
    for (index, definition) in physical.definitions.iter().enumerate() {
        if filters.contains(&symbol_name(&definition.name))
            || !ancestors.contains(&symbol_name(&definition.name))
        {
            continue;
        }
        if !prefix.contains(&symbol_name(&definition.name))
            && !definition
                .operator
                .operands()
                .iter()
                .any(|name| replacements.contains_key(&symbol_name(name)))
        {
            continue;
        }
        if ModeKind::from(&definition.mode) != ModeKind::Pipe {
            return Err(syn::Error::new_spanned(
                &definition.name,
                "adaptive alternatives cannot clone a retained result",
            ));
        }
        let mandatory = match &definition.operator {
            Operator::Probe(_, join) | Operator::Merge(_, join) | Operator::Semi(_, join) => {
                if replacements.contains_key(&symbol_name(&join.right)) {
                    return Err(syn::Error::new_spanned(
                        &definition.name,
                        "adaptive alternatives cannot change a provider build",
                    ));
                }
                matches!(
                    definition.operator,
                    Operator::Probe(_, _) | Operator::Merge(_, _)
                ) && join.right == keyed.right
                    && join.columns == keyed.columns
            }
            Operator::Union(_) | Operator::Project(_) => false,
            _ => {
                return Err(syn::Error::new_spanned(
                    &definition.name,
                    "adaptive alternatives cannot cross an opaque boundary",
                ));
            }
        };
        let mut copy = definition.clone();
        rewrite_operator(&mut copy.operator, &replacements);
        copy.name = fresh(&definition.name, "unfiltered", &mut occupied);
        replacements.insert(symbol_name(&definition.name), copy.name.clone());
        copied_names.push((definition.name.clone(), copy.name.clone()));
        copies.push(copy);
        if mandatory && definition.name == *requested {
            endpoint = Some(index);
            break;
        }
    }
    let endpoint = endpoint.ok_or_else(|| {
        syn::Error::new_spanned(
            &candidate.filter,
            "optional lookup lacks a covering mandatory join",
        )
    })?;
    let filtered = physical.definitions[endpoint].name.clone();
    let raw = replacements[&symbol_name(&filtered)].clone();
    let joined = fresh(&filtered, "alternatives", &mut occupied);
    let redirect = BTreeMap::from([(symbol_name(&filtered), joined.clone())]);
    let mut prepared = source.clone();
    // A union otherwise restores the complete relational heading, including
    // dead opaque columns that neither branch carries. Project both checked
    // alternatives to the original live layout before joining their support.
    let columns = &analysis.structures.layouts[&symbol_name(&filtered)];
    let project =
        columns.len() != analysis.structures.physical.headings[&symbol_name(&filtered)].len();
    let (yes, no) = if project {
        let yes = fresh(&filtered, "live", &mut occupied);
        let no = fresh(&raw, "live", &mut occupied);
        let columns: Vec<_> = columns
            .iter()
            .map(|column| Ident::new(column, filtered.span()))
            .collect();
        copies.push(syn::parse_quote!(#yes = pipe project #filtered keep {#(#columns),*};));
        copies.push(syn::parse_quote!(#no = pipe project #raw keep {#(#columns),*};));
        (yes, no)
    } else {
        (filtered.clone(), raw.clone())
    };
    copies.push(syn::parse_quote!(#joined = pipe union #yes, #no;));
    let inserted = copies.len();
    prepared
        .structures
        .physical
        .program
        .definitions
        .splice(endpoint + 1..endpoint + 1, copies);
    for definition in
        &mut prepared.structures.physical.program.definitions[endpoint + 1 + inserted..]
    {
        rewrite_operator(&mut definition.operator, &redirect);
    }
    if prepared.structures.physical.program.returned == filtered {
        prepared.structures.physical.program.returned = joined.clone();
    }
    for (old, new) in copied_names {
        for entry in source
            .structures
            .structures
            .entries
            .iter()
            .filter(|entry| entry.name == old)
        {
            let mut copy = entry.clone();
            copy.name = new.clone();
            prepared.structures.structures.entries.push(copy);
        }
    }

    let sequence_endpoint = definition(source, &filtered)?;
    let mut sequence_names = filter_group(source, candidate)?;
    let sequence_ancestors = sequence_ancestors(source, &filtered);
    let mut terms = Vec::new();
    for original in &source.sequence.definitions[..=sequence_endpoint] {
        if filters.contains(&symbol_name(&original.name))
            || !sequence_ancestors.contains(&symbol_name(&original.name))
        {
            continue;
        }
        let mut term = original.term.clone();
        let changed = rewrite_term(&mut term, &sequence_names, &replacements);
        if !changed && !replacements.contains_key(&symbol_name(&original.name)) {
            continue;
        }
        if analysis.stores.contains_key(&symbol_name(&original.name)) {
            return Err(syn::Error::new_spanned(
                &original.name,
                "adaptive alternatives preserve existing publications",
            ));
        }
        let name = replacements
            .get(&symbol_name(&original.name))
            .cloned()
            .unwrap_or_else(|| fresh(&original.name, "unfiltered", &mut occupied));
        sequence_names.insert(symbol_name(&original.name), name.clone());
        terms.push(cq_lang_sequence::Definition { name, term });
    }
    if sequence_names.get(&symbol_name(&filtered)) != Some(&raw) {
        return Err(syn::Error::new_spanned(
            &filtered,
            "adaptive join lacks an explicit Sequence producer",
        ));
    }
    if project {
        terms.push(cq_lang_sequence::Definition {
            name: yes.clone(),
            term: Term::Map(yes.clone(), Box::new(Term::Ref(filtered.clone()))),
        });
        terms.push(cq_lang_sequence::Definition {
            name: no.clone(),
            term: Term::Map(no.clone(), Box::new(Term::Ref(raw.clone()))),
        });
    }
    terms.push(cq_lang_sequence::Definition {
        name: joined.clone(),
        term: Term::Union(vec![Term::Ref(yes.clone()), Term::Ref(no.clone())]),
    });
    let inserted = terms.len();
    prepared
        .sequence
        .definitions
        .splice(sequence_endpoint + 1..sequence_endpoint + 1, terms);
    let unchanged = BTreeMap::new();
    for definition in &mut prepared.sequence.definitions[sequence_endpoint + 1 + inserted..] {
        rewrite_term(&mut definition.term, &redirect, &unchanged);
    }
    let prepared = checked_in(checker, source, prepared)?;
    Ok((prepared, joined, Term::Ref(yes), Term::Ref(no)))
}

fn rename(name: &mut Ident, replacements: &BTreeMap<String, Ident>) -> bool {
    let mut changed = false;
    while let Some(replacement) = replacements.get(&symbol_name(name)) {
        *name = replacement.clone();
        changed = true;
    }
    changed
}

fn rewrite_operator(operator: &mut Operator, replacements: &BTreeMap<String, Ident>) {
    match operator {
        Operator::Probe(_, keyed)
        | Operator::Merge(_, keyed)
        | Operator::Semi(_, keyed)
        | Operator::Anti(_, keyed) => {
            rename(&mut keyed.left, replacements);
            rename(&mut keyed.right, replacements);
        }
        Operator::Union(union) => {
            for operand in &mut union.operands {
                rename(operand, replacements);
            }
        }
        Operator::Select(select) => {
            rename(&mut select.input, replacements);
        }
        Operator::Project(project) => {
            rename(&mut project.input, replacements);
        }
        Operator::Point(_, select) | Operator::Range(_, select) => {
            rename(&mut select.input, replacements);
        }
        Operator::Bind(bind) => {
            rename(&mut bind.input, replacements);
        }
        Operator::Guard(guard) => {
            rename(&mut guard.input, replacements);
        }
        Operator::Group(group) => {
            rename(&mut group.input, replacements);
        }
        _ => {}
    }
}

fn rewrite_term(
    term: &mut Term,
    references: &BTreeMap<String, Ident>,
    results: &BTreeMap<String, Ident>,
) -> bool {
    match term {
        Term::Ref(name) => rename(name, references),
        Term::Rows(name) => rename(name, results),
        Term::Map(name, inner) | Term::Compact(name, inner) | Term::Group(name, inner) => {
            let named = rename(name, results);
            rewrite_term(inner, references, results) | named
        }
        Term::Lookup {
            provider, inner, ..
        } => {
            let named = rename(provider, results);
            rewrite_term(inner, references, results) | named
        }
        Term::Flatten(inner)
        | Term::Canon(inner)
        | Term::Fold(_, inner)
        | Term::Tile { inner, .. }
        | Term::Store { inner, .. } => rewrite_term(inner, references, results),
        Term::Union(arms) => {
            let mut changed = false;
            for arm in arms {
                changed |= rewrite_term(arm, references, results);
            }
            changed
        }
        Term::Choose(arms) => {
            let mut changed = false;
            for (_, arm) in arms {
                changed |= rewrite_term(arm, references, results);
            }
            changed
        }
    }
}

fn base_count(source: &Program, provider: &Ident) -> Option<usize> {
    let definitions = &source.structures.physical.program.definitions;
    let mut current = provider;
    while let Some(definition) = definitions
        .iter()
        .find(|definition| definition.name == *current)
    {
        if let Operator::Rename(rename) = &definition.operator {
            return source
                .evidence
                .count(&symbol_name(current))
                .or_else(|| source.evidence.count(&symbol_name(&rename.relation)));
        }
        current = definition.operator.input()?;
        if let Some(count) = source.evidence.count(&symbol_name(current)) {
            return Some(count);
        }
    }
    None
}
