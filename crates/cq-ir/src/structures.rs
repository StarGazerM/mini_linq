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
//! `(key, payload)` pairs of the RA set, so `α(built) = ⟦r⟧`. Whether `Type`
//! or the host value implements the trait behind each read is checked by
//! rustc where the generated program performs it; nothing here claims a
//! capability. Contract: every Physical premise is discharged —
//!
//! - `probe[C] a, r` / `semi[C] a, r`: a built entry on `r` keyed on `C`, or
//!   `r` renames a host keyed on the columns `C` translates to;
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
//! Module ::= Physical "structures" "{" (Entry ";")* "}"
//! Entry  ::= Ident ["keyed" "[" Ident,* "]" ["ordered"]] ["canon"]
//!          | Ident "=" RustType "[" Ident,* "]"
//! ```

use std::collections::{BTreeMap, BTreeSet};

use syn::{Ident, Token, token};

use crate::physical::{self, ModeKind, Operator};
use crate::{CommaList, relational_plan as ra, symbol_name};

pub mod kw {
    syn::custom_keyword!(structures);
    syn::custom_keyword!(keyed);
    syn::custom_keyword!(ordered);
    syn::custom_keyword!(canon);
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Module {
    pub physical: physical::Module,
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

/// `Type [a, b]`: a Rust type built over the result, keyed on these columns.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Keyed {
    pub ty: syn::Type,
    #[syn(bracketed)]
    pub bracket: token::Bracket,
    #[syn(in = bracket)]
    #[parse(CommaList::parse_terminated)]
    pub columns: CommaList<Ident>,
}

fn names(columns: &CommaList<Ident>) -> Vec<String> {
    let mut names: Vec<String> = columns.iter().map(symbol_name).collect();
    names.sort();
    names
}

/// The Structures contract in diagnostic and predicate form.
pub mod contract {
    pub fn check(module: &super::Module) -> syn::Result<super::Analysis> {
        let physical = super::physical::contract::check(&module.physical)?;
        super::analyze(module, physical)
    }

    pub fn well_formed(module: &super::Module) -> bool {
        check(module).is_ok()
    }
}

/// A key a value is stored under, and what that key answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyFact {
    pub columns: Vec<String>,
    pub ordered: bool,
}

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
    /// Built structures per result: (sorted key columns, type).
    pub built: BTreeMap<String, Vec<(Vec<String>, syn::Type)>>,
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

    pub fn built_on(&self, result: &str, columns: &[String]) -> Option<&syn::Type> {
        let mut wanted = columns.to_vec();
        wanted.sort();
        self.built
            .get(result)?
            .iter()
            .find(|(c, _)| *c == wanted)
            .map(|(_, ty)| ty)
    }
}

fn error<T>(spanned: &impl quote::ToTokens, message: impl std::fmt::Display) -> syn::Result<T> {
    Err(syn::Error::new_spanned(spanned, message))
}

fn analyze(module: &Module, physical: physical::Analysis) -> syn::Result<Analysis> {
    let ra_module = &module.physical.index_requirements.relational_plan;
    let mut analysis = Analysis {
        physical,
        host_layouts: BTreeMap::new(),
        host_keys: BTreeMap::new(),
        host_canon: BTreeSet::new(),
        built: BTreeMap::new(),
        unique: BTreeMap::new(),
    };
    for input in ra_module.program.inputs() {
        analysis.host_layouts.insert(
            symbol_name(&input.name),
            input.columns.iter().map(|c| symbol_name(&c.name)).collect(),
        );
    }
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
                    return error(&entry.name, "structure must name an RA result");
                };
                for column in &keyed.columns {
                    if !heading.contains(&symbol_name(column)) {
                        return error(column, format!("`{name}` has no column `{column}`"));
                    }
                }
                if analysis.physical.modes[&name] != ModeKind::Let {
                    return error(
                        &entry.name,
                        format!("`{name}` is a pipe; a structure is built only over a `let`"),
                    );
                }
                let columns = names(&keyed.columns);
                if !seen.insert((name.clone(), Some(columns.clone()))) {
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
    let program = &module.physical.program;
    let merge_key: BTreeMap<String, Vec<String>> = program
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
    let infos = ra::check_in(ra_module.program.inputs(), &ra_module.plan)?;
    for (definition, physical_definition) in
        ra_module.plan.definitions.iter().zip(&program.definitions)
    {
        let name = symbol_name(&definition.result);
        let heading: Vec<String> = infos[&name].heading.iter().cloned().collect();
        let key = match &definition.operator {
            ra::Operator::Unit(_) => Some(Vec::new()),
            ra::Operator::Rename(r) => {
                let host = symbol_name(&r.relation);
                let full_by_structure = analysis.built_on(&name, &heading).is_some();
                (analysis.host_canon.contains(&host) || full_by_structure).then(|| heading.clone())
            }
            ra::Operator::NaturalJoin(j) => {
                let (l, r) = (symbol_name(&j.left), symbol_name(&j.right));
                match (analysis.unique.get(&l), analysis.unique.get(&r)) {
                    (Some(kl), Some(kr)) => {
                        let mut key: Vec<String> = kl.iter().chain(kr).cloned().collect();
                        key.sort();
                        key.dedup();
                        Some(key)
                    }
                    _ => None,
                }
            }
            ra::Operator::Semijoin(j) => analysis.unique.get(&symbol_name(&j.left)).cloned(),
            ra::Operator::Select(s) => analysis.unique.get(&symbol_name(&s.input)).cloned(),
            ra::Operator::Extend(e) => analysis.unique.get(&symbol_name(&e.input)).cloned(),
            ra::Operator::Project(p) => analysis
                .unique
                .get(&symbol_name(&p.input))
                .filter(|key| key.iter().all(|k| heading.contains(k)))
                .cloned(),
        };
        let key = match key {
            Some(key) => Some(key),
            None if physical_definition.canon.is_some() => Some(heading.clone()),
            None => analysis.built_on(&name, &heading).map(|_| heading.clone()),
        };
        if let Some(key) = key {
            analysis.unique.insert(name, key);
        }
    }
    let returned = symbol_name(&program.returned);
    if !analysis.unique.contains_key(&returned) {
        return error(
            &program.returned,
            format!(
                "returned `{returned}` is not canonical: flag it `canon` or make its inputs canonical"
            ),
        );
    }
    Ok(analysis)
}
