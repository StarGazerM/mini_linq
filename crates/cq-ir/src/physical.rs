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

use std::collections::{BTreeMap, BTreeSet};

use syn::{Ident, Token, token};

use crate::{CommaList, cq, index_requirements as ix, relational_plan as ra, symbol_name};

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
pub struct Module {
    pub index_requirements: ix::Module,
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
    pub fn input(&self, logical: &ra::Operator) -> Option<Ident> {
        match (self, logical) {
            (Operator::Probe(_, k) | Operator::Merge(_, k) | Operator::Semi(_, k), _) => {
                Some(k.left.clone())
            }
            (_, ra::Operator::Select(s)) => Some(s.input.clone()),
            (_, ra::Operator::Extend(e)) => Some(e.input.clone()),
            (_, ra::Operator::Project(p)) => Some(p.input.clone()),
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

pub mod contract {
    pub fn check(module: &super::Module) -> syn::Result<super::Analysis> {
        super::ix::contract::check(&module.index_requirements)?;
        super::analyze(module)
    }

    pub fn well_formed(module: &super::Module) -> bool {
        check(module).is_ok()
    }
}

fn error<T>(spanned: &impl quote::ToTokens, message: impl std::fmt::Display) -> syn::Result<T> {
    Err(syn::Error::new_spanned(spanned, message))
}

fn analyze(module: &Module) -> syn::Result<Analysis> {
    let ix = &module.index_requirements;
    let plan = &ix.relational_plan.plan;
    let results = ra::check_in(ix.relational_plan.program.inputs(), plan)?;
    let demands: BTreeSet<_> = ix
        .indexes
        .iter()
        .map(|index| ix::signature(&index.requirement))
        .collect();
    let mut analysis = Analysis {
        headings: results
            .iter()
            .map(|(n, i)| (n.clone(), i.heading.iter().cloned().collect()))
            .collect(),
        ..Analysis::default()
    };
    let program = &module.program;

    // One physical definition per logical result, same order.
    if program.definitions.len() != plan.definitions.len() {
        return error(
            &module.physical_token,
            format!(
                "physical restates every RA result: {} logical, {} physical",
                plan.definitions.len(),
                program.definitions.len()
            ),
        );
    }
    let mut readers: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (definition, ra_definition) in program.definitions.iter().zip(&plan.definitions) {
        let name = symbol_name(&definition.name);
        if name != symbol_name(&ra_definition.result) {
            return error(
                &definition.name,
                format!("expected `{}` here, in RA order", ra_definition.result),
            );
        }
        let mode = ModeKind::from(&definition.mode);
        analysis.modes.insert(name.clone(), mode);
        if definition.canon.is_some() {
            analysis.canon.insert(name.clone());
        }
        let ra_op = &ra_definition.operator;
        let mismatch = |what: &str| -> syn::Result<()> {
            error(
                &definition.operator,
                format!("`{name}` is {what} in RA; the physical form must match"),
            )
        };
        match (&definition.operator, ra_op) {
            (Operator::Unit(_), ra::Operator::Unit(_)) => {}
            (Operator::Rename(_), ra::Operator::Rename(r)) => {
                analysis
                    .hosts
                    .insert(name.clone(), symbol_name(&r.relation));
                analysis.renames.insert(
                    name.clone(),
                    r.mapping
                        .iter()
                        .map(|m| (symbol_name(&m.target), symbol_name(&m.source)))
                        .collect(),
                );
            }
            (Operator::Select(_), ra::Operator::Select(s)) => {
                if cq::Predicate::from_expr(&s.condition).is_some() {
                    analysis.fragment.insert(name.clone());
                }
            }
            (Operator::Range(_, ranged), ra::Operator::Select(s)) => {
                let column = symbol_name(&ranged.column);
                let input = symbol_name(&s.input);
                if !results[&input].heading.contains(&column) {
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
            (Operator::Extend(_), ra::Operator::Extend(_)) => {}
            (Operator::Project(_), ra::Operator::Project(_)) => {}
            (Operator::Probe(_, k) | Operator::Merge(_, k), ra::Operator::NaturalJoin(j)) => {
                check_operands(k, &j.left, &j.right)?;
                let columns = join_columns(
                    k,
                    &results[&symbol_name(&j.left)],
                    &results[&symbol_name(&j.right)],
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
            (Operator::Semi(_, k), ra::Operator::Semijoin(j)) => {
                check_operands(k, &j.left, &j.right)?;
                let columns = join_columns(
                    k,
                    &results[&symbol_name(&j.left)],
                    &results[&symbol_name(&j.right)],
                )?;
                analysis
                    .key_reads
                    .entry(symbol_name(&k.right))
                    .or_default()
                    .push((name.clone(), columns));
            }
            (_, ra::Operator::Unit(_)) => mismatch("unit")?,
            (_, ra::Operator::Rename(_)) => mismatch("a rename")?,
            (_, ra::Operator::Select(_)) => mismatch("a select")?,
            (_, ra::Operator::Extend(_)) => mismatch("an extend")?,
            (_, ra::Operator::Project(_)) => mismatch("a project")?,
            (_, ra::Operator::NaturalJoin(_)) => mismatch("a natural join")?,
            (_, ra::Operator::Semijoin(_)) => mismatch("a semijoin")?,
        }
        // Reads: the input stream, and merge sides, are enumerated.
        if let Some(input) = definition.operator.input(ra_op) {
            let input = symbol_name(&input);
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
        if mode == ModeKind::Alias && !matches!(ra_op, ra::Operator::Rename(_)) {
            return error(&definition.mode, "only a rename of a host can be an alias");
        }
    }
    let returned = symbol_name(&program.returned);
    if !analysis.modes.contains_key(&returned) {
        return error(&program.returned, "return names no result");
    }
    if returned != symbol_name(&plan.output.input) {
        return error(
            &program.returned,
            format!("RA outputs `{}`", plan.output.input),
        );
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

fn check_operands(keyed: &Keyed, left: &Ident, right: &Ident) -> syn::Result<()> {
    if symbol_name(&keyed.left) != symbol_name(left)
        || symbol_name(&keyed.right) != symbol_name(right)
    {
        return error(
            &keyed.left,
            format!("operands must be `{left}, {right}` as in RA"),
        );
    }
    Ok(())
}

fn join_columns(
    keyed: &Keyed,
    left: &ra::RelationInfo,
    right: &ra::RelationInfo,
) -> syn::Result<Vec<String>> {
    let shared: BTreeSet<String> = left.heading.intersection(&right.heading).cloned().collect();
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
