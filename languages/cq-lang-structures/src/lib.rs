//! Structures records only built let structures, their provider types and trie levels.
//! Host renames need no entry: type dispatch selects their direct or lazy built reads.
//! Contracts discharge plan premises, never host capability claims.
//! The denotation is `bottom + Set`; opaque Rust binds and guards stay in their
//! written chain. Const supplies use one point read and never justify a build.
//! ```text
//! Entry ::= Ident "=" RustType ("[" Ident,* "]")+ ";"
//! ```
//!
//! Relational equality is Leibniz identity, decided by admissible column `==`.
//! Functional extensionality of shared-reference Rust inputs permits canonical
//! projection whenever free variables remain; see the premises in
//! `cq-syntax`.

pub use cq_lang_physical as physical;

use std::collections::{BTreeMap, BTreeSet};

use syn::{Ident, Token, token};

pub use cq_syntax::{CommaList, symbol_name};

pub mod kw {
    syn::custom_keyword!(structures);
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

use physical::{ModeKind, Operator, syntax::SupplyKind};

/// A built structure: the sorted columns of each level, and its type.
pub type Built = (Vec<Vec<String>>, syn::Type);

/// What the contract establishes for later stages.
#[derive(Clone, Debug)]
pub struct Analysis {
    pub physical: physical::Analysis,
    /// Declared column order of every host relation.
    pub host_layouts: BTreeMap<String, Vec<String>>,
    /// Built structures per result: (sorted columns of each level, type).
    pub built: BTreeMap<String, Vec<Built>>,
    /// Results that are canonical (a set) by inference or flag, with the key
    /// that makes them so when inferred.
    pub unique: BTreeMap<String, Vec<String>>,
}

impl Analysis {
    /// Reusable host reads are selected by runtime type dispatch.
    pub fn host_read(&self, result: &str) -> bool {
        self.physical.hosts.contains_key(result) && self.physical.modes[result] == ModeKind::Pipe
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

/// Formal rule WF-Structures: Check entries, read coverage and inferred canonical return.
/// Formal rule STR-Key: Answer probe and semi reads by host dispatch or a built level prefix.
/// Formal rule STR-Merge: Answer each merge side by ordered storage or a same-key merge pipe.
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
        built: BTreeMap::new(),
        unique: BTreeMap::new(),
    };
    let mut seen = BTreeSet::new();
    for entry in &module.entries {
        let name = symbol_name(&entry.name);
        let Structure::Built { keyed, .. } = &entry.structure;
        {
            if analysis.host_layouts.contains_key(&name) {
                return error(
                    &entry.name,
                    "a host relation is represented by its host value; build over a let result",
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
                cq_syntax::algebra::check_leibniz_type(
                    &analysis.physical.rows[&name].types[&symbol_name(column)],
                    column,
                )?;
            }
            if analysis.physical.modes[&name] != ModeKind::Let {
                return error(
                    &entry.name,
                    format!("`{name}` is a pipe; a structure is built only over a `let`"),
                );
            }
            let columns = keyed.level_names();
            let serves_stream = analysis
                .physical
                .keyed_reads
                .get(&name)
                .into_iter()
                .flatten()
                .any(|(_, read_columns, supply)| {
                    *supply == SupplyKind::Stream && prefix_depth(&columns, read_columns).is_some()
                });
            if !serves_stream {
                return error(
                    &entry.name,
                    "a built structure must serve a stream demand; const-only reads use direct access or scan and filter",
                );
            }
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
    // Formal rule STR-Range: Const/range supplies always have direct or scan/filter reads without builds.
    // Only stream supplies require reusable storage. Const supplies always
    // have the direct-host or scan-and-filter implementation available.
    let answered = |result: &str, columns: &[String]| -> bool {
        analysis.host_read(result) || analysis.built_on(result, columns).is_some()
    };
    for (result, reads) in &analysis.physical.keyed_reads {
        for (reader, columns, supply) in reads {
            if *supply != SupplyKind::Stream
                || analysis
                    .physical
                    .merge_reads
                    .get(result)
                    .is_some_and(|merge| {
                        merge.iter().any(|(merge_reader, _)| merge_reader == reader)
                    })
            {
                continue;
            }
            if !answered(result, columns) {
                return error(
                    &module.structures_token,
                    format!(
                        "`{reader}` reads `{result}` on {columns:?}; no reusable host read or built structure answers it"
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
            let by_structure =
                analysis.host_read(side) || analysis.built_on(side, columns).is_some();
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
            Operator::Rename(_) => None,
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
            Operator::Select(s) | Operator::Point(_, s) | Operator::Range(_, s) => {
                analysis.unique.get(&symbol_name(&s.input)).cloned()
            }
            Operator::Bind(b) => analysis.unique.get(&symbol_name(&b.input)).cloned(),
            Operator::Guard(g) => analysis.unique.get(&symbol_name(&g.input)).cloned(),
            Operator::Project(p) => analysis
                .unique
                .get(&symbol_name(&p.input))
                .filter(|key| key.iter().all(|k| heading.contains(k)))
                .cloned(),
        };
        let key = match key {
            Some(key) => Some(key),
            None if analysis.physical.canon.contains(&name) => Some(heading.clone()),
            None => None,
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
