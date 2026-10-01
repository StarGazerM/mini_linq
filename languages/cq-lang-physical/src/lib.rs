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
//! Operator ::= Unit | Rename | Select | Point | Bind | Guard | Project
//!            | "probe" "[" Ident,* "]" Ident "," Ident
//!            | "merge" "[" Ident,* "]" Ident "," Ident
//!            | "semi" "[" Ident,* "]" Ident "," Ident
//! ```
//!
//! Unit, Rename, Select, Bind, Guard and Project are shared syntax.
//! Meaning is Kleisli over bottom + Set: bottom aborts the whole evaluation.
//! Bind/guard expressions are opaque partial functions. Their ancestry chain
//! preserves written order; only relational steps may move earlier across it.
//! Other effects and evaluation counts are outside this meaning.

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
    syn::custom_keyword!(point);
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
    #[parse(peek = kw::point)]
    Point(kw::point, Box<cq_syntax::algebra::Select>),
    #[parse(peek = kw::bind)]
    Bind(Box<cq_syntax::algebra::Bind>),
    #[parse(peek = kw::guard)]
    Guard(Box<cq_syntax::algebra::Guard>),
    #[parse(peek = kw::project)]
    Project(Box<cq_syntax::algebra::Project>),
    #[parse(peek = kw::probe)]
    Probe(kw::probe, Keyed),
    #[parse(peek = kw::merge)]
    Merge(kw::merge, Keyed),
    #[parse(peek = kw::semi)]
    Semi(kw::semi, Keyed),
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

impl Operator {
    /// The result read as this operator's input stream, if any.
    pub fn input(&self) -> Option<&Ident> {
        match self {
            Self::Probe(_, k) | Self::Merge(_, k) | Self::Semi(_, k) => Some(&k.left),
            Self::Select(s) => Some(&s.input),
            Self::Point(_, s) => Some(&s.input),
            Self::Bind(b) => Some(&b.input),
            Self::Guard(g) => Some(&g.input),
            Self::Project(p) => Some(&p.input),
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
    /// Every keyed read records where its key values come from.
    pub keyed_reads: BTreeMap<String, Vec<(String, Vec<String>, cq_syntax::SupplyKind)>>,
    /// One compile-time key per point reader; these reads never need a build.
    pub point_reads: BTreeMap<String, Vec<(String, Vec<String>)>>,
    /// Results whose rows are enumerated (pipe input, scan of a let).
    pub scanned: BTreeSet<String>,
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
            Operator::Bind(b) => cq_syntax::rust::bindings(&b.pattern)
                .into_iter()
                .map(|i| symbol_name(&i))
                .collect(),
            _ => Vec::new(),
        })
        .collect();
    let mut analysis = Analysis::default();
    let mut readers: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut operands: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut chains: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut written_chain = Vec::new();
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
                let sides = if matches!(definition.operator, Operator::Merge(..)) {
                    vec![&k.left, &k.right]
                } else {
                    vec![&k.right]
                };
                for side in sides {
                    analysis
                        .keyed_reads
                        .entry(symbol_name(side))
                        .or_default()
                        .push((name.clone(), columns.clone(), cq_syntax::SupplyKind::Stream));
                }
                for c in &columns {
                    check_setoid(&left.types[c], c, &definition.name)?;
                    check_setoid(&right.types[c], c, &definition.name)?;
                }
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
            Operator::Select(s) | Operator::Point(_, s) => {
                let row = of(&s.input)?;
                for c in s.condition.columns() {
                    let Some(ty) = row.types.get(&c) else {
                        return error(s, format!("select `{name}` uses unavailable column `{c}`"));
                    };
                    check_setoid(ty, &c, &definition.name)?;
                }
                if matches!(definition.operator, Operator::Point(..)) {
                    let Some(keys) = s.condition.constant_keys() else {
                        return error(
                            s,
                            "point read requires a nonempty conjunction of column = literal equalities",
                        );
                    };
                    let columns: Vec<_> = keys.into_keys().collect();
                    analysis
                        .point_reads
                        .entry(symbol_name(&s.input))
                        .or_default()
                        .push((name.clone(), columns.clone()));
                    analysis
                        .keyed_reads
                        .entry(symbol_name(&s.input))
                        .or_default()
                        .push((name.clone(), columns, cq_syntax::SupplyKind::Const));
                }
                deps.push(symbol_name(&s.input));
                row
            }
            Operator::Guard(g) => {
                let row = of(&g.input)?;
                check_uses(&g.condition, &row)?;
                deps.push(symbol_name(&g.input));
                row
            }
            Operator::Bind(b) => {
                let mut row = of(&b.input)?;
                check_uses(&b.expression, &row)?;
                for (ident, ty) in b.binding_types() {
                    let c = symbol_name(&ident);
                    if row.bindings.insert(c.clone(), ident).is_some() {
                        return error(b, format!("bind `{name}` repeats attribute `{c}`"));
                    }
                    row.computed.insert(c.clone());
                    row.types.insert(c, ty);
                }
                deps.push(symbol_name(&b.input));
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
        let mut chain = Vec::new();
        for dep in &deps {
            let input_chain = &chains[dep];
            if !chain.starts_with(input_chain) && !input_chain.starts_with(&chain) {
                return error(
                    &definition.name,
                    "Rust bind/guard nodes must form one ancestry chain",
                );
            }
            if input_chain.len() > chain.len() {
                chain = input_chain.clone();
            }
        }
        if matches!(definition.operator, Operator::Bind(_) | Operator::Guard(_)) {
            if chain != written_chain {
                return error(
                    &definition.name,
                    "Rust bind/guard ancestry must preserve written order",
                );
            }
            chain.push(name.clone());
            written_chain.push(name.clone());
        }
        chains.insert(name.clone(), chain);
        if definition.canon.is_some() {
            for (c, ty) in &row.types {
                check_setoid(ty, c, &definition.name)?;
            }
        }
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
                if analysis.key_reads.contains_key(&name) {
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

/// Reject syntactically known non-setoid columns; rustc checks aliases.
fn check_setoid(ty: &syn::Type, column: &str, at: &impl quote::ToTokens) -> syn::Result<()> {
    let column = syn::Ident::new(column, proc_macro2::Span::call_site());
    cq_syntax::algebra::check_setoid_type(ty, &column)
        .map_err(|e| syn::Error::new_spanned(at, e.to_string()))
}
