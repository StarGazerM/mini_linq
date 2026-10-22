use std::collections::{BTreeMap, BTreeSet};

use cq_lang_physical::{self as ir, Choice, ModeKind, Operator, symbol_name};
use proc_macro2::TokenStream;

use crate::selection::Results;

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
            })
            && columns.iter().all(|column| {
                let ident = syn::Ident::new(column, definition.name.span());
                [&key.left, &key.right].iter().all(|side| {
                    ir::syntax::algebra::check_ord_type(
                        &facts.rows[&symbol_name(side)].types[column],
                        &ident,
                    )
                    .is_ok()
                })
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
                    let admissible = inherited.is_some()
                        || heading.iter().all(|column| {
                            ir::syntax::algebra::check_leibniz_type(
                                &facts.rows[&name].types[column],
                                &facts.rows[&name].bindings[column],
                            )
                            .is_ok()
                        });
                    if admissible {
                        matched.insert(name.clone());
                        if inherited.is_none() {
                            definition.canon = Some(Default::default());
                        }
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

/// Expose binary intersections by hoisting independent relational definitions
/// before their candidate probe, then record smallest. Rust nodes stay pinned.
/// Admissibility and correspondence belong to the public language checks.
/// In bag semantics, choosing a membership intersection also requires checked
/// host_set facts (or derived keys) on candidate sources: multiplicities cannot
/// be discarded. This language currently denotes Set; K-relation gating is separate.
pub fn smallest_intersections(
    source: &ir::Program,
    arguments: TokenStream,
) -> syn::Result<ir::Program> {
    let facts = ir::contract::check(source)?;
    let selected = Results::parse(arguments)?;
    let mut target = source.clone();
    let mut matched = BTreeSet::new();
    // Visit source names, rather than positions that change during hoisting,
    // so independent candidate pairs in a moved interval are still visited.
    for definition in &source.program.definitions {
        if !selected.includes(&definition.name)
            || !matches!(definition.operator, Operator::Probe(..))
        {
            continue;
        }
        let name = symbol_name(&definition.name);
        let definitions = &mut target.program.definitions;
        let position = definitions
            .iter()
            .position(|d| symbol_name(&d.name) == name)
            .expect("rewrite retains every result");
        let Some(offset) = definitions[position + 1..].iter().position(|check| {
            ir::binary_intersection(&definitions[position], check, &facts).is_some()
        }) else {
            continue;
        };
        let check_position = position + 1 + offset;
        if !definitions[position + 1..check_position]
            .iter()
            .all(|d| independent_relational(&d.operator, &name))
        {
            continue;
        }
        // The source is topological. Preserving the interval's order and
        // excluding references to the candidate also preserves its transitive
        // dependencies. This interval contains no Rust node to move across.
        definitions[position..check_position].rotate_left(1);
        if let Operator::Probe(_, key) = &mut definitions[check_position - 1].operator {
            key.choice = Some(Choice::Smallest(Default::default()));
            matched.insert(name);
        }
    }
    selected.require_matches(&matched)?;
    ir::check_rewrite(source, &target)?;
    Ok(target)
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

/// Retain reusable host renames that serve integral keyed stream reads.
///
/// This records only a Physical mode decision. Key columns, predicates and
/// reader order are unchanged; selection subsequently chooses the providers.
/// Const-only point/range scans and lazy membership groups remain host reads.
pub fn build_host_reads(source: &ir::Program, arguments: TokenStream) -> syn::Result<ir::Program> {
    let facts = ir::contract::check(source)?;
    let selected = Results::parse(arguments)?;
    let mut target = source.clone();
    let mut matched = BTreeSet::new();
    for definition in &mut target.program.definitions {
        if !selected.includes(&definition.name)
            || !matches!(definition.operator, Operator::Rename(_))
        {
            continue;
        }
        let name = symbol_name(&definition.name);
        let reads = facts
            .keyed_reads
            .get(&name)
            .into_iter()
            .flatten()
            .filter(|(_, _, supply)| *supply == ir::syntax::SupplyKind::Stream)
            .collect::<Vec<_>>();
        if reads.is_empty()
            || !reads.iter().all(|(_, keys, _)| {
                !keys.is_empty()
                    && keys.iter().all(|key| {
                        crate::indexes::sorted_key(&facts.rows[&name].types[key])
                    })
            })
        {
            continue;
        }
        // A lazy keyed read has a distinct demand contract; retaining its host
        // eagerly is not a mode-only refinement.
        if source.program.definitions.iter().any(|reader| {
            matches!(&reader.operator, Operator::Semi(_, key) | Operator::Anti(_, key)
                if symbol_name(&key.right) == name && matches!(key.choice, Some(Choice::Lazy(_))))
        }) {
            continue;
        }
        definition.mode = ir::Mode::Let(Default::default());
        matched.insert(name);
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
