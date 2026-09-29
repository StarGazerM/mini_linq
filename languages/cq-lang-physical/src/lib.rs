//! Physical: IX with one physical operator per result, in linear graded ANF.
//!
//! The definitions restate IX's results by name with the physical form the
//! policy chose for each: a natural join is a `probe` (the right result read
//! through a key), a `merge` (both sides co-iterated in key order), a
//! semijoin is `semi` (membership on the key), a select whose input is read
//! through an ordered key may be a `range`. Every other operator keeps its
//! logical name; its detail (mapping, condition, value, kept columns) lives
//! in the RA definition of the same name and is not repeated.
//! Each probe/semi's right operand and join columns must have a matching IX
//! demand. These uses are derived from operands, without separate use syntax.
//!
//! Denotation is the set IX denotes. What this language adds is the
//! representation discipline that a printer relies on:
//!
//! - **mode**: a `pipe` is a stream read exactly once by the operator that
//!   consumes it as its input; a `let` is a value that structures are built
//!   over and read through a key; an `alias` is a host relation read in
//!   place. Any result read through a key, or read twice, is a `let`.
//! - **canon**: a result flagged `canon` is inserted into a canonical index
//!   (a set) — in `pipe` mode by the endpoint's sink, in `let` mode as a
//!   materialized set. The returned result must be canonical, by this flag or
//!   by key inference under the structures chosen later.
//! - **grades** (order on a key, uniqueness by a key) are premises of
//!   `merge` and of the return; they are discharged by the Structures stage,
//!   which knows the hosts. This language only fixes where they are needed.
//!
//! Nothing here names a Rust type, a loop, or a variable: operands are
//! result names, columns are attribute names.
//!
//! ```text
//! physical {
//!     r0 = alias rename;
//!     r1 = let rename;
//!     r2 = pipe probe[y] r0, r1;
//!     r3 = pipe select r2;
//!     r4 = pipe canon project r3;
//!     return r4;
//! }
//! ```
//!
//! ```text
//! Module     ::= IX "physical" "{" Definition* "return" Ident ";" "}"
//! Definition ::= Ident "=" Mode ["canon"] Operator ";"
//! Mode       ::= "pipe" | "let" | "alias"
//! Operator   ::= "unit" | "rename" | "select" | "extend" | "project"
//!              | "probe" "[" Ident,* "]" Ident "," Ident
//!              | "merge" "[" Ident,* "]" Ident "," Ident
//!              | "semi"  "[" Ident,* "]" Ident "," Ident
//!              | "range" "[" Ident "]"
//! ```

#[cfg(feature = "reference-prefix")]
pub use cq_lang_ix as ix;

use std::collections::{BTreeMap, BTreeSet};

use syn::{Ident, Token, token};

pub use cq_syntax::{CommaList, symbol_name};
#[cfg(feature = "reference-prefix")]
pub use ix::relational_plan as ra;
#[cfg(feature = "reference-prefix")]
pub use ra::cq;

pub mod kw {
    syn::custom_keyword!(physical);
    syn::custom_keyword!(pipe);
    syn::custom_keyword!(alias);
    syn::custom_keyword!(canon);
    syn::custom_keyword!(unit);
    syn::custom_keyword!(rename);
    syn::custom_keyword!(select);
    syn::custom_keyword!(extend);
    syn::custom_keyword!(project);
    syn::custom_keyword!(probe);
    syn::custom_keyword!(merge);
    syn::custom_keyword!(semi);
    syn::custom_keyword!(range);
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Section {
    pub physical_token: kw::physical,
    #[syn(braced)]
    pub brace: token::Brace,
    #[syn(in = brace)]
    pub program: Program,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Program {
    #[syn(repeat, until = Token![return])]
    pub definitions: Vec<Definition>,
    pub return_token: Token![return],
    pub returned: Ident,
    pub return_semi: Token![;],
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Definition {
    pub name: Ident,
    pub eq_token: Token![=],
    pub mode: Mode,
    pub canon: Option<kw::canon>,
    pub operator: Operator,
    pub semi_token: Token![;],
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum Mode {
    #[parse(peek = kw::pipe)]
    Pipe(kw::pipe),
    #[parse(peek = Token![let])]
    Let(Token![let]),
    #[parse(peek = kw::alias)]
    Alias(kw::alias),
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum Operator {
    #[parse(peek = kw::unit)]
    Unit(kw::unit),
    #[parse(peek = kw::rename)]
    Rename(kw::rename),
    #[parse(peek = kw::select)]
    Select(kw::select),
    #[parse(peek = kw::extend)]
    Extend(kw::extend),
    #[parse(peek = kw::project)]
    Project(kw::project),
    #[parse(peek = kw::probe)]
    Probe(kw::probe, Keyed),
    #[parse(peek = kw::merge)]
    Merge(kw::merge, Keyed),
    #[parse(peek = kw::semi)]
    Semi(kw::semi, Keyed),
    #[parse(peek = kw::range)]
    Range(kw::range, Ranged),
}

/// `[a, b] left, right`: the join columns by name, then both operands.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Keyed {
    #[syn(bracketed)]
    pub bracket: token::Bracket,
    #[syn(in = bracket)]
    #[parse(CommaList::parse_terminated)]
    pub columns: CommaList<Ident>,
    pub left: Ident,
    pub comma: Token![,],
    pub right: Ident,
}

/// `[c]`: the ordered column the select's bounds apply to.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Ranged {
    #[syn(bracketed)]
    pub bracket: token::Bracket,
    #[syn(in = bracket)]
    pub column: Ident,
}

impl Operator {
    /// The result read as this operator's input stream, if any.
    pub fn input(&self, logical: &LogicalOp) -> Option<String> {
        match (self, logical) {
            (Operator::Probe(_, k) | Operator::Merge(_, k) | Operator::Semi(_, k), _) => {
                Some(symbol_name(&k.left))
            }
            (_, LogicalOp::Select { input, .. }) => Some(input.clone()),
            (_, LogicalOp::Extend { input, .. }) => Some(input.clone()),
            (_, LogicalOp::Project { input, .. }) => Some(input.clone()),
            _ => None,
        }
    }

    /// The result read through a key, with the key columns, if any.
    pub fn keyed(&self) -> Option<(&Ident, Vec<String>)> {
        match self {
            Operator::Probe(_, k) | Operator::Semi(_, k) => {
                Some((&k.right, k.columns.iter().map(symbol_name).collect()))
            }
            _ => None,
        }
    }
}

/// How a result's grades and reads are known once the contract holds.
#[derive(Clone, Debug, Default)]
pub struct Analysis {
    /// Sorted heading of every result, from RA.
    pub headings: BTreeMap<String, Vec<String>>,
    /// The host relation behind every `rename`, by result name.
    pub hosts: BTreeMap<String, String>,
    /// Column mapping of every `rename`: result attribute → host attribute.
    pub renames: BTreeMap<String, BTreeMap<String, String>>,
    /// For every result read through a key: (reader, key columns).
    pub key_reads: BTreeMap<String, Vec<(String, Vec<String>)>>,
    /// For every result read as a merge side: (reader, key columns).
    pub merge_reads: BTreeMap<String, Vec<(String, Vec<String>)>>,
    /// For every result read as a range: (reader, ordered column).
    pub range_reads: BTreeMap<String, Vec<(String, String)>>,
    /// Results whose rows are enumerated (pipe input, scan of a let).
    pub scanned: BTreeSet<String>,
    /// Select conditions inside the total comparison fragment.
    pub fragment: BTreeSet<String>,
    /// Results flagged `canon`.
    pub canon: BTreeSet<String>,
    /// Mode of every result.
    pub modes: BTreeMap<String, ModeKind>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeKind {
    Pipe,
    Let,
    Alias,
}

impl From<&Mode> for ModeKind {
    fn from(mode: &Mode) -> Self {
        match mode {
            Mode::Pipe(_) => ModeKind::Pipe,
            Mode::Let(_) => ModeKind::Let,
            Mode::Alias(_) => ModeKind::Alias,
        }
    }
}

/// Exactly the logical facts inspected by the Physical contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogicalOp {
    Unit,
    Rename,
    Select { input: String },
    Extend { input: String },
    Project { input: String, keep: Vec<String> },
    Join { left: String, right: String },
    Semijoin { left: String, right: String },
}

/// A producer supplies facts; no producer language type crosses this boundary.
pub trait IxEnv {
    fn results(&self) -> Vec<(String, LogicalOp)>;
    fn heading(&self, result: &str) -> Option<&[String]>;
    fn output(&self) -> String;
    fn demands(&self) -> BTreeSet<(String, Vec<usize>)>;
    fn host_of(&self, result: &str) -> Option<(String, BTreeMap<String, String>)>;
    fn select_condition(&self, result: &str) -> Option<&syn::Expr>;
}

pub mod contract {
    pub fn check(
        section: &super::Section,
        env: &impl super::IxEnv,
    ) -> syn::Result<super::Analysis> {
        super::analyze(section, env)
    }
    pub fn well_formed(section: &super::Section, env: &impl super::IxEnv) -> bool {
        check(section, env).is_ok()
    }
    #[cfg(feature = "reference-prefix")]
    pub fn check_module(module: &super::Module) -> syn::Result<super::Analysis> {
        check(
            &module.section,
            &super::reference::Env::new(&module.index_requirements)?,
        )
    }
    #[cfg(feature = "reference-prefix")]
    pub fn well_formed_module(module: &super::Module) -> bool {
        check_module(module).is_ok()
    }
}

fn error<T>(spanned: &impl quote::ToTokens, message: impl std::fmt::Display) -> syn::Result<T> {
    Err(syn::Error::new_spanned(spanned, message))
}

fn analyze(module: &Section, env: &impl IxEnv) -> syn::Result<Analysis> {
    let results = env.results();
    let demands = env.demands();
    let mut headings = BTreeMap::new();
    for (name, _) in &results {
        let Some(heading) = env.heading(name) else {
            return error(
                &module.physical_token,
                format!("result `{name}` has no heading"),
            );
        };
        headings.insert(name.clone(), heading.to_vec());
    }
    let mut analysis = Analysis {
        headings,
        ..Analysis::default()
    };
    let program = &module.program;

    // One physical definition per logical result, same order.
    if program.definitions.len() != results.len() {
        return error(
            &module.physical_token,
            format!(
                "physical restates every RA result: {} logical, {} physical",
                results.len(),
                program.definitions.len()
            ),
        );
    }
    let mut readers: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (definition, (logical_name, ra_op)) in program.definitions.iter().zip(&results) {
        let name = symbol_name(&definition.name);
        if name != *logical_name {
            return error(
                &definition.name,
                format!("expected `{}` here, in RA order", logical_name),
            );
        }
        let mode = ModeKind::from(&definition.mode);
        analysis.modes.insert(name.clone(), mode);
        if definition.canon.is_some() {
            analysis.canon.insert(name.clone());
        }
        let mismatch = |what: &str| -> syn::Result<()> {
            error(
                &definition.operator,
                format!("`{name}` is {what} in RA; the physical form must match"),
            )
        };
        match (&definition.operator, ra_op) {
            (Operator::Unit(_), LogicalOp::Unit) => {}
            (Operator::Rename(_), LogicalOp::Rename) => {
                let Some((host, mapping)) = env.host_of(&name) else {
                    return error(&definition.name, "rename has no host layout");
                };
                analysis.hosts.insert(name.clone(), host);
                analysis.renames.insert(name.clone(), mapping);
            }
            (Operator::Select(_), LogicalOp::Select { .. }) => {
                if env
                    .select_condition(&name)
                    .and_then(cq_syntax::predicate::Predicate::from_expr)
                    .is_some()
                {
                    analysis.fragment.insert(name.clone());
                }
            }
            (Operator::Range(_, ranged), LogicalOp::Select { input }) => {
                let column = symbol_name(&ranged.column);
                let input = input.clone();
                if !analysis
                    .headings
                    .get(&input)
                    .is_some_and(|h| h.contains(&column))
                {
                    return error(
                        &ranged.column,
                        format!("`{input}` has no column `{column}`"),
                    );
                }
                analysis
                    .range_reads
                    .entry(input)
                    .or_default()
                    .push((name.clone(), column));
            }
            (Operator::Extend(_), LogicalOp::Extend { .. }) => {}
            (Operator::Project(_), LogicalOp::Project { .. }) => {}
            (Operator::Probe(_, k) | Operator::Merge(_, k), LogicalOp::Join { left, right }) => {
                check_operands(k, left, right)?;
                let columns =
                    join_columns(
                        k,
                        analysis.headings.get(left).ok_or_else(|| {
                            syn::Error::new_spanned(&k.left, "unknown left result")
                        })?,
                        analysis.headings.get(right).ok_or_else(|| {
                            syn::Error::new_spanned(&k.right, "unknown right result")
                        })?,
                    )?;
                let right = symbol_name(&k.right);
                if matches!(definition.operator, Operator::Probe(..)) {
                    analysis
                        .key_reads
                        .entry(right)
                        .or_default()
                        .push((name.clone(), columns));
                } else {
                    for side in [&k.left, &k.right] {
                        analysis
                            .merge_reads
                            .entry(symbol_name(side))
                            .or_default()
                            .push((name.clone(), columns.clone()));
                    }
                }
            }
            (Operator::Semi(_, k), LogicalOp::Semijoin { left, right }) => {
                check_operands(k, left, right)?;
                let columns =
                    join_columns(
                        k,
                        analysis.headings.get(left).ok_or_else(|| {
                            syn::Error::new_spanned(&k.left, "unknown left result")
                        })?,
                        analysis.headings.get(right).ok_or_else(|| {
                            syn::Error::new_spanned(&k.right, "unknown right result")
                        })?,
                    )?;
                analysis
                    .key_reads
                    .entry(symbol_name(&k.right))
                    .or_default()
                    .push((name.clone(), columns));
            }
            (_, LogicalOp::Unit) => mismatch("unit")?,
            (_, LogicalOp::Rename) => mismatch("a rename")?,
            (_, LogicalOp::Select { .. }) => mismatch("a select")?,
            (_, LogicalOp::Extend { .. }) => mismatch("an extend")?,
            (_, LogicalOp::Project { .. }) => mismatch("a project")?,
            (_, LogicalOp::Join { .. }) => mismatch("a natural join")?,
            (_, LogicalOp::Semijoin { .. }) => mismatch("a semijoin")?,
        }
        // Reads: the input stream, and merge sides, are enumerated.
        if let Some(input) = definition.operator.input(ra_op) {
            readers.entry(input.clone()).or_default().push(name.clone());
            analysis.scanned.insert(input);
        }
        if let Operator::Merge(_, k) = &definition.operator {
            // A merge side is read through its order: a `let` side through a
            // structure, a pipe side as the stream itself.
            analysis.scanned.remove(&symbol_name(&k.left));
            for side in [&k.left, &k.right] {
                let side = symbol_name(side);
                if side != symbol_name(&k.left) {
                    readers.entry(side.clone()).or_default().push(name.clone());
                }
            }
        }
        if let Some((right, mut columns)) = definition.operator.keyed() {
            let right_name = symbol_name(right);
            columns.sort();
            columns.dedup();
            if !demands.contains(&(
                right_name.clone(),
                ordinals(&analysis, &right_name, &columns),
            )) {
                return error(
                    &definition.operator,
                    format!(
                        "probe `{name}` needs demand `@{right_name}[{}]`",
                        columns.join(",")
                    ),
                );
            }
            readers
                .entry(symbol_name(right))
                .or_default()
                .push(name.clone());
        }
        // Alias only over a host rename.
        if mode == ModeKind::Alias && !matches!(ra_op, LogicalOp::Rename) {
            return error(&definition.mode, "only a rename of a host can be an alias");
        }
    }
    let returned = symbol_name(&program.returned);
    if !analysis.modes.contains_key(&returned) {
        return error(&program.returned, "return names no result");
    }
    if returned != env.output() {
        return error(&program.returned, format!("RA outputs `{}`", env.output()));
    }
    readers
        .entry(returned.clone())
        .or_default()
        .push("return".into());
    analysis.scanned.insert(returned.clone());

    // Modes: a pipe is read exactly once, as a stream; anything read
    // through a key or read twice is a let (or a host alias).
    for definition in &program.definitions {
        let name = symbol_name(&definition.name);
        let mode = analysis.modes[&name];
        let reads = readers.get(&name).map_or(0, Vec::len);
        match mode {
            ModeKind::Pipe => {
                if reads != 1 {
                    return error(
                        &definition.name,
                        format!(
                            "pipe `{name}` must be read exactly once; it is read {reads} times"
                        ),
                    );
                }
                if analysis.key_reads.contains_key(&name)
                    || analysis.range_reads.contains_key(&name)
                {
                    return error(
                        &definition.name,
                        format!("`{name}` is read through a key, so it must be `let`"),
                    );
                }
            }
            ModeKind::Let => {
                if reads == 0 {
                    return error(&definition.name, format!("`{name}` is never read"));
                }
            }
            ModeKind::Alias => {}
        }
    }
    Ok(analysis)
}

fn check_operands(keyed: &Keyed, left: &str, right: &str) -> syn::Result<()> {
    if symbol_name(&keyed.left) != left || symbol_name(&keyed.right) != right {
        return error(
            &keyed.left,
            format!("operands must be `{left}, {right}` as in RA"),
        );
    }
    Ok(())
}

fn join_columns(keyed: &Keyed, left: &[String], right: &[String]) -> syn::Result<Vec<String>> {
    let shared: BTreeSet<String> = left.iter().filter(|c| right.contains(c)).cloned().collect();
    let written: BTreeSet<String> = keyed.columns.iter().map(symbol_name).collect();
    if written != shared {
        return Err(syn::Error::new(
            keyed.bracket.span.join(),
            format!("join columns are {shared:?}, written {written:?}"),
        ));
    }
    Ok(shared.into_iter().collect())
}

/// Erase physical forms: the IX module every Physical program restates.
#[cfg(feature = "reference-prefix")]
pub fn erase(module: &Module) -> ix::Module {
    module.index_requirements.clone()
}

/// The join columns of a keyed read, as sorted-heading ordinals of `result`.
pub fn ordinals(analysis: &Analysis, result: &str, columns: &[String]) -> Vec<usize> {
    let heading = &analysis.headings[result];
    columns
        .iter()
        .map(|c| {
            heading
                .iter()
                .position(|h| h == c)
                .expect("column in heading")
        })
        .collect()
}

/// The local binary intersection recognized by selection and realization:
/// one pipe probe introduces one column, immediately followed by a probe
/// or semijoin that only checks it. Both candidate relations are materialized,
/// the prefixes are nonempty, and the new column completes each heading.
/// This intentionally excludes wider intersections, intervening operators,
/// canonicalization of the intermediate pipe, and host-only access.
pub fn binary_intersection<'a>(
    first: &'a Definition,
    second: &'a Definition,
    analysis: &Analysis,
) -> Option<(&'a Keyed, &'a Keyed, String)> {
    let Operator::Probe(_, a) = &first.operator else {
        return None;
    };
    let (Operator::Probe(_, b) | Operator::Semi(_, b)) = &second.operator else {
        return None;
    };
    if !matches!(first.mode, Mode::Pipe(_)) || first.canon.is_some() || b.left != first.name {
        return None;
    }
    let left = &analysis.headings[&symbol_name(&a.left)];
    let ah = &analysis.headings[&symbol_name(&a.right)];
    let bh = &analysis.headings[&symbol_name(&b.right)];
    let added: Vec<_> = ah.iter().filter(|c| !left.contains(c)).collect();
    if added.len() != 1 {
        return None;
    }
    let v = added[0];
    let ak: Vec<_> = a.columns.iter().map(symbol_name).collect();
    let bk: Vec<_> = b.columns.iter().map(symbol_name).collect();
    if ak.is_empty()
        || bk.len() < 2
        || !bk.contains(v)
        || ah.len() != ak.len() + 1
        || bh.len() != bk.len()
        || !bh.iter().all(|c| left.contains(c) || c == v)
        || [&a.right, &b.right]
            .iter()
            .any(|r| analysis.modes[&symbol_name(r)] != ModeKind::Let)
    {
        return None;
    }
    Some((a, b, v.clone()))
}

pub mod policy;

/// Shared syntax used by this language.
pub use cq_syntax as syntax;

#[cfg(feature = "reference-prefix")]
pub mod reference;
#[cfg(feature = "reference-prefix")]
pub use reference::Module;

/// Source facts read by Physical consumers, including the Rust expressions
/// carried through to execution. No producer language types occur here.
pub trait Env: IxEnv {
    /// Declared host layouts, types and structure-selection advice.
    fn inputs(&self) -> Vec<cq_syntax::RelationDecl>;
    fn head(&self) -> &cq_syntax::Atom;
    /// Logical column to Rust binder, retaining the caller's identifier hygiene.
    fn bindings(&self, result: &str) -> BTreeMap<String, Ident>;
    /// Columns owned because they were introduced by an Extend.
    fn computed(&self, result: &str) -> BTreeSet<String>;
    fn extension(&self, result: &str) -> Option<(Vec<cq_syntax::ColumnDecl>, syn::Expr)>;
}
