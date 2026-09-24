//! Structures: IX with every attribute-keyed demand bound to a representation.
//!
//! This is where physical design happens, before any access rule. IX names
//! what must be answerable (`@r[C]`); this language names, for each host
//! relation, the key its value is stored under (a fact about the value) and,
//! for each demand no host answers, the Rust type built over the result and
//! the columns it is keyed on. Modes (pipe or materialized rows) are not
//! chosen here: they follow from result use counts and belong to Stream.
//!
//! Denotation: a host entry is the host value with its abstraction function;
//! a built entry is `Type` constructed through `cq_storage::Build` from
//! `(key, payload)` pairs of the RA set, so `α(built) = ⟦r⟧`. Which cursor
//! operations the program performs on an entry is decided by the plan and
//! the policy; the entry answers them by its key, and whether `Type` (or the
//! host value) implements the trait behind each operation is checked by
//! rustc where the generated program uses it. Nothing here claims a
//! capability. Contract: every IX demand `@r[C]` is answered — a built entry
//! on `r` keyed on `C`, or `r` renames a host keyed on the columns `C`
//! translates to; every input has one host entry; columns are valid heading
//! ordinals; no entry is duplicated.
//!
//! ```text
//! Module ::= IX "structures" "{" (Entry ";")* "}"
//! Entry  ::= Ident ["keyed" "[" Column,* "]" ["ordered"]]
//!          | Ident "=" RustType "[" Column,* "]"
//! ```

use std::collections::{BTreeMap, BTreeSet};

use syn::{Ident, Token, token};

use crate::capabilities::Operation;
use crate::{Column, CommaList, index_requirements as ix, relational_plan as ra, symbol_name};

pub mod kw {
    syn::custom_keyword!(structures);
    syn::custom_keyword!(keyed);
    syn::custom_keyword!(ordered);
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Module {
    pub index_requirements: ix::Module,
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

/// A host relation's value, stored under `key` if any.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Host {
    #[syn(optional, peek = kw::keyed)]
    pub key: Option<Key>,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Key {
    pub keyed_token: kw::keyed,
    #[syn(bracketed)]
    pub bracket: token::Bracket,
    #[syn(in = bracket)]
    #[parse(CommaList::parse_terminated)]
    pub columns: CommaList<Column>,
    pub ordered: Option<kw::ordered>,
}

impl Key {
    pub fn ordinals(&self) -> Vec<usize> {
        self.columns.iter().map(|c| c.index as usize).collect()
    }
}

/// A structure built over an RA result: a Rust type keyed on `columns`
/// (payload = the remaining heading columns).
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Keyed {
    pub ty: syn::Type,
    #[syn(bracketed)]
    pub bracket: token::Bracket,
    #[syn(in = bracket)]
    #[parse(CommaList::parse_terminated)]
    pub columns: CommaList<Column>,
}

impl Keyed {
    pub fn ordinals(&self) -> Vec<usize> {
        self.columns.iter().map(|c| c.index as usize).collect()
    }
}

/// The operations a structure keyed on `columns` answers, structurally: a
/// key answers exact seek and membership; an ordered key also key order.
fn answered(columns: &[usize], ordered: bool) -> Vec<Operation> {
    let mut operations = vec![
        Operation::Seek(columns.to_vec()),
        Operation::Contains(columns.to_vec()),
    ];
    if ordered {
        operations.push(Operation::Ordered(columns.to_vec()));
    }
    operations
}

/// The Structures contract in diagnostic and predicate form.
pub mod contract {
    pub fn check(module: &super::Module) -> syn::Result<super::Analysis> {
        super::ix::contract::check(&module.index_requirements)?;
        super::analyze(module)
    }

    pub fn well_formed(module: &super::Module) -> bool {
        check(module).is_ok()
    }
}

/// What the contract establishes for later stages.
#[derive(Clone, Debug, Default)]
pub struct Analysis {
    /// Sorted heading of every host relation and RA result.
    pub headings: BTreeMap<String, Vec<String>>,
    /// Operations each name answers by its keys, in its own heading
    /// ordinals: hosts always `iter`; keys answer `seek` and `contains`,
    /// ordered host keys and every built structure also `ordered`. Renames
    /// of hosts inherit the host's operations translated through the rename.
    pub provided: BTreeMap<String, Vec<Operation>>,
    /// Host relation of every rename that has no structure of its own.
    pub aliases: BTreeMap<String, String>,
}

fn error<T>(spanned: &impl quote::ToTokens, message: impl std::fmt::Display) -> syn::Result<T> {
    Err(syn::Error::new_spanned(spanned, message))
}

fn analyze(module: &Module) -> syn::Result<Analysis> {
    let ra_module = &module.index_requirements.relational_plan;
    let infos = ra::check_in(ra_module.program.inputs(), &ra_module.plan)?;
    let mut analysis = Analysis::default();
    let declared: BTreeSet<String> = ra_module
        .program
        .inputs()
        .map(|d| symbol_name(&d.name))
        .collect();
    for input in ra_module.program.inputs() {
        let mut heading: Vec<_> = input.columns.iter().map(|c| symbol_name(&c.name)).collect();
        heading.sort();
        analysis.headings.insert(symbol_name(&input.name), heading);
    }
    for definition in &ra_module.plan.definitions {
        let name = symbol_name(&definition.result);
        analysis
            .headings
            .insert(name.clone(), infos[&name].heading.iter().cloned().collect());
    }

    let mut seen = BTreeSet::new();
    for entry in &module.entries {
        let name = symbol_name(&entry.name);
        let Some(heading) = analysis.headings.get(&name) else {
            return error(
                &entry.name,
                "structure must name a host relation or an RA result",
            );
        };
        let provided = match &entry.structure {
            Structure::Host(host) => {
                if !declared.contains(&name) {
                    return error(&entry.name, "only a declared host relation is a host");
                }
                if !seen.insert((name.clone(), None)) {
                    return error(&entry.name, "host relation declared twice");
                }
                let mut hosted = vec![Operation::Iter];
                if let Some(key) = &host.key {
                    let columns = key.ordinals();
                    check_columns(&columns, heading.len(), key)?;
                    hosted.extend(answered(&columns, key.ordered.is_some()));
                }
                hosted
            }
            Structure::Built { keyed, .. } => {
                if declared.contains(&name) {
                    return error(
                        &entry.name,
                        "a host relation is represented by its host value; declare its key",
                    );
                }
                let columns = keyed.ordinals();
                check_columns(&columns, heading.len(), keyed)?;
                if !seen.insert((name.clone(), Some(columns.clone()))) {
                    return error(&entry.name, "duplicate structure on these columns");
                }
                answered(&columns, true)
            }
        };
        analysis.provided.entry(name).or_default().extend(provided);
    }
    if let Some(missing) = declared
        .iter()
        .find(|name| !analysis.provided.contains_key(*name))
    {
        return error(
            &module.structures_token,
            format!("host relation `{missing}` has no entry"),
        );
    }

    // Renames without structures of their own stand for their host, with the
    // host's operations translated into the rename's heading ordinals.
    for definition in &ra_module.plan.definitions {
        let name = symbol_name(&definition.result);
        let ra::Operator::Rename(rename) = &definition.operator else {
            continue;
        };
        if analysis.provided.contains_key(&name) {
            continue;
        }
        let host = symbol_name(&rename.relation);
        let host_heading = &analysis.headings[&host];
        let heading = &analysis.headings[&name];
        let translate = |ordinals: &[usize]| -> Vec<usize> {
            let mut translated: Vec<usize> = ordinals
                .iter()
                .map(|&column| {
                    let target = rename
                        .mapping
                        .iter()
                        .find(|m| symbol_name(&m.source) == host_heading[column])
                        .map(|m| symbol_name(&m.target))
                        .expect("complete rename");
                    heading
                        .iter()
                        .position(|h| *h == target)
                        .expect("target in heading")
                })
                .collect();
            translated.sort_unstable();
            translated
        };
        let translated: Vec<Operation> = analysis.provided[&host]
            .iter()
            .map(|op| match op {
                Operation::Iter => Operation::Iter,
                Operation::Contains(c) => Operation::Contains(translate(c)),
                Operation::Seek(c) => Operation::Seek(translate(c)),
                Operation::Ordered(c) => Operation::Ordered(translate(c)),
            })
            .collect();
        analysis.provided.insert(name.clone(), translated);
        analysis.aliases.insert(name, host);
    }

    // Every IX demand is served on the demanded relation and columns.
    let ix_module = &module.index_requirements;
    for index_use in &ix_module.uses {
        let index = ix_module
            .indexes
            .iter()
            .find(|i| symbol_name(&i.name) == symbol_name(&index_use.index))
            .expect("IX contract validated uses");
        let (relation, columns) = ix::signature(&index.requirement);
        let consumer = symbol_name(&index_use.result);
        let operator = ra_module
            .plan
            .definitions
            .iter()
            .find(|d| symbol_name(&d.result) == consumer)
            .map(|d| &d.operator)
            .expect("IX use names a definition");
        let required = match operator {
            ra::Operator::NaturalJoin(_) => Operation::Seek(columns.clone()),
            ra::Operator::Semijoin(_) => Operation::Contains(columns.clone()),
            _ => unreachable!("IX uses name binary operators"),
        };
        let served = analysis
            .provided
            .get(&relation)
            .is_some_and(|ops| ops.contains(&required));
        if !served {
            return error(
                index_use,
                format!(
                    "no structure on `{relation}` serves {required:?} demanded by `{consumer}`"
                ),
            );
        }
    }
    Ok(analysis)
}

fn check_columns(
    columns: &[usize],
    arity: usize,
    spanned: &impl quote::ToTokens,
) -> syn::Result<()> {
    if columns.windows(2).any(|w| w[0] >= w[1]) {
        return error(
            spanned,
            "columns must be strictly increasing heading ordinals",
        );
    }
    if let Some(c) = columns.iter().find(|&&c| c >= arity) {
        return error(spanned, format!("column {c} exceeds heading arity {arity}"));
    }
    Ok(())
}
