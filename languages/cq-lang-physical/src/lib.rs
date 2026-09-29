//! Complete physical programs with detailed operators and linear read modes.
//!
//! `contract::check` derives headings, source identifier bindings, computed
//! columns, hosts, renames, reads and modes from this program alone. A pipe
//! has exactly one stream read; keyed and repeated reads require retained
//! storage or a host rename. Structures discharges order and uniqueness
//! premises. IX demands and relational erasure belong to the lowering pass.
//!
//! ```text
//! Program ::= "physical" "{" RelationDecl* Definition* "return" Ident "as" Atom ";" "}"
//! Definition ::= Ident "=" Mode ["canon"] Operator ";"
//! Mode ::= "pipe" | "let"
//! Operator ::= Unit | Rename | Select | Extend | Project
//!            | "probe" "[" Ident,* "]" Ident "," Ident
//!            | "merge" "[" Ident,* "]" Ident "," Ident
//!            | "semi" "[" Ident,* "]" Ident "," Ident
//!            | "range" "[" Ident "]" Ident "if" "(" Expr ")"
//! ```
//!
//! Unit, Rename, Select, Extend and Project are shared algebra syntax.

use std::collections::{BTreeMap, BTreeSet};

use syn::{Ident, Token, token};

pub use cq_syntax::{CommaList, symbol_name};
pub mod kw {
    pub use cq_syntax::algebra::kw::*;
    syn::custom_keyword!(physical);
    syn::custom_keyword!(pipe);
    syn::custom_keyword!(canon);
    syn::custom_keyword!(probe);
    syn::custom_keyword!(merge);
    syn::custom_keyword!(semi);
    syn::custom_keyword!(range);
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Program {
    pub physical_token: kw::physical,
    #[syn(braced)]
    pub brace: token::Brace,
    #[syn(in = brace)]
    pub program: Body,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Body {
    #[parse(cq_syntax::parse_declarations)]
    #[to_tokens(|tokens, declarations| quote::TokenStreamExt::append_all(tokens, declarations))]
    pub declarations: Vec<cq_syntax::RelationDecl>,
    #[syn(repeat, until = Token![return])]
    pub definitions: Vec<Definition>,
    pub return_token: Token![return],
    pub returned: Ident,
    pub as_token: Token![as],
    pub head: cq_syntax::Atom,
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
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum Operator {
    #[parse(peek = kw::unit)]
    Unit(cq_syntax::algebra::Unit),
    #[parse(peek = kw::rename)]
    Rename(Box<cq_syntax::algebra::Rename>),
    #[parse(peek = kw::select)]
    Select(Box<cq_syntax::algebra::Select>),
    #[parse(peek = kw::extend)]
    Extend(Box<cq_syntax::algebra::Extend>),
    #[parse(peek = kw::project)]
    Project(Box<cq_syntax::algebra::Project>),
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

/// `[c] input if (condition)`: a bounded read on an ordered column.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Ranged {
    #[syn(bracketed)]
    pub bracket: token::Bracket,
    #[syn(in = bracket)]
    pub column: Ident,
    pub input: Ident,
    pub if_token: Token![if],
    #[syn(parenthesized)]
    pub condition_paren: token::Paren,
    #[syn(in = condition_paren)]
    pub condition: syn::Expr,
}

impl Operator {
    /// The result read as this operator's input stream, if any.
    pub fn input(&self) -> Option<&Ident> {
        match self {
            Self::Probe(_, k) | Self::Merge(_, k) | Self::Semi(_, k) => Some(&k.left),
            Self::Select(s) => Some(&s.input),
            Self::Extend(e) => Some(&e.input),
            Self::Project(p) => Some(&p.input),
            Self::Range(_, r) => Some(&r.input),
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
    pub rows: BTreeMap<String, Row>,
    /// Sorted heading of every result.
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
}

impl From<&Mode> for ModeKind {
    fn from(mode: &Mode) -> Self {
        match mode {
            Mode::Pipe(_) => ModeKind::Pipe,
            Mode::Let(_) => ModeKind::Let,
        }
    }
}

/// Column binders retain the source identifiers and their macro hygiene.
#[derive(Clone, Debug, Default)]
pub struct Row {
    pub bindings: BTreeMap<String, Ident>,
    pub computed: BTreeSet<String>,
    pub types: BTreeMap<String, syn::Type>,
}

impl Program {
    pub fn inputs(&self) -> impl Iterator<Item = &cq_syntax::RelationDecl> {
        self.program
            .declarations
            .iter()
            .filter(|d| symbol_name(&d.name) != symbol_name(&self.program.head.relation))
    }
}

pub mod contract {
    pub fn check(program: &super::Program) -> syn::Result<super::Analysis> {
        super::analyze(program)
    }
}

fn error<T>(spanned: &impl quote::ToTokens, message: impl std::fmt::Display) -> syn::Result<T> {
    Err(syn::Error::new_spanned(spanned, message))
}

/// Formal rule WF-Physical: Derive local headings and reads, then check output, reachability and modes.
/// Formal rule PH-Pipe: Require linear stream use except for reusable noncanonical host renames.
/// Formal rule PH-Let: Require every let to be read.
fn analyze(module: &Program) -> syn::Result<Analysis> {
    let program = &module.program;
    let declarations = cq_syntax::check_input_declarations(&program.declarations)?;
    if let Some(output) = declarations.get(&symbol_name(&program.head.relation)) {
        output.check_result()?;
    }
    let logical: BTreeSet<String> = program
        .definitions
        .iter()
        .flat_map(|d| match &d.operator {
            Operator::Rename(r) => r.mapping.iter().map(|m| symbol_name(&m.target)).collect(),
            Operator::Extend(e) => e.columns.iter().map(|c| symbol_name(&c.name)).collect(),
            _ => Vec::new(),
        })
        .collect();
    let mut analysis = Analysis::default();
    let mut readers: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut operands: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for definition in &program.definitions {
        let name = symbol_name(&definition.name);
        if analysis.rows.contains_key(&name) {
            return error(
                &definition.name,
                format!("result `{name}` is defined twice"),
            );
        }
        let of = |id: &Ident| -> syn::Result<Row> {
            analysis.rows.get(&symbol_name(id)).cloned().ok_or_else(|| {
                syn::Error::new_spanned(
                    id,
                    format!("operand `{id}` must be defined before `{name}`"),
                )
            })
        };
        let check_uses = |expr: &syn::Expr, row: &Row| -> syn::Result<()> {
            let heading = row.bindings.keys().cloned().collect();
            if let Some(missing) = cq_syntax::rust::known_uses(expr, &logical)
                .difference(&heading)
                .next()
            {
                return error(
                    expr,
                    format!("`{name}` uses unavailable attribute `{missing}`"),
                );
            }
            Ok(())
        };
        let mut deps = Vec::new();
        let row = match &definition.operator {
            Operator::Unit(_) => Row::default(),
            Operator::Rename(r) => {
                let host = symbol_name(&r.relation);
                let Some(declaration) = declarations.get(&host) else {
                    return error(r, format!("`{name}` renames undeclared relation `{host}`"));
                };
                let mut row = Row::default();
                let mut mapping = BTreeMap::new();
                let mut sources = BTreeSet::new();
                for m in &r.mapping {
                    let source = symbol_name(&m.source);
                    let target = symbol_name(&m.target);
                    let Some(column) = declaration
                        .columns
                        .iter()
                        .find(|c| symbol_name(&c.name) == source)
                    else {
                        return error(
                            m,
                            format!("rename `{name}`: `{host}` has no column `{source}`"),
                        );
                    };
                    if !sources.insert(source.clone())
                        || row
                            .bindings
                            .insert(target.clone(), m.target.clone())
                            .is_some()
                    {
                        return error(m, format!("rename `{name}` must be one-to-one"));
                    }
                    row.types.insert(target.clone(), column.ty.clone());
                    mapping.insert(target, source);
                }
                if sources.len() != declaration.columns.len() {
                    return error(
                        r,
                        format!("rename `{name}` must map every column of `{host}`"),
                    );
                }
                analysis.hosts.insert(name.clone(), host);
                analysis.renames.insert(name.clone(), mapping);
                row
            }
            Operator::Probe(_, k) | Operator::Merge(_, k) | Operator::Semi(_, k) => {
                let left = of(&k.left)?;
                let right = of(&k.right)?;
                let lh = left.bindings.keys().cloned().collect::<Vec<_>>();
                let rh = right.bindings.keys().cloned().collect::<Vec<_>>();
                let columns = join_columns(k, &lh, &rh).map_err(|e| {
                    syn::Error::new_spanned(&definition.name, format!("`{name}`: {e}"))
                })?;
                deps.extend([symbol_name(&k.left), symbol_name(&k.right)]);
                if matches!(definition.operator, Operator::Merge(..)) {
                    for side in [&k.left, &k.right] {
                        analysis
                            .merge_reads
                            .entry(symbol_name(side))
                            .or_default()
                            .push((name.clone(), columns.clone()));
                    }
                } else {
                    analysis
                        .key_reads
                        .entry(symbol_name(&k.right))
                        .or_default()
                        .push((name.clone(), columns));
                }
                if matches!(definition.operator, Operator::Semi(..)) {
                    left
                } else {
                    let mut row = right;
                    row.bindings.extend(left.bindings);
                    row.computed.extend(left.computed);
                    row.types.extend(left.types);
                    row
                }
            }
            Operator::Select(s) => {
                let row = of(&s.input)?;
                check_uses(&s.condition, &row)?;
                if cq_syntax::predicate::Predicate::from_expr(&s.condition).is_some() {
                    analysis.fragment.insert(name.clone());
                }
                deps.push(symbol_name(&s.input));
                row
            }
            Operator::Range(_, r) => {
                let row = of(&r.input)?;
                check_uses(&r.condition, &row)?;
                let column = symbol_name(&r.column);
                if !row.bindings.contains_key(&column) {
                    return error(
                        r,
                        format!("range `{name}`: `{}` has no column `{column}`", r.input),
                    );
                }
                if let Some(host) = analysis.hosts.get(&symbol_name(&r.input)) {
                    let leading = symbol_name(
                        &declarations[host]
                            .columns
                            .first()
                            .expect("positive arity")
                            .name,
                    );
                    if analysis.renames[&symbol_name(&r.input)][&column] != leading {
                        return error(r, "a host range must read its leading declared column");
                    }
                }
                analysis
                    .range_reads
                    .entry(symbol_name(&r.input))
                    .or_default()
                    .push((name.clone(), column));
                deps.push(symbol_name(&r.input));
                row
            }
            Operator::Extend(e) => {
                let mut row = of(&e.input)?;
                check_uses(&e.value, &row)?;
                for column in &e.columns {
                    let c = symbol_name(&column.name);
                    if row
                        .bindings
                        .insert(c.clone(), column.name.clone())
                        .is_some()
                    {
                        return error(e, format!("extend `{name}` repeats attribute `{c}`"));
                    }
                    row.computed.insert(c.clone());
                    row.types.insert(c, column.ty.clone());
                }
                deps.push(symbol_name(&e.input));
                row
            }
            Operator::Project(p) => {
                let mut row = of(&p.input)?;
                let mut kept = BTreeSet::new();
                for c in &p.attributes {
                    let column = symbol_name(c);
                    if !row.bindings.contains_key(&column) || !kept.insert(column.clone()) {
                        return error(
                            p,
                            format!("project `{name}` has invalid attribute `{column}`"),
                        );
                    }
                }
                row.bindings.retain(|c, _| kept.contains(c));
                row.types.retain(|c, _| kept.contains(c));
                row.computed.retain(|c| kept.contains(c));
                deps.push(symbol_name(&p.input));
                row
            }
        };
        analysis
            .headings
            .insert(name.clone(), row.bindings.keys().cloned().collect());
        analysis.rows.insert(name.clone(), row);
        operands.insert(name.clone(), deps.clone());
        for input in deps {
            readers.entry(input).or_default().push(name.clone());
        }
        if let Some(input) = definition.operator.input() {
            analysis.scanned.insert(symbol_name(input));
        }
        if let Operator::Merge(_, k) = &definition.operator {
            analysis.scanned.remove(&symbol_name(&k.left));
        }
        let mode = ModeKind::from(&definition.mode);
        analysis.modes.insert(name.clone(), mode);
        if definition.canon.is_some() {
            analysis.canon.insert(name);
        }
    }
    let returned = symbol_name(&program.returned);
    let Some(row) = analysis.rows.get(&returned) else {
        return error(
            &program.returned,
            format!("return `{returned}` names no result"),
        );
    };
    let heading: BTreeSet<_> = row.bindings.keys().cloned().collect();
    for expr in &program.head.args {
        if let Some(c) = cq_syntax::rust::known_uses(expr, &logical)
            .difference(&heading)
            .next()
        {
            return error(
                expr,
                format!("return `{returned}` uses unavailable attribute `{c}`"),
            );
        }
    }
    let output: BTreeSet<_> = program
        .head
        .args
        .iter()
        .flat_map(|e| cq_syntax::rust::free_variables(e).dependencies(&heading))
        .collect();
    if program.head.variables().count() == program.head.args.len() && output != heading {
        return error(
            &program.head,
            format!("return `{returned}` head must match its heading"),
        );
    }
    let mut reachable = BTreeSet::new();
    let mut pending = vec![returned.clone()];
    while let Some(name) = pending.pop() {
        if reachable.insert(name.clone()) {
            pending.extend(operands[&name].iter().cloned());
        }
    }
    if let Some(unused) = program
        .definitions
        .iter()
        .find(|d| !reachable.contains(&symbol_name(&d.name)))
    {
        return error(
            &unused.name,
            format!(
                "result `{}` does not contribute to return `{returned}`",
                unused.name
            ),
        );
    }
    readers
        .entry(returned.clone())
        .or_default()
        .push("return".into());
    analysis.scanned.insert(returned);
    for definition in &program.definitions {
        let name = symbol_name(&definition.name);
        let mode = analysis.modes[&name];
        let reads = readers.get(&name).map_or(0, Vec::len);
        match mode {
            ModeKind::Pipe
                if !analysis.hosts.contains_key(&name) || analysis.canon.contains(&name) =>
            {
                if reads != 1 {
                    return error(
                        &definition.name,
                        format!("pipe `{name}` must be read exactly once; read {reads} times"),
                    );
                }
                if analysis.key_reads.contains_key(&name)
                    || analysis.range_reads.contains_key(&name)
                {
                    return error(
                        &definition.name,
                        format!("`{name}` is read through a key and must be let"),
                    );
                }
            }
            ModeKind::Let if reads == 0 => {
                return error(&definition.name, format!("`{name}` is never read"));
            }
            _ => (),
        }
    }
    Ok(analysis)
}

fn join_columns(keyed: &Keyed, left: &[String], right: &[String]) -> syn::Result<Vec<String>> {
    let shared: BTreeSet<String> = left.iter().filter(|c| right.contains(c)).cloned().collect();
    let written: BTreeSet<String> = keyed.columns.iter().map(symbol_name).collect();
    if written != shared || written.len() != keyed.columns.len() {
        return Err(syn::Error::new(
            keyed.bracket.span.join(),
            format!("join columns are {shared:?}, written {written:?}"),
        ));
    }
    Ok(shared.into_iter().collect())
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
        || [&a.right, &b.right].iter().any(|r| {
            analysis.modes[&symbol_name(r)] != ModeKind::Let
                && !analysis.hosts.contains_key(&symbol_name(r))
        })
    {
        return None;
    }
    Some((a, b, v.clone()))
}

pub mod policy;

/// Shared syntax used by this language.
pub use cq_syntax as syntax;
