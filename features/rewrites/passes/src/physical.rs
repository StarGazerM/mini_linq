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
    let mut unique = BTreeMap::<String, BTreeSet<String>>::new();
    for definition in &mut target.program.definitions {
        let name = symbol_name(&definition.name);
        let heading: BTreeSet<_> = facts.headings[&name].iter().cloned().collect();
        let inherited = match &definition.operator {
            Operator::Unit(_) => Some(BTreeSet::new()),
            Operator::Rename(_) => None,
            Operator::Probe(_, k) | Operator::Merge(_, k) => unique
                .get(&symbol_name(&k.left))
                .zip(unique.get(&symbol_name(&k.right)))
                .map(|(l, r)| l.union(r).cloned().collect()),
            Operator::Semi(_, k) => unique.get(&symbol_name(&k.left)).cloned(),
            Operator::Select(s) | Operator::Point(_, s) | Operator::Range(_, s) => {
                unique.get(&symbol_name(&s.input)).cloned()
            }
            Operator::Bind(b) => unique.get(&symbol_name(&b.input)).cloned(),
            Operator::Guard(g) => unique.get(&symbol_name(&g.input)).cloned(),
            Operator::Project(p) => unique
                .get(&symbol_name(&p.input))
                .filter(|key| key.is_subset(&heading))
                .cloned(),
        };
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
        let Operator::Semi(_, key) = &mut definition.operator else {
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
                    Operator::Semi(_, other) => {
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

fn sorted(columns: &ir::CommaList<syn::Ident>) -> Vec<String> {
    let mut names: Vec<_> = columns.iter().map(symbol_name).collect();
    names.sort();
    names
}
