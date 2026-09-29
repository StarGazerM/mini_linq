//! Structures: Physical with every keyed read bound to a value that answers it.
//!
//! Physical fixes where a key, an order, or uniqueness is needed; this
//! language names, for each host relation, the facts about its value (the
//! key it is stored under, whether that key is ordered, whether the value is
//! a canonical set) and, for each read no host answers, the Rust type built
//! over the result and the columns it is keyed on. Columns are attribute
//! names of the result's heading.
//!
//! Denotation: a host entry is the host value with its abstraction function;
//! a built entry is `Type` constructed through `cq_storage::Build` from
//! `(key, payload)` pairs of the result set, so `α(built) = ⟦r⟧`. Whether `Type`
//! or the host value implements the trait behind each read is checked by
//! rustc where the generated program performs it; nothing here claims a
//! capability. Contract: every Physical premise is discharged —
//!
//! - `probe[C] a, r` / `semi[C] a, r`: a built entry on `r` whose key — or,
//!   for a leveled entry, the union of some prefix of its levels — is `C`,
//!   or `r` renames a host keyed on the columns `C` translates to;
//! - `range[c]` over `r`: the same, ordered;
//! - `merge[C] l, r`: each side ordered on `C` — a host key `ordered`, a
//!   built entry (rustc checks `OrderedRead`), or a pipe whose own merge is
//!   on `C` (the stream is sorted by construction);
//! - the returned result is canonical: flagged `canon`, or unique by key
//!   inference from `canon` hosts through joins, selects, extends and
//!   key-keeping projections;
//!
//! every input has one host entry; no entry is duplicated.
//!
//! ```text
//! Program ::= physical::Program "structures" "{" (Entry ";")* "}"
//! Entry  ::= Ident ["keyed" "[" Ident,* "]" ["ordered"]] ["canon"]
//!          | Ident "=" RustType ("[" Ident,* "]")+
//! ```
//!
//! A built entry with several bracketed levels is a curried index: level
//! `i` is keyed on its columns given every earlier level. By the exponential
//! law `(A × B → C) ≅ (A → B → C)` one such value answers a read on every
//! prefix `K₁ ∪ … ∪ Kᵢ`, which is how a trie serves `[a]`, `[a, b]` and
//! `[a, b, c]` at once. Level order is representation; IX demands stay sets.

pub use cq_lang_physical as physical;

use std::collections::{BTreeMap, BTreeSet};

use syn::{Ident, Token, token};

pub use cq_syntax::{CommaList, symbol_name};

pub mod kw {
    syn::custom_keyword!(structures);
    syn::custom_keyword!(keyed);
    syn::custom_keyword!(ordered);
    syn::custom_keyword!(canon);
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Structures {
    pub structures_token: kw::structures,
    #[syn(braced)]
    pub brace: token::Brace,
    #[syn(in = brace)]
    #[parse(parse_entries)]
    #[to_tokens(|tokens, entries| quote::TokenStreamExt::append_all(tokens, entries))]
    pub entries: Vec<Entry>,
}

fn parse_entries(input: syn::parse::ParseStream<'_>) -> syn::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    while !input.is_empty() {
        entries.push(input.parse()?);
    }
    Ok(entries)
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Entry {
    pub name: Ident,
    pub structure: Structure,
    pub semi_token: Token![;],
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum Structure {
    #[parse(peek = Token![=])]
    Built {
        eq_token: Token![=],
        keyed: Box<Keyed>,
    },
    Host(Host),
}

/// Facts about a host value: the key it is stored under, and whether it is
/// a canonical set of its rows.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Host {
    #[parse(|input| if input.peek(kw::keyed) { input.parse().map(Some) } else { Ok(None) })]
    pub key: Option<Key>,
    pub canon: Option<kw::canon>,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Key {
    pub keyed_token: kw::keyed,
    #[syn(bracketed)]
    pub bracket: token::Bracket,
    #[syn(in = bracket)]
    #[parse(CommaList::parse_terminated)]
    pub columns: CommaList<Ident>,
    pub ordered: Option<kw::ordered>,
}

/// `Type [a, b]` or `Type [a][b]`: a Rust type built over the result, keyed
/// on these columns, one bracket per level.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Keyed {
    pub ty: syn::Type,
    #[parse(parse_levels)]
    #[to_tokens(|tokens, levels| quote::TokenStreamExt::append_all(tokens, levels))]
    pub levels: Vec<Level>,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Level {
    #[syn(bracketed)]
    pub bracket: token::Bracket,
    #[syn(in = bracket)]
    #[parse(CommaList::parse_terminated)]
    pub columns: CommaList<Ident>,
}

fn parse_levels(input: syn::parse::ParseStream<'_>) -> syn::Result<Vec<Level>> {
    let mut levels = vec![input.parse()?];
    while input.peek(token::Bracket) {
        levels.push(input.parse()?);
    }
    Ok(levels)
}

impl Keyed {
    /// Every column across all levels, level by level.
    pub fn columns(&self) -> impl Iterator<Item = &Ident> {
        self.levels.iter().flat_map(|l| l.columns.iter())
    }

    /// Sorted column names of each level.
    pub fn level_names(&self) -> Vec<Vec<String>> {
        self.levels.iter().map(|l| names(&l.columns)).collect()
    }
}

fn names(columns: &CommaList<Ident>) -> Vec<String> {
    let mut names: Vec<String> = columns.iter().map(symbol_name).collect();
    names.sort();
    names
}

/// Physical execution paired with all required storage choices.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Program {
    pub physical: physical::Program,
    pub structures: Structures,
}

pub mod contract {
    pub fn check(program: &super::Program) -> syn::Result<super::Analysis> {
        super::analyze(program)
    }
}

use physical::{ModeKind, Operator};

/// A key a value is stored under, and what that key answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyFact {
    pub columns: Vec<String>,
    pub ordered: bool,
}

/// A built structure: the sorted columns of each level, and its type.
pub type Built = (Vec<Vec<String>>, syn::Type);

/// What the contract establishes for later stages.
#[derive(Clone, Debug)]
pub struct Analysis {
    pub physical: physical::Analysis,
    /// Declared column order of every host relation.
    pub host_layouts: BTreeMap<String, Vec<String>>,
    /// Key facts of every host, in the host's own attribute names.
    pub host_keys: BTreeMap<String, Option<KeyFact>>,
    /// Hosts declared canonical.
    pub host_canon: BTreeSet<String>,
    /// Built structures per result: (sorted columns of each level, type).
    pub built: BTreeMap<String, Vec<Built>>,
    /// Results that are canonical (a set) by inference or flag, with the key
    /// that makes them so when inferred.
    pub unique: BTreeMap<String, Vec<String>>,
}

impl Analysis {
    /// The key fact answering reads of `result` on `columns`, if a host
    /// key does through the rename.
    pub fn host_key_for(&self, result: &str, columns: &[String]) -> Option<&KeyFact> {
        let host = self.physical.hosts.get(result)?;
        let fact = self.host_keys.get(host)?.as_ref()?;
        let rename = &self.physical.renames[result];
        let mut translated: Vec<String> = columns.iter().map(|c| rename[c].clone()).collect();
        translated.sort();
        (translated == fact.columns).then_some(fact)
    }

    /// The structure answering a read of `result` on `columns`, and how
    /// many of its levels the read applies.
    pub fn built_on(&self, result: &str, columns: &[String]) -> Option<(&syn::Type, usize)> {
        let mut wanted = columns.to_vec();
        wanted.sort();
        self.built
            .get(result)?
            .iter()
            .find_map(|(levels, ty)| prefix_depth(levels, &wanted).map(|depth| (ty, depth)))
    }
}

/// The number of leading levels whose union is exactly `wanted`, if any.
pub fn prefix_depth(levels: &[Vec<String>], wanted: &[String]) -> Option<usize> {
    let mut union: Vec<String> = Vec::new();
    for (depth, level) in levels.iter().enumerate() {
        union.extend(level.iter().cloned());
        union.sort();
        if union == wanted {
            return Some(depth + 1);
        }
    }
    None
}

fn error<T>(spanned: &impl quote::ToTokens, message: impl std::fmt::Display) -> syn::Result<T> {
    Err(syn::Error::new_spanned(
        spanned,
        format!("{}: {message}", spanned.to_token_stream()),
    ))
}

fn analyze(program: &Program) -> syn::Result<Analysis> {
    let module = &program.structures;
    let mut analysis = Analysis {
        physical: physical::contract::check(&program.physical)?,
        host_layouts: program
            .physical
            .inputs()
            .map(|r| {
                (
                    symbol_name(&r.name),
                    r.columns.iter().map(|c| symbol_name(&c.name)).collect(),
                )
            })
            .collect(),
        host_keys: BTreeMap::new(),
        host_canon: BTreeSet::new(),
        built: BTreeMap::new(),
        unique: BTreeMap::new(),
    };
    let mut seen = BTreeSet::new();
    for entry in &module.entries {
        let name = symbol_name(&entry.name);
        match &entry.structure {
            Structure::Host(host) => {
                let Some(layout) = analysis.host_layouts.get(&name) else {
                    return error(&entry.name, "only a declared host relation is a host");
                };
                if !seen.insert((name.clone(), None)) {
                    return error(&entry.name, "host relation declared twice");
                }
                let fact = match &host.key {
                    Some(key) => {
                        for column in &key.columns {
                            if !layout.contains(&symbol_name(column)) {
                                return error(column, format!("`{name}` has no column `{column}`"));
                            }
                        }
                        Some(KeyFact {
                            columns: names(&key.columns),
                            ordered: key.ordered.is_some(),
                        })
                    }
                    None => None,
                };
                analysis.host_keys.insert(name.clone(), fact);
                if host.canon.is_some() {
                    analysis.host_canon.insert(name);
                }
            }
            Structure::Built { keyed, .. } => {
                if analysis.host_layouts.contains_key(&name) {
                    return error(
                        &entry.name,
                        "a host relation is represented by its host value; declare its key",
                    );
                }
                let Some(heading) = analysis.physical.headings.get(&name) else {
                    return error(&entry.name, "structure must name a Physical result");
                };
                let mut all = BTreeSet::new();
                for column in keyed.columns() {
                    if !heading.contains(&symbol_name(column)) {
                        return error(column, format!("`{name}` has no column `{column}`"));
                    }
                    if !all.insert(symbol_name(column)) {
                        return error(column, format!("`{column}` appears in two levels"));
                    }
                }
                if analysis.physical.modes[&name] != ModeKind::Let {
                    return error(
                        &entry.name,
                        format!("`{name}` is a pipe; a structure is built only over a `let`"),
                    );
                }
                let columns = keyed.level_names();
                if !seen.insert((name.clone(), Some(columns.concat()))) {
                    return error(&entry.name, "duplicate structure on these columns");
                }
                analysis
                    .built
                    .entry(name)
                    .or_default()
                    .push((columns, keyed.ty.clone()));
            }
        }
    }
    if let Some(missing) = analysis
        .host_layouts
        .keys()
        .find(|name| !analysis.host_keys.contains_key(*name))
    {
        return error(
            &module.structures_token,
            format!("host relation `{missing}` has no entry"),
        );
    }

    // Keyed reads: probe, semi, range.
    let answered = |result: &str, columns: &[String]| -> bool {
        analysis.host_key_for(result, columns).is_some()
            || analysis.built_on(result, columns).is_some()
    };
    for (result, reads) in &analysis.physical.key_reads {
        for (reader, columns) in reads {
            if !answered(result, columns) {
                return error(
                    &module.structures_token,
                    format!(
                        "`{reader}` reads `{result}` on {columns:?}; no host key or structure answers it"
                    ),
                );
            }
        }
    }
    for (result, reads) in &analysis.physical.range_reads {
        for (reader, column) in reads {
            let columns = vec![column.clone()];
            let ordered = analysis
                .host_key_for(result, &columns)
                .is_some_and(|k| k.ordered)
                || analysis.built_on(result, &columns).is_some();
            if !ordered {
                return error(
                    &module.structures_token,
                    format!(
                        "`{reader}` ranges over `{result}` on `{column}`; no ordered key answers it"
                    ),
                );
            }
        }
    }
    // Merge sides: ordered on the key by Γ, or a pipe sorted on it by its own merge.
    let merge_key: BTreeMap<String, Vec<String>> = program
        .physical
        .program
        .definitions
        .iter()
        .filter_map(|d| match &d.operator {
            Operator::Merge(_, k) => Some((symbol_name(&d.name), names(&k.columns))),
            _ => None,
        })
        .collect();
    for (side, reads) in &analysis.physical.merge_reads {
        for (reader, columns) in reads {
            let by_structure = analysis
                .host_key_for(side, columns)
                .is_some_and(|k| k.ordered)
                || analysis.built_on(side, columns).is_some();
            let by_stream = analysis.physical.modes[side] == ModeKind::Pipe
                && merge_key.get(side) == Some(columns);
            if !by_structure && !by_stream {
                return error(
                    &module.structures_token,
                    format!(
                        "`{reader}` merges `{side}` on {columns:?}; it is neither ordered on that key nor a pipe sorted on it"
                    ),
                );
            }
        }
    }

    // Uniqueness by key inference, then the canonical return.
    for definition in &program.physical.program.definitions {
        let name = symbol_name(&definition.name);
        let op = &definition.operator;
        let Some(heading) = analysis.physical.headings.get(&name).cloned() else {
            return error(
                &module.structures_token,
                format!("result `{name}` has no heading"),
            );
        };
        let key = match op {
            Operator::Unit(_) => Some(Vec::new()),
            Operator::Rename(r) => {
                let full_by_structure = analysis.built_on(&name, &heading).is_some();
                (analysis.host_canon.contains(&symbol_name(&r.relation)) || full_by_structure)
                    .then(|| heading.clone())
            }
            Operator::Probe(_, k) | Operator::Merge(_, k) => {
                match (
                    analysis.unique.get(&symbol_name(&k.left)),
                    analysis.unique.get(&symbol_name(&k.right)),
                ) {
                    (Some(kl), Some(kr)) => {
                        let mut key: Vec<String> = kl.iter().chain(kr).cloned().collect();
                        key.sort();
                        key.dedup();
                        Some(key)
                    }
                    _ => None,
                }
            }
            Operator::Semi(_, k) => analysis.unique.get(&symbol_name(&k.left)).cloned(),
            Operator::Select(s) => analysis.unique.get(&symbol_name(&s.input)).cloned(),
            Operator::Range(_, r) => analysis.unique.get(&symbol_name(&r.input)).cloned(),
            Operator::Extend(e) => analysis.unique.get(&symbol_name(&e.input)).cloned(),
            Operator::Project(p) => analysis
                .unique
                .get(&symbol_name(&p.input))
                .filter(|key| key.iter().all(|k| heading.contains(k)))
                .cloned(),
        };
        let key = match key {
            Some(key) => Some(key),
            None if analysis.physical.canon.contains(&name) => Some(heading.clone()),
            None => analysis.built_on(&name, &heading).map(|_| heading.clone()),
        };
        if let Some(key) = key {
            analysis.unique.insert(name, key);
        }
    }
    let returned = symbol_name(&program.physical.program.returned);
    if !analysis.unique.contains_key(&returned) {
        return error(
            &module.structures_token,
            format!(
                "returned `{returned}` is not canonical: flag it `canon` or make its inputs canonical"
            ),
        );
    }
    Ok(analysis)
}

/// Shared syntax used by this language.
pub use cq_syntax as syntax;
