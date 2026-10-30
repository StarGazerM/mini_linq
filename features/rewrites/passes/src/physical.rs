use std::collections::{BTreeMap, BTreeSet};

use cq_lang_physical::{self as ir, Choice, ModeKind, Operator, symbol_name};
use proc_macro2::TokenStream;

use cq_structure_selection::arguments::Results;

/// Replace selected probes with merges when each side already supplies order
/// through a retained result, reusable host, or same-key merge stream. Modes
/// are never changed. Native host reads retain rustc's OrderedRead obligation.
pub fn merge_joins(source: &ir::Program, arguments: TokenStream) -> syn::Result<ir::Program> {
    let facts = ir::contract::check(source)?;
    let selected = Results::parse(arguments)?;
    let mut target = source.clone();
    let mut matched = BTreeSet::new();
    let mut ordered = BTreeMap::<String, Vec<String>>::new();
    for position in 0..target.program.definitions.len() {
        let definition = &target.program.definitions[position];
        if let Operator::Merge(_, key) = &definition.operator {
            ordered.insert(symbol_name(&definition.name), sorted(&key.columns));
            if selected.includes(&definition.name) {
                matched.insert(symbol_name(&definition.name));
            }
            continue;
        }
        if !selected.includes(&definition.name) {
            continue;
        }
        let Operator::Probe(_, key) = &definition.operator else {
            continue;
        };
        let columns = sorted(&key.columns);
        let admissible = !columns.is_empty()
            && [&key.left, &key.right].iter().all(|side| {
                let name = symbol_name(side);
                facts.modes[&name] != ModeKind::Pipe
                    || facts.hosts.contains_key(&name)
                    || ordered.get(&name) == Some(&columns)
            });
        if admissible {
            let name = symbol_name(&definition.name);
            let mut key = key.clone();
            key.choice = None;
            target.program.definitions[position].operator =
                Operator::Merge(Default::default(), key);
            ordered.insert(name.clone(), columns);
            matched.insert(name);
        }
    }
    selected.require_matches(&matched)?;
    ir::check_rewrite(source, &target)?;
    Ok(target)
}

/// Add canon only at selected projections that drop columns and do not retain
/// a known uniqueness key. Existing canonical boundaries are preserved.
pub fn canon_prefixes(source: &ir::Program, arguments: TokenStream) -> syn::Result<ir::Program> {
    let facts = ir::contract::check(source)?;
    let selected = Results::parse(arguments)?;
    let mut target = source.clone();
    let mut matched = BTreeSet::new();
    let mut unique = BTreeMap::<String, Vec<String>>::new();
    for definition in &mut target.program.definitions {
        let name = symbol_name(&definition.name);
        let heading = facts.headings[&name].clone();
        let inherited = ir::inherited_key(
            &definition.operator,
            &heading,
            &target.program.host_sets,
            |id| unique.get(&symbol_name(id)).cloned(),
        );
        if selected.includes(&definition.name) {
            if let Operator::Project(p) = &definition.operator {
                if heading.len() < facts.headings[&symbol_name(&p.input)].len() {
                    matched.insert(name.clone());
                    if inherited.is_none() {
                        definition.canon = Some(Default::default());
                    }
                }
            }
        }
        if let Some(key) = inherited.or_else(|| definition.canon.is_some().then_some(heading)) {
            unique.insert(name, key);
        }
    }
    selected.require_matches(&matched)?;
    ir::check_rewrite(source, &target)?;
    Ok(target)
}

/// Expose k-way intersections by hoisting independent relational definitions
/// before their candidate probe. Empty/all/result advice retains its existing
/// meaning; `smallest(R@a, S@b, ...)` selects one exact occurrence site.
/// Admissibility and correspondence belong to the public Physical Set law.
pub fn smallest_intersections(
    source: &ir::Program,
    arguments: TokenStream,
) -> syn::Result<ir::Program> {
    let facts = ir::contract::check(source)?;
    let selection: SmallestSelection = syn::parse2(arguments)?;
    let definitions = &source.program.definitions;
    let mut plans = Vec::new();
    let mut matched = BTreeSet::new();
    let mut site_matches = match &selection {
        SmallestSelection::Sites(sites) => vec![0; sites.len()],
        SmallestSelection::Results(_) => Vec::new(),
    };
    for (position, first) in definitions.iter().enumerate() {
        if !matches!(first.operator, Operator::Probe(..)) {
            continue;
        }
        let mut chain = vec![position];
        loop {
            let previous = *chain.last().unwrap();
            let Some(next) = (previous + 1..definitions.len()).find(|&next| {
                ir::intersection_extension(first, &definitions[previous], &definitions[next], &facts)
                    .is_some()
                    || matches!(selection, SmallestSelection::Sites(_))
                        && ir::intersection_key_projection(
                            first, &definitions[previous], &definitions[next], &facts,
                        )
            }) else {
                break;
            };
            if !definitions[previous + 1..next].iter().all(|definition| {
                chain.iter().all(|&member| independent_relational(
                    &definition.operator, &symbol_name(&definitions[member].name),
                ))
            }) {
                break;
            }
            chain.push(next);
        }
        if chain.len() < 2 {
            continue;
        }
        let selected = match &selection {
            SmallestSelection::Results(results) => results.includes(&first.name),
            SmallestSelection::Sites(sites) => {
                let origins: Option<BTreeSet<_>> = chain.iter().map(|&index| {
                    let (Operator::Probe(_, key) | Operator::Semi(_, key)) =
                        &definitions[index].operator else { unreachable!() };
                    provider_occurrence(definitions, &key.right)
                }).collect();
                let mut selected = false;
                for (index, site) in sites.iter().enumerate() {
                    if origins.as_ref() == Some(site) && site.len() == chain.len() {
                        site_matches[index] += 1;
                        selected = true;
                    }
                }
                selected
            }
        };
        if selected {
            matched.insert(symbol_name(&first.name));
            plans.push(chain);
        }
    }
    match &selection {
        SmallestSelection::Results(results) => results.require_matches(&matched)?,
        SmallestSelection::Sites(sites) => {
            for (site, count) in sites.iter().zip(site_matches) {
                if count != 1 {
                    return Err(syn::Error::new(
                        proc_macro2::Span::call_site(),
                        format!("smallest({}) must identify exactly one admissible complete intersection site; matched {count}",
                            site.iter().map(|(relation, occurrence)| format!("{relation}@{occurrence}"))
                                .collect::<Vec<_>>().join(", ")),
                    ));
                }
            }
        }
    }
    let mut target = source.clone();
    for chain in plans {
        let first_name = &definitions[chain[0]].name;
        for pair in chain.windows(2) {
            let member = &definitions[pair[1]];
            if ir::intersection_extension(
                &definitions[chain[0]], &definitions[pair[0]], member, &facts,
            ).is_some() {
                continue;
            }
            let Operator::Semi(_, key) = &member.operator else {
                unreachable!("only membership sides need a key projection");
            };
            let mut suffix = 0;
            let view = loop {
                let name = syn::Ident::new(
                    &format!("__cq_smallest_{}_{}", member.name, suffix),
                    member.name.span(),
                );
                if !target.program.definitions.iter().any(|d| d.name == name) {
                    break name;
                }
                suffix += 1;
            };
            let provider = &key.right;
            let columns = key.columns.iter();
            let projection: ir::Definition =
                syn::parse_quote!(#view = let project #provider keep {#(#columns),*};);
            let position = target.program.definitions.iter()
                .position(|d| d.name == member.name).unwrap();
            let Operator::Semi(_, key) = &mut target.program.definitions[position].operator else {
                unreachable!("source membership preserved");
            };
            key.right = view;
            target.program.definitions.insert(position, projection);
        }
        for (length, &next) in chain.iter().enumerate().skip(1) {
            let start = target.program.definitions.iter().position(|d| &d.name == first_name).unwrap();
            let end = target.program.definitions.iter()
                .position(|d| d.name == definitions[next].name).unwrap();
            // Stable hoisting preserves the source's topological order. No
            // interval contains Rust or references to an intersection result.
            target.program.definitions[start..end].rotate_left(length);
        }
        let first = target.program.definitions.iter_mut().find(|d| &d.name == first_name).unwrap();
        if let Operator::Probe(_, key) = &mut first.operator {
            key.choice = Some(Choice::Smallest(Default::default()));
        }
    }
    ir::check_rewrite(source, &target)?;
    Ok(target)
}

type OccurrenceSite = BTreeSet<(String, ir::syntax::Occurrence)>;

enum SmallestSelection {
    Results(Results),
    Sites(Vec<OccurrenceSite>),
}

impl syn::parse::Parse for SmallestSelection {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        use syn::{Ident, Token};
        let fork = input.fork();
        let is_site = fork.parse::<Ident>().is_ok() && fork.peek(syn::token::Paren);
        if !is_site {
            return input.parse().map(Self::Results);
        }
        let mut sites = Vec::new();
        while !input.is_empty() {
            let name: Ident = input.parse()?;
            if name != "smallest" {
                return Err(syn::Error::new_spanned(name, "expected smallest(R@a, S@b, ...)"));
            }
            let content;
            syn::parenthesized!(content in input);
            let mut site = BTreeSet::new();
            while !content.is_empty() {
                let relation: Ident = content.parse()?;
                content.parse::<Token![@]>()?;
                let occurrence: ir::syntax::Occurrence = content.parse()?;
                occurrence.check()?;
                if !site.insert((symbol_name(&relation), occurrence)) {
                    return Err(syn::Error::new_spanned(relation, "duplicate smallest occurrence"));
                }
                if !content.is_empty() {
                    content.parse::<Token![,]>()?;
                }
            }
            if site.len() < 2 {
                return Err(syn::Error::new_spanned(name, "smallest requires at least two distinct occurrences"));
            }
            if sites.contains(&site) {
                return Err(syn::Error::new_spanned(name, "duplicate smallest site"));
            }
            sites.push(site);
            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }
        Ok(Self::Sites(sites))
    }
}

/// Follow only unary relational provenance, never infer a relation from Rust.
fn provider_occurrence(
    definitions: &[ir::Definition],
    result: &syn::Ident,
) -> Option<(String, ir::syntax::Occurrence)> {
    let mut result = result;
    loop {
        let definition = definitions.iter().find(|definition| &definition.name == result)?;
        result = match &definition.operator {
            Operator::Rename(rename) => {
                return Some((symbol_name(&rename.relation), rename.occurrence.clone()));
            }
            Operator::Project(project) => &project.input,
            Operator::Select(select) | Operator::Point(_, select) | Operator::Range(_, select) => &select.input,
            _ => return None,
        };
    }
}

fn independent_relational(operator: &Operator, candidate: &str) -> bool {
    if matches!(operator, Operator::Bind(_) | Operator::Guard(_))
        || operator
            .input()
            .is_some_and(|input| symbol_name(input) == candidate)
    {
        return false;
    }
    match operator {
        Operator::Probe(_, key) | Operator::Merge(_, key) | Operator::Semi(_, key) => {
            symbol_name(&key.right) != candidate
        }
        _ => true,
    }
}

/// Change selected semijoin membership reads to lazy only when every read
/// sharing the host and key columns is lazy after the rewrite. A named subset
/// that would leave an eager peer is rejected; all changes the whole group.
pub fn lazy_semijoins(source: &ir::Program, arguments: TokenStream) -> syn::Result<ir::Program> {
    let facts = ir::contract::check(source)?;
    let selected = Results::parse(arguments)?;
    let mut target = source.clone();
    let mut matched = BTreeSet::new();
    for definition in &mut target.program.definitions {
        if !selected.includes(&definition.name) {
            continue;
        }
        let (Operator::Semi(_, key) | Operator::Anti(_, key)) = &mut definition.operator else {
            continue;
        };
        let right = symbol_name(&key.right);
        let columns = sorted(&key.columns);
        let admissible = facts.hosts.contains_key(&right)
            && facts.modes[&right] == ModeKind::Pipe
            && source
                .program
                .definitions
                .iter()
                .all(|reader| match &reader.operator {
                    Operator::Probe(_, other) => {
                        symbol_name(&other.right) != right || sorted(&other.columns) != columns
                    }
                    Operator::Semi(_, other) | Operator::Anti(_, other) => {
                        symbol_name(&other.right) != right
                            || sorted(&other.columns) != columns
                            || matches!(other.choice, Some(Choice::Lazy(_)))
                            || selected.includes(&reader.name)
                    }
                    _ => true,
                });
        if admissible {
            key.choice = Some(Choice::Lazy(Default::default()));
            matched.insert(symbol_name(&definition.name));
        }
    }
    selected.require_matches(&matched)?;
    ir::check_rewrite(source, &target)?;
    Ok(target)
}

/// Retain selected stream results (`pipe` → `let`): their rows are
/// materialized once, and their consumer streams from the retained rows.
///
/// This records only a Physical mode decision, a kernel or task boundary at a
/// retained result (formal rule EXT-RetainStream). Operators, keys and Rust
/// steps are unchanged; under exhaustive evaluation the retained rows and any
/// panic are those of the stream. A host rename is a reusable host read, not a
/// stream, and is rejected.
pub fn retain_streams(source: &ir::Program, arguments: TokenStream) -> syn::Result<ir::Program> {
    ir::contract::check(source)?;
    let selected = Results::parse(arguments)?;
    if matches!(selected, Results::All) {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "retain_streams requires the names of the stream results to retain",
        ));
    }
    let mut target = source.clone();
    let mut matched = BTreeSet::new();
    for definition in &mut target.program.definitions {
        if !selected.includes(&definition.name) {
            continue;
        }
        if matches!(definition.operator, Operator::Rename(_))
            || !matches!(definition.mode, ir::Mode::Pipe(_))
        {
            return Err(syn::Error::new_spanned(
                &definition.name,
                "retain_streams selects a pipe result that is not a host rename",
            ));
        }
        definition.mode = ir::Mode::Let(Default::default());
        matched.insert(symbol_name(&definition.name));
    }
    selected.require_matches(&matched)?;
    ir::check_rewrite(source, &target)?;
    Ok(target)
}

/// Hoist explicitly selected relational producers and their dependency closure
/// before the remaining definitions. The stable partition preserves topology;
/// opaque Rust producers cannot be hoisted. The public correspondence checker
/// owns branch scopes and Rust Before obligations.
pub fn hoist(source: &ir::Program, arguments: TokenStream) -> syn::Result<ir::Program> {
    ir::contract::check(source)?;
    let selected = Results::parse(arguments)?;
    if matches!(selected, Results::All) {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "hoist requires explicit result names",
        ));
    }
    let definitions = &source.program.definitions;
    let mut closure = BTreeSet::new();
    let mut matched = BTreeSet::new();
    for definition in definitions {
        if selected.includes(&definition.name) {
            let name = symbol_name(&definition.name);
            closure.insert(name.clone());
            matched.insert(name);
        }
    }
    selected.require_matches(&matched)?;
    // Source order is topological, so one reverse traversal closes all inputs.
    for definition in definitions.iter().rev() {
        if !closure.contains(&symbol_name(&definition.name)) {
            continue;
        }
        if matches!(definition.operator, Operator::Bind(_) | Operator::Guard(_)) {
            return Err(syn::Error::new_spanned(
                &definition.name,
                "hoist cannot move an opaque Rust dependency",
            ));
        }
        if let Some(input) = definition.operator.input() {
            closure.insert(symbol_name(input));
        }
        match &definition.operator {
            Operator::Probe(_, key)
            | Operator::Merge(_, key)
            | Operator::Semi(_, key)
            | Operator::Anti(_, key) => {
                closure.insert(symbol_name(&key.right));
            }
            Operator::Union(union) => closure.extend(union.operands.iter().map(symbol_name)),
            _ => {}
        }
    }
    let mut target = source.clone();
    target
        .program
        .definitions
        .sort_by_key(|definition| !closure.contains(&symbol_name(&definition.name)));
    ir::check_rewrite(source, &target)?;
    Ok(target)
}

/// Record checked full-heading host sets, then remove only canon boundaries
/// already implied by the resulting keys. Arguments name host relations.
pub fn host_sets(source: &ir::Program, arguments: TokenStream) -> syn::Result<ir::Program> {
    use syn::parse::Parser;
    let relations = syn::punctuated::Punctuated::<syn::Ident, syn::Token![,]>::parse_terminated
        .parse2(arguments)?;
    if relations.is_empty() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "host_sets requires host relation names",
        ));
    }
    let facts = ir::contract::check(source)?;
    let mut target = source.clone();
    for relation in relations {
        if !target.program.host_is_set(&relation) {
            target.program.host_sets.push(ir::HostSet {
                host_set_token: Default::default(),
                relation,
                discharged: None,
                semi_token: Default::default(),
            });
        }
    }
    ir::contract::check(&target)?;
    let mut keys = BTreeMap::<String, Vec<String>>::new();
    for definition in &mut target.program.definitions {
        let name = symbol_name(&definition.name);
        let heading = &facts.headings[&name];
        let inherited = ir::inherited_key(
            &definition.operator,
            heading,
            &target.program.host_sets,
            |id| keys.get(&symbol_name(id)).cloned(),
        );
        if inherited.is_some() {
            definition.canon = None;
        }
        if let Some(key) = inherited.or_else(|| definition.canon.is_some().then(|| heading.clone()))
        {
            keys.insert(name, key);
        }
    }
    if keys.contains_key(&symbol_name(&target.program.returned)) {
        target.program.return_canon = None;
    }
    ir::check_rewrite(source, &target)?;
    Ok(target)
}

fn sorted(columns: &ir::CommaList<syn::Ident>) -> Vec<String> {
    let mut names: Vec<_> = columns.iter().map(symbol_name).collect();
    names.sort();
    names
}
