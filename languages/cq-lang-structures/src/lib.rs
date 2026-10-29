//! Structures records providers, retained rows or positions, and read adapters.
//! Ordinary host reads keep their recorded native/lazy fallback path. Explicit
//! positional decisions name a base, total column lineage, marker and summary;
//! generated Rust checks the asserted host and provider capabilities.
//! Provider and codec names carry no Rust type meaning here: trait capabilities,
//! including equality and ordered access, remain generated Rust obligations.
//! Structural checks still enforce key coverage, lineage and read ownership.
//! Host key levels retain their recorded order; their flattened, sorted key set
//! must exactly match the named read. A scan has one empty level. Printers may
//! impose stricter representation-specific level requirements.
//! Smallest intersections require two candidate levels on every retained side,
//! alongside any structures needed by other reads of the same result.
//! Rust expressions are opaque and total by premise, placed by free variables.
//! Const supplies use one point read and never justify a build.
//! ```text
//! Entry ::= Ident ("=" RustType ("[" Ident,* "]")+
//!                | "host" ReaderRef "=" RustType ("[" Ident,* "]")+
//!                | ("rows" | "canon" | "head" | "group") "=" RustType
//!                | "selection" "=" RustType Origin "summary" Summary
//!                | "keyset" "=" RustType "[" Ident,* "]" Ownership Origin
//!                | "read" "=" "positional" Origin "column" Ident
//!                    "marker" RustType "predicate" ReadPredicate
//!                | "repr" "=" ("tuples" | "positions" "{"
//!                    (Ident "lineage" "{" (Ident "->" Ident),* "}"),*
//!                    "}" "owned" "[" Ident,* "]")
//!                    ["witness" "{" Ident "[" Ident,* "]" "at" Ident "as" RustType "}"]
//!                | "share" "=" "lookup" Ident "[" Ident,* "]" "first" Ident "force" Ident
//!                | "stage" "=" "batch" Integer ["expand"]) ";"
//! Origin ::= "base" Ident "lineage" "{" (Ident "->" Ident),* "}"
//! ReaderRef ::= "scan" | "candidates" | "membership" | "result" "(" Ident ")"
//! ```
//!
//! Relational equality is Leibniz identity, decided by admissible column `==`.
//! Functional extensionality of shared-reference Rust inputs permits canonical
//! projection whenever free variables remain; see the premises in
//! `cq-syntax`.

//! Structures records union dedup providers and anti membership representations.
//! A derived union used only by a shared-key index streams its chained scan directly into the filtered build path, without a retained row buffer.
//!
//! Core blocks denote sets: `{ B1 } | { B2 }` unions projected answers;
//! `{ B1 } else { B2 }` selects the first nonempty arm. F is the free heading
//! and I the common new interface; other binders remain branch-local.
//! Join distributes over union; fallback is `B1 union (B2 anti_F B1)`.
//! Only explicit relational predicates introduce comparisons and constraints.
//!
pub use cq_lang_physical as physical;

use quote::ToTokens;
use std::collections::{BTreeMap, BTreeSet};

use syn::{Ident, Token, token};

pub use cq_syntax::{CommaList, symbol_name};

pub mod kw {
    syn::custom_keyword!(structures);
    syn::custom_keyword!(host);
    syn::custom_keyword!(scan);
    syn::custom_keyword!(candidates);
    syn::custom_keyword!(result);
    syn::custom_keyword!(group);
    syn::custom_keyword!(canon);
    syn::custom_keyword!(head);
    syn::custom_keyword!(rows);
    syn::custom_keyword!(payload);
    syn::custom_keyword!(constants);
    syn::custom_keyword!(selection);
    syn::custom_keyword!(keyset);
    syn::custom_keyword!(read);
    syn::custom_keyword!(positional);
    syn::custom_keyword!(base);
    syn::custom_keyword!(lineage);
    syn::custom_keyword!(summary);
    syn::custom_keyword!(none);
    syn::custom_keyword!(per_block);
    syn::custom_keyword!(column);
    syn::custom_keyword!(marker);
    syn::custom_keyword!(predicate);
    syn::custom_keyword!(physical);
    syn::custom_keyword!(membership);
    syn::custom_keyword!(exact);
    syn::custom_keyword!(row);
    syn::custom_keyword!(owned);
    syn::custom_keyword!(borrowed);
    syn::custom_keyword!(join_filter);
    syn::custom_keyword!(build);
    syn::custom_keyword!(covering);
    syn::custom_keyword!(repr);
    syn::custom_keyword!(positions);
    syn::custom_keyword!(tuples);
    syn::custom_keyword!(stage);
    syn::custom_keyword!(batch);
    syn::custom_keyword!(share);
    syn::custom_keyword!(lookup);
    syn::custom_keyword!(first);
    syn::custom_keyword!(force);
    syn::custom_keyword!(witness);
    syn::custom_keyword!(at);
    syn::custom_keyword!(expand);
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

/// Host roles occupy a different namespace from Physical result identifiers.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReaderRef<I = Ident> {
    Scan,
    Candidates,
    Membership,
    Result(I),
}

impl<I> ReaderRef<I> {
    pub fn result(&self) -> Option<&I> {
        match self {
            Self::Result(name) => Some(name),
            _ => None,
        }
    }
}

impl ReaderRef {
    pub fn key(&self) -> ReaderRef<String> {
        match self {
            Self::Scan => ReaderRef::Scan,
            Self::Candidates => ReaderRef::Candidates,
            Self::Membership => ReaderRef::Membership,
            Self::Result(name) => ReaderRef::Result(symbol_name(name)),
        }
    }
}

/// Formal rule STR-Reader: Parse disjoint host roles and explicitly named results.
impl syn::parse::Parse for ReaderRef {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        if input.peek(kw::scan) {
            input.parse::<kw::scan>()?;
            Ok(Self::Scan)
        } else if input.peek(kw::candidates) {
            input.parse::<kw::candidates>()?;
            Ok(Self::Candidates)
        } else if input.peek(kw::membership) {
            input.parse::<kw::membership>()?;
            Ok(Self::Membership)
        } else {
            input.parse::<kw::result>()?;
            let body;
            syn::parenthesized!(body in input);
            let name = body.parse()?;
            if !body.is_empty() {
                return Err(body.error("expected one Physical result identifier"));
            }
            Ok(Self::Result(name))
        }
    }
}

impl ToTokens for ReaderRef {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        tokens.extend(match self {
            Self::Scan => quote::quote!(scan),
            Self::Candidates => quote::quote!(candidates),
            Self::Membership => quote::quote!(membership),
            Self::Result(name) => quote::quote!(result(#name)),
        });
    }
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum Structure {
    #[parse(peek = kw::share)]
    Share {
        share_token: kw::share,
        eq_token: Token![=],
        lookup_token: kw::lookup,
        build: Ident,
        keys: Level,
        first_token: kw::first,
        first: Ident,
        force_token: kw::force,
        force: Ident,
    },
    #[parse(peek = kw::repr)]
    Repr {
        repr_token: kw::repr,
        eq_token: Token![=],
        repr: Repr,
        #[parse(repr::parse_witness)]
        witness: Option<Witness>,
    },
    #[parse(peek = kw::stage)]
    Stage {
        stage_token: kw::stage,
        eq_token: Token![=],
        batch_token: kw::batch,
        size: syn::LitInt,
        expand: Option<kw::expand>,
    },
    #[parse(peek = kw::join_filter)]
    JoinFilter {
        join_filter_token: kw::join_filter,
        eq_token: Token![=],
        filter: JoinFilter,
    },
    #[parse(peek = kw::selection)]
    Selection {
        selection_token: kw::selection,
        eq_token: Token![=],
        ty: syn::Type,
        origin: Origin,
        summary_token: kw::summary,
        summary: Summary,
    },
    #[parse(peek = kw::keyset)]
    KeySet {
        keyset_token: kw::keyset,
        eq_token: Token![=],
        ty: syn::Type,
        key: Level,
        ownership: KeyOwnership,
        origin: Origin,
    },
    #[parse(peek = kw::read)]
    Read {
        read_token: kw::read,
        eq_token: Token![=],
        positional_token: kw::positional,
        origin: Origin,
        column_token: kw::column,
        column: Ident,
        marker_token: kw::marker,
        marker: syn::Type,
        predicate_token: kw::predicate,
        predicate: ReadPredicate,
    },
    #[parse(peek = kw::host)]
    Host {
        host_token: kw::host,
        reader: ReaderRef,
        eq_token: Token![=],
        keyed: Box<Keyed>,
    },
    #[parse(peek = kw::canon)]
    Dedup {
        canon_token: kw::canon,
        eq_token: Token![=],
        ty: syn::Type,
    },
    #[parse(peek = kw::head)]
    HeadDedup {
        head_token: kw::head,
        eq_token: Token![=],
        ty: syn::Type,
    },
    #[parse(peek = kw::rows)]
    Rows {
        rows_token: kw::rows,
        eq_token: Token![=],
        ty: syn::Type,
    },
    #[parse(peek = kw::group)]
    Group {
        group_token: kw::group,
        eq_token: Token![=],
        ty: syn::Type,
    },
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
    pub membership: Option<kw::membership>,
    #[parse(parse_filtered)]
    pub filtered: Option<Filtered>,
}

/// A filtered index retains this projected row schema. Key columns are
/// extracted from it; the remaining columns form each stored item. Constant
/// columns are reconstructed from the selection's program text.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Filtered {
    pub payload_token: kw::payload,
    pub payload: Level,
    pub constants_token: kw::constants,
    #[syn(braced)]
    pub brace: token::Brace,
    #[syn(in = brace)]
    #[parse(CommaList::parse_terminated)]
    pub constants: CommaList<ConstantBinding>,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct ConstantBinding {
    pub column: Ident,
    pub eq_token: Token![=],
    pub value: syntax::constraint::Term,
}

fn parse_filtered(input: syn::parse::ParseStream<'_>) -> syn::Result<Option<Filtered>> {
    if input.peek(kw::payload) {
        input.parse().map(Some)
    } else {
        Ok(None)
    }
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
    /// Checked by-need lookup decisions, keyed by the first semijoin result.
    pub shares: BTreeMap<String, Share>,
    /// Non-column cursor carriers attached to result representations.
    pub witnesses: BTreeMap<String, Witness>,
    /// Probe boundaries with length-known range expansion.
    pub expands: BTreeSet<String>,
    /// Explicit result representations; omitted results keep heading tuples.
    pub repr: BTreeMap<String, Repr>,
    /// Checked batch sizes; omitted boundaries keep scalar evaluation.
    pub stages: BTreeMap<String, usize>,
    /// Independently derived single-base occurrence lineage.
    pub lineage: BTreeMap<String, Lineage>,
    /// Checked base occurrence and complete logical-to-host column mappings.
    pub selections: BTreeMap<String, Selection>,
    /// Explicit key-only builds; they confer neither payload nor ordered reads.
    pub keysets: BTreeMap<(String, Vec<String>), KeySet>,
    /// Positional access decisions attached to existing Physical readers.
    pub reads: BTreeMap<String, Read>,
    /// Distinct runtime filters per scan occurrence, in first-witness order.
    /// Every raw decision's exact covering witness is checked before coalescing.
    pub join_filters: BTreeMap<String, Vec<JoinFilter>>,
    pub physical: physical::Analysis,
    /// Declared column order of every host relation.
    pub host_layouts: BTreeMap<String, Vec<String>>,
    /// Built structures per result: (sorted columns of each level, type).
    pub built: BTreeMap<String, Vec<Built>>,
    /// Exact fallback per host rename, typed reader role/identity, and key.
    pub fallbacks: BTreeMap<(String, ReaderRef<String>, Vec<String>), syn::Type>,
    /// Concrete accumulator tables for keyed Group results.
    pub groups: BTreeMap<String, syn::Type>,
    pub dedup: BTreeMap<String, syn::Type>,
    pub head_dedup: BTreeMap<String, syn::Type>,
    pub retained: BTreeMap<String, syn::Type>,
    /// Explicit projections and constant bindings of index-only selections.
    pub filtered: BTreeMap<String, Filtered>,
    /// Full-key probes certified to observe existence only, not witness payloads.
    pub membership: BTreeSet<(String, Vec<String>)>,
    /// Emitted row layouts, derived from the recorded filtered projections.
    pub layouts: BTreeMap<String, Vec<String>>,
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
            .filter_map(|(levels, ty)| {
                prefix_depth(levels, &wanted).map(|depth| (levels, ty, depth))
            })
            // Complete keys take precedence over candidate prefixes: an
            // ordinary probe needs rows, not the next trie level.
            .min_by_key(|(levels, _, depth)| (levels.len() != *depth, levels.len()))
            .map(|(_, ty, depth)| (ty, depth))
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
/// Formal rule CK-Return: A canonical consumer may own the returned set boundary.
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
        shares: BTreeMap::new(),
        witnesses: BTreeMap::new(),
        expands: BTreeSet::new(),
        repr: BTreeMap::new(),
        stages: BTreeMap::new(),
        lineage: BTreeMap::new(),
        selections: BTreeMap::new(),
        keysets: BTreeMap::new(),
        reads: BTreeMap::new(),
        join_filters: BTreeMap::new(),
        built: BTreeMap::new(),
        fallbacks: BTreeMap::new(),
        groups: BTreeMap::new(),
        dedup: BTreeMap::new(),
        head_dedup: BTreeMap::new(),
        retained: BTreeMap::new(),
        filtered: BTreeMap::new(),
        membership: BTreeSet::new(),
        layouts: BTreeMap::new(),
        unique: BTreeMap::new(),
    };
    analysis.lineage = lineages(&program.physical, &analysis.physical);
    let candidates = physical::candidate_prefixes(&program.physical, &analysis.physical);
    let mut seen = BTreeSet::new();
    for entry in &module.entries {
        let name = symbol_name(&entry.name);
        let Some(heading) = analysis.physical.headings.get(&name).cloned() else {
            return error(&entry.name, "structure must name a Physical result");
        };
        if positional::entry(program, entry, &mut analysis)? {
            continue;
        }
        let keyed = match &entry.structure {
            Structure::Repr { .. } | Structure::Stage { .. } | Structure::Share { .. } => continue,
            Structure::JoinFilter { .. }
            | Structure::Selection { .. }
            | Structure::KeySet { .. }
            | Structure::Read { .. } => {
                unreachable!()
            }
            Structure::Built { keyed, .. } => keyed,
            Structure::Host { reader, keyed, .. } => {
                if keyed.filtered.is_some() {
                    return error(keyed, "a host fallback cannot retain a filtered projection");
                }
                if !(analysis.host_read(&name)
                    || matches!(reader, ReaderRef::Scan)
                        && analysis.physical.hosts.contains_key(&name))
                {
                    return error(
                        &entry.name,
                        "fallback requires a reusable host rename or initial host scan",
                    );
                }
                concrete_type(&keyed.ty)?;
                let columns = keyed.level_names();
                if columns.is_empty() || (columns.len() > 1 && columns.iter().any(Vec::is_empty)) {
                    return error(keyed, "host fallback requires nonempty key levels");
                }
                let mut read_columns = columns.concat();
                read_columns.sort();
                let reader_name = reader.key();
                let valid = match &reader_name {
                    ReaderRef::Scan => columns.len() == 1 && read_columns.is_empty(),
                    ReaderRef::Candidates | ReaderRef::Membership => {
                        candidates.iter().flat_map(|c| &c.sides).any(|(n, p)| {
                            *n == name
                                && read_columns
                                    == if matches!(reader, ReaderRef::Candidates) {
                                        p.clone()
                                    } else {
                                        heading.clone()
                                    }
                        })
                    }
                    ReaderRef::Result(result) => analysis
                        .physical
                        .keyed_reads
                        .get(&name)
                        .into_iter()
                        .flatten()
                        .any(|(r, c, _)| r == result && *c == read_columns),
                };
                if !valid {
                    return error(reader, "fallback does not match a host read");
                }
                if keyed.membership.is_some() {
                    if columns.len() != 1
                        || read_columns != heading
                        || !matches!(reader, ReaderRef::Result(_))
                        || analysis
                            .physical
                            .keyed_reads
                            .get(&name)
                            .into_iter()
                            .flatten()
                            .any(|(_, key, supply)| {
                                key != &read_columns || *supply != SupplyKind::Stream
                            })
                        || analysis
                            .physical
                            .merge_reads
                            .get(&name)
                            .is_some_and(|reads| !reads.is_empty())
                    {
                        return error(
                            keyed,
                            "host membership requires one complete row key and no ordered reads",
                        );
                    }
                    analysis
                        .membership
                        .insert((name.clone(), read_columns.clone()));
                }
                if analysis
                    .fallbacks
                    .insert((name.clone(), reader_name, read_columns), keyed.ty.clone())
                    .is_some()
                {
                    return error(entry, "duplicate host fallback");
                }
                continue;
            }
            Structure::Group { ty, .. } => {
                check_group(program, entry, ty)?;
                if analysis.groups.insert(name, ty.clone()).is_some() {
                    return error(entry, "duplicate group accumulator type");
                }
                continue;
            }
            Structure::Dedup { ty, .. }
            | Structure::HeadDedup { ty, .. }
            | Structure::Rows { ty, .. } => {
                concrete_type(ty)?;
                let map = match &entry.structure {
                    Structure::Dedup { .. } => {
                        if !analysis.physical.canon.contains(&name) {
                            return error(entry, "dedup requires a canon boundary");
                        }
                        &mut analysis.dedup
                    }
                    Structure::HeadDedup { .. } => {
                        if name != symbol_name(&program.physical.program.returned)
                            || !head_needs_dedup(&program.physical, &analysis.physical)
                        {
                            return error(
                                entry,
                                "head dedup requires a noninjective computed head without a canonical consumer",
                            );
                        }
                        &mut analysis.head_dedup
                    }
                    _ => {
                        if analysis.physical.modes[&name] == ModeKind::Pipe {
                            return error(
                                entry,
                                "retained rows require an enumerated let or exists result",
                            );
                        }
                        &mut analysis.retained
                    }
                };
                if map.insert(name, ty.clone()).is_some() {
                    return error(entry, "duplicate storage decision");
                }
                continue;
            }
        };
        concrete_type(&keyed.ty)?;
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
            }
            if analysis.physical.modes[&name] == ModeKind::Pipe {
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
            if !seen.insert((name.clone(), columns.clone())) {
                return error(&entry.name, "duplicate structure on these columns");
            }
            analysis
                .built
                .entry(name.clone())
                .or_default()
                .push((columns, keyed.ty.clone()));
            if keyed.membership.is_some() {
                if keyed.levels.len() != 1 {
                    return error(keyed, "membership storage requires one complete key");
                }
                // Formal rule STR-FullKey: raw set rows can be represented by
                // existence only when every logical column is bound by the key.
                if keyed.filtered.is_none()
                    && (keyed.level_names().concat() != analysis.physical.headings[&name]
                        || analysis
                            .physical
                            .keyed_reads
                            .get(&name)
                            .into_iter()
                            .flatten()
                            .any(|(_, columns, supply)| {
                                columns != &keyed.level_names().concat()
                                    || *supply != SupplyKind::Stream
                            })
                        || analysis
                            .physical
                            .merge_reads
                            .get(&name)
                            .is_some_and(|reads| !reads.is_empty()))
                {
                    return error(
                        keyed,
                        "raw membership storage requires the complete row key and no ordered reads",
                    );
                }
                analysis
                    .membership
                    .insert((name.clone(), keyed.level_names().concat()));
            }
            if let Some(filtered) = &keyed.filtered {
                if analysis
                    .filtered
                    .get(&name)
                    .is_some_and(|previous| previous != filtered)
                {
                    return error(
                        entry,
                        "filtered indices must record an identical projection",
                    );
                }
                analysis.filtered.insert(name, filtered.clone());
            }
        }
    }
    positional::check(program, &analysis)?;
    analysis.shares = share::check(program, &analysis)?;
    analysis.witnesses = repr::check_witnesses(program, &analysis)?;
    analysis.repr = repr::check(program, &analysis)?;
    analysis.stages = stage::check(program, &analysis)?;
    analysis.expands = program
        .structures
        .entries
        .iter()
        .filter(|entry| {
            matches!(
                entry.structure,
                Structure::Stage {
                    expand: Some(_),
                    ..
                }
            )
        })
        .map(|entry| symbol_name(&entry.name))
        .collect();
    filtered::check(program, &analysis)?;
    analysis.layouts = filtered::layouts(program, &analysis);
    let mut shared: BTreeMap<(String, Vec<String>, bool), &syn::Type> = BTreeMap::new();
    for ((name, reader, columns), ty) in &analysis.fallbacks {
        if matches!(reader, ReaderRef::Scan)
            || program.physical.program.definitions.iter().any(|d| {
                matches!(reader, ReaderRef::Result(result) if symbol_name(&d.name) == *result)
                    && matches!(d.operator, Operator::Point(..) | Operator::Range(..))
            })
        {
            continue;
        }
        let ordered = program.physical.program.definitions.iter().any(|d| {
            matches!(reader, ReaderRef::Result(result) if symbol_name(&d.name) == *result)
                && matches!(d.operator, Operator::Merge(..))
        });
        if let Some(other) = shared.insert((name.clone(), columns.clone(), ordered), ty) {
            if other != ty {
                return error(ty, "shared host fallback types on one key must agree");
            }
        }
    }
    for candidate in candidates {
        let v = candidate.column;
        for (name, prefix) in candidate.sides {
            if analysis.host_read(&name) {
                for (reader, key) in [
                    (ReaderRef::Candidates, prefix.clone()),
                    (
                        ReaderRef::Membership,
                        analysis.physical.headings[&name].clone(),
                    ),
                ] {
                    if !analysis
                        .fallbacks
                        .contains_key(&(name.clone(), reader, key))
                    {
                        return error(candidate.reader, "missing intersection host fallback type");
                    }
                }
            } else {
                let valid = analysis
                    .built
                    .get(&name)
                    .into_iter()
                    .flatten()
                    .any(|(levels, _)| {
                        levels.len() == 2 && levels[0] == prefix && levels[1] == [v.clone()]
                    });
                if !valid {
                    return error(
                        candidate.reader,
                        "smallest requires a two-level candidate structure on each retained side",
                    );
                }
            }
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
            let membership = program.physical.program.definitions.iter().any(|d| {
                symbol_name(&d.name) == *reader
                    && matches!(d.operator, Operator::Semi(..) | Operator::Anti(..))
            });
            if !(answered(result, columns)
                || membership
                    && analysis
                        .keysets
                        .contains_key(&(result.clone(), columns.clone())))
            {
                return error(
                    &module.structures_token,
                    format!(
                        "`{reader}` reads `{result}` on {columns:?}; no reusable host read or built structure answers it"
                    ),
                );
            }
            if let Some((ty, depth)) = analysis.built_on(result, columns) {
                let payload = program.physical.program.definitions.iter().any(|d| {
                    symbol_name(&d.name) == *reader && matches!(d.operator, Operator::Probe(..))
                });
                if payload {
                    let levels = &analysis.built[result]
                        .iter()
                        .find(|(levels, t)| t == ty && prefix_depth(levels, columns) == Some(depth))
                        .expect("selected build")
                        .0;
                    if depth != levels.len() && !program.physical.program.definitions.iter().any(|d| symbol_name(&d.name) == *reader && matches!(&d.operator, Operator::Probe(_,k) if matches!(k.choice, Some(physical::Choice::Smallest(_))))) {
                        return error(ty, "a payload probe must read all built levels");
                    }
                }
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
        if matches!(&definition.operator, Operator::Group(group) if !group.keys.is_empty())
            && !analysis.groups.contains_key(&name)
        {
            return error(&definition.name, "missing group accumulator type");
        }
        if definition.canon.is_some() && !analysis.dedup.contains_key(&name) {
            return error(&definition.name, "missing canon dedup type");
        }
        if (analysis.physical.scanned.contains(&name)
            || analysis
                .built
                .get(&name)
                .is_some_and(|built| built.len() > 1 && !analysis.filtered.contains_key(&name))
            || analysis.physical.modes[&name] == ModeKind::Exists)
            && analysis.physical.modes[&name] != ModeKind::Pipe
            && !analysis.retained.contains_key(&name)
            && !analysis.selections.contains_key(&name)
        {
            return error(&definition.name, "missing retained rows type");
        }
        if analysis.physical.hosts.contains_key(&name) {
            if !analysis
                .fallbacks
                .contains_key(&(name.clone(), ReaderRef::Scan, Vec::new()))
            {
                return error(&definition.name, "missing host scan type");
            }
            for (reader, columns, _) in analysis
                .physical
                .keyed_reads
                .get(&name)
                .into_iter()
                .flatten()
            {
                if !analysis.host_read(&name) || analysis.reads.contains_key(reader) {
                    continue;
                }
                if !analysis.fallbacks.contains_key(&(
                    name.clone(),
                    ReaderRef::Result(reader.clone()),
                    columns.clone(),
                )) {
                    return error(
                        &definition.name,
                        format!("missing host fallback for `{reader}`"),
                    );
                }
            }
        }
        let op = &definition.operator;
        let Some(heading) = analysis.physical.headings.get(&name).cloned() else {
            return error(
                &module.structures_token,
                format!("result `{name}` has no heading"),
            );
        };
        let key =
            physical::inherited_key(op, &heading, &program.physical.program.host_sets, |id| {
                analysis.unique.get(&symbol_name(id)).cloned()
            });
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
    if !analysis.unique.contains_key(&returned)
        && program.physical.program.return_canon.is_none()
        && !analysis.physical.consumer_canonical
    {
        return error(
            &module.structures_token,
            format!(
                "returned `{returned}` is not canonical: flag it `canon` or make its inputs canonical"
            ),
        );
    }
    if head_needs_dedup(&program.physical, &analysis.physical)
        && !analysis.head_dedup.contains_key(&returned)
    {
        return error(
            &program.physical.program.returned,
            "missing computed-head dedup type",
        );
    }
    coalesce_join_filters(program, &mut analysis);
    Ok(analysis)
}

/// Formal rule STR-Group: Exactly keyed Group results record a concrete accumulator table.
fn check_group(program: &Program, entry: &Entry, ty: &syn::Type) -> syn::Result<()> {
    concrete_type(ty)?;
    if !program
        .physical
        .program
        .definitions
        .iter()
        .any(|definition| {
            definition.name == entry.name
                && matches!(&definition.operator, Operator::Group(group) if !group.keys.is_empty())
        })
    {
        return error(entry, "group accumulator requires a keyed Group result");
    }
    Ok(())
}

/// Shared syntax used by this language.
pub use cq_syntax as syntax;

/// Whether computed answers can collapse distinct canonical valuations.
pub fn head_needs_dedup(program: &physical::Program, facts: &physical::Analysis) -> bool {
    if facts.consumer_canonical {
        return false;
    }
    if program.program.return_canon.is_some() {
        return true;
    }
    let head = &program.program.head;
    head.args
        .iter()
        .any(|e| cq_syntax::rust::variable(e).is_none())
        && facts.headings[&symbol_name(&program.program.returned)]
            .iter()
            .any(|c| {
                !head
                    .args
                    .iter()
                    .filter_map(cq_syntax::rust::variable)
                    .any(|v| symbol_name(v) == *c)
            })
}

/// Concrete representation names may contain inference parameters, never macro slots.
pub fn concrete_type(ty: &syn::Type) -> syn::Result<()> {
    struct Macros(bool);
    impl<'ast> syn::visit::Visit<'ast> for Macros {
        fn visit_type_macro(&mut self, _: &'ast syn::TypeMacro) {
            self.0 = true;
        }
    }
    let mut macros = Macros(false);
    syn::visit::Visit::visit_type(&mut macros, ty);
    if macros.0 || matches!(ty, syn::Type::Infer(_)) {
        return error(ty, "record a concrete Rust representation type");
    }
    Ok(())
}

/// Structures rewrites preserve Physical and discharge every read and boundary.
pub fn check_rewrite(source: &Program, target: &Program) -> syn::Result<()> {
    contract::check(source)?;
    contract::check(target)?;
    if source.physical != target.physical
        || quote::ToTokens::to_token_stream(&source.physical).to_string()
            != quote::ToTokens::to_token_stream(&target.physical).to_string()
    {
        return error(
            &target.physical,
            "Structures rewrite must preserve Physical AST and tokens exactly",
        );
    }
    Ok(())
}

/// Compare an entry's semantic layout while allowing its storage type to change.
/// Host readers and all non-storage syntax remain exact, including opaque tokens.
pub fn storage_layout_eq(left: &Entry, right: &Entry) -> bool {
    if left.name != right.name
        || std::mem::discriminant(&left.structure) != std::mem::discriminant(&right.structure)
    {
        return false;
    }
    let mut normalized = left.clone();
    match (&mut normalized.structure, &right.structure) {
        (Structure::Built { keyed: left, .. }, Structure::Built { keyed: right, .. }) => {
            left.ty = right.ty.clone()
        }
        (Structure::Dedup { ty: left, .. }, Structure::Dedup { ty: right, .. })
        | (Structure::HeadDedup { ty: left, .. }, Structure::HeadDedup { ty: right, .. })
        | (Structure::Rows { ty: left, .. }, Structure::Rows { ty: right, .. })
        | (Structure::Group { ty: left, .. }, Structure::Group { ty: right, .. })
        | (Structure::Selection { ty: left, .. }, Structure::Selection { ty: right, .. })
        | (Structure::KeySet { ty: left, .. }, Structure::KeySet { ty: right, .. }) => {
            *left = right.clone()
        }
        (
            Structure::JoinFilter { filter: left, .. },
            Structure::JoinFilter { filter: right, .. },
        ) => left.ty = right.ty.clone(),
        _ => {}
    }
    normalized == *right
        && normalized.to_token_stream().to_string() == right.to_token_stream().to_string()
}

/// Check catalog retention across the Physical/Structures boundary.
///
/// Unlike an ordinary Structures rewrite, this permits checked host renames
/// to move from `pipe` to `let`, replacing keyed fallbacks with built indexes.
/// Operators, their order, opaque syntax, scan adapters and existing layouts
/// remain unchanged. Storage implementations may be retyped.
pub fn check_retention_rewrite(source: &Program, target: &Program) -> syn::Result<()> {
    contract::check(source)?;
    let facts = contract::check(target)?;
    let mut normalized = target.physical.clone();
    if source.physical.program.definitions.len() != normalized.program.definitions.len() {
        return error(
            target,
            "catalog retention cannot add or remove Physical definitions",
        );
    }
    for (old, new) in source
        .physical
        .program
        .definitions
        .iter()
        .zip(&mut normalized.program.definitions)
    {
        if old.mode != new.mode {
            if !matches!(
                (&old.mode, &new.mode, &old.operator),
                (
                    physical::Mode::Pipe(_),
                    physical::Mode::Let(_),
                    physical::Operator::Rename(_)
                )
            ) {
                return error(
                    new,
                    "catalog retention only permits a host rename from pipe to let",
                );
            }
            new.mode = old.mode.clone();
        }
    }
    if normalized != source.physical
        || normalized.to_token_stream().to_string() != source.physical.to_token_stream().to_string()
    {
        return error(
            target,
            "catalog retention must preserve all non-mode Physical syntax exactly",
        );
    }
    physical::check_rewrite(&source.physical, &target.physical)?;
    let same_entry = storage_layout_eq;
    for entry in &source.structures.entries {
        if target
            .structures
            .entries
            .iter()
            .any(|other| same_entry(entry, other))
        {
            continue;
        }
        if let Structure::Host { reader, keyed, .. } = &entry.structure
            && !matches!(reader, ReaderRef::Scan)
            && facts.physical.modes.get(&symbol_name(&entry.name)) == Some(&physical::ModeKind::Let)
        {
            let mut keys = keyed.level_names().concat();
            keys.sort();
            if facts.built_on(&symbol_name(&entry.name), &keys).is_some() {
                continue;
            }
        }
        return error(
            entry,
            "catalog retention must preserve scans, opaque adapters and existing storage layouts",
        );
    }
    for entry in &target.structures.entries {
        if !matches!(entry.structure, Structure::Built { .. })
            && !source
                .structures
                .entries
                .iter()
                .any(|other| same_entry(other, entry))
        {
            return error(
                entry,
                "catalog retention may add built indexes, not new opaque adapters",
            );
        }
    }
    Ok(())
}

pub mod positional;
pub use positional::*;
mod join_filter;
pub use join_filter::*;
mod repr;
pub use repr::Witness;
pub use repr::{Repr, ReprBase, representation_bases};
mod share;
pub use share::{Share, leveled_host_read};
mod stage;

mod filtered;
pub use filtered::{reconstructible_constants, required_columns, selection_constants};

impl cq_stage::Language for Program {
    fn context(&self) -> cq_stage::Context {
        cq_stage::Language::context(&self.physical)
    }
    fn check(&self) -> syn::Result<()> {
        contract::check(self).map(|_| ())
    }
    fn check_rewrite(source: &Self, target: &Self) -> syn::Result<()> {
        check_rewrite(source, target)
    }
}
