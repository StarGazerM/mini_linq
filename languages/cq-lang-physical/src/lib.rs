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
//! Operator ::= Unit | Rename | Select | Point | Range | Bind | Guard | Project
//!            | "probe" "[" Ident,* "]" Ident "," Ident
//!            | "merge" "[" Ident,* "]" Ident "," Ident
//!            | "semi" "[" Ident,* "]" Ident "," Ident
//! ```
//!
//! Point/Range ::= ("point" | "range") Select
//! Unit, Rename, Select, Bind, Guard and Project are shared syntax.
//! Meaning is Kleisli over bottom + Set: bottom aborts the whole evaluation.
//! Bind/guard expressions are opaque partial functions. Their ancestry chain
//! preserves written order; only relational steps may move earlier across it.
//! Other effects and evaluation counts are outside this meaning.
//!
//! Relational equality is Leibniz identity, decided by admissible column `==`.
//! Functional extensionality of shared-reference Rust inputs permits canonical
//! projection whenever free variables remain; see the premises in
//! `cq-syntax`.

//! Physical records union canon/plain, test witnesses any/all, fused/retained prefix use, and anti eager/lazy.
//! Defaults preserve written arm order, hoist Rust-free arms, stop test union at a witness, canon binding union, and fuse per-row Rust arms.
//! Plain union defers dedup to a recorded final boundary.
//!
//! Core blocks denote `bottom + Set`: `{ B1 } | { B2 }` unions projected
//! answers; `{ B1 } else { B2 }` selects the first nonempty arm. F is the free
//! heading and I the common new interface; other binders remain branch-local.
//! Join distributes over union; fallback is `B1 union (B2 anti_F B1)`.
//! Rust keeps a branch ancestry tree: union siblings unordered, fallback ordered;
//! each arm retains prefix Before facts and fallback anti steps. Rust `||` keeps
//! its existing recognition. Only differing literals on one interface column
//! prove union arms disjoint; no data keys or constraints are assumed.
//!
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
    syn::custom_keyword!(range);
    syn::custom_keyword!(exists);
    syn::custom_keyword!(first);
    syn::custom_keyword!(smallest);
    syn::custom_keyword!(eager);
    syn::custom_keyword!(lazy);
    syn::custom_keyword!(any);
    syn::custom_keyword!(all);
    syn::custom_keyword!(fused);
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
    pub return_canon: Option<kw::canon>,
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
    #[parse(peek = kw::exists)]
    Exists(kw::exists),
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum Operator {
    #[parse(peek = kw::union)]
    Union(Box<Union>),
    #[parse(peek = kw::anti)]
    Anti(kw::anti, Keyed),
    #[parse(peek = kw::unit)]
    Unit(cq_syntax::algebra::Unit),
    #[parse(peek = kw::rename)]
    Rename(Box<cq_syntax::algebra::Rename>),
    #[parse(peek = kw::select)]
    Select(Box<cq_syntax::algebra::Select>),
    #[parse(peek = kw::point)]
    Point(kw::point, Box<cq_syntax::algebra::Select>),
    #[parse(peek = kw::range)]
    Range(kw::range, Box<cq_syntax::algebra::Select>),
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

/// Physical union records witness execution and shared-prefix fusion. The
/// definition's canon flag records its sole cross-branch dedup boundary.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Union {
    pub union_token: kw::union,
    #[parse(parse_witness)]
    pub witness: Option<Witness>,
    pub fused: Option<kw::fused>,
    #[parse(CommaList::parse_separated_nonempty)]
    pub operands: CommaList<Ident>,
    #[parse(parse_origin)]
    pub origin: Option<cq_syntax::algebra::BlockOrigin>,
}
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum Witness {
    #[parse(peek = kw::any)]
    Any(kw::any),
    #[parse(peek = kw::all)]
    All(kw::all),
}
fn parse_witness(input: syn::parse::ParseStream<'_>) -> syn::Result<Option<Witness>> {
    if input.peek(kw::any) || input.peek(kw::all) {
        input.parse().map(Some)
    } else {
        Ok(None)
    }
}
fn parse_origin(
    input: syn::parse::ParseStream<'_>,
) -> syn::Result<Option<cq_syntax::algebra::BlockOrigin>> {
    if input.peek(kw::block) {
        input.parse().map(Some)
    } else {
        Ok(None)
    }
}
impl Union {
    pub fn logical(&self) -> cq_syntax::algebra::Union {
        cq_syntax::algebra::Union {
            union_token: self.union_token,
            operands: self.operands.clone(),
            origin: self.origin.clone(),
        }
    }
}

/// `[a, b] left, right`: the join columns by name, then both operands.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Keyed {
    #[parse(parse_choice)]
    pub choice: Option<Choice>,
    #[syn(bracketed)]
    pub bracket: token::Bracket,
    #[syn(in = bracket)]
    #[parse(CommaList::parse_terminated)]
    pub columns: CommaList<Ident>,
    pub left: Ident,
    pub comma: Token![,],
    pub right: Ident,
}

/// Per-operator execution choice; never a query-wide strategy.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum Choice {
    #[parse(peek = kw::first)]
    First(kw::first),
    #[parse(peek = kw::smallest)]
    Smallest(kw::smallest),
    #[parse(peek = kw::eager)]
    Eager(kw::eager),
    #[parse(peek = kw::lazy)]
    Lazy(kw::lazy),
}

fn parse_choice(input: syn::parse::ParseStream<'_>) -> syn::Result<Option<Choice>> {
    if input.peek(kw::first)
        || input.peek(kw::smallest)
        || input.peek(kw::eager)
        || input.peek(kw::lazy)
    {
        input.parse().map(Some)
    } else {
        Ok(None)
    }
}

impl Operator {
    /// Every named operand, including union arms and negative references.
    pub fn operands(&self) -> Vec<&Ident> {
        match self {
            Self::Union(union) => union.operands.iter().collect(),
            Self::Probe(_, k) | Self::Merge(_, k) | Self::Semi(_, k) | Self::Anti(_, k) => {
                vec![&k.left, &k.right]
            }
            _ => self.input().into_iter().collect(),
        }
    }
    /// The result read as this operator's input stream, if any.
    pub fn input(&self) -> Option<&Ident> {
        match self {
            Self::Probe(_, k) | Self::Merge(_, k) | Self::Semi(_, k) | Self::Anti(_, k) => {
                Some(&k.left)
            }
            Self::Select(s) => Some(&s.input),
            Self::Point(_, s) | Self::Range(_, s) => Some(&s.input),
            Self::Bind(b) => Some(&b.input),
            Self::Guard(g) => Some(&g.input),
            Self::Project(p) => Some(&p.input),
            _ => None,
        }
    }

    /// The result read through a key, with the key columns, if any.
    pub fn keyed(&self) -> Option<(&Ident, Vec<String>)> {
        match self {
            Operator::Probe(_, k) | Operator::Semi(_, k) | Operator::Anti(_, k) => {
                Some((&k.right, k.columns.iter().map(symbol_name).collect()))
            }
            _ => None,
        }
    }
}

/// How a result's grades and reads are known once the contract holds.
#[derive(Clone, Debug, Default)]
pub struct Analysis {
    /// Union arms are disjoint only by fallback construction or different
    /// explicit literals on the same interface column.
    pub disjoint: BTreeSet<String>,
    /// Fused union -> prefix and arm-local definitions. Shared pipe reads in
    /// these groups are one streamed traversal, never independent traversals.
    pub fused: BTreeMap<String, Fused>,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fused {
    pub prefix: String,
    pub members: BTreeSet<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeKind {
    Pipe,
    Let,
    Exists,
}

impl From<&Mode> for ModeKind {
    fn from(mode: &Mode) -> Self {
        match mode {
            Mode::Pipe(_) => ModeKind::Pipe,
            Mode::Let(_) => ModeKind::Let,
            Mode::Exists(_) => ModeKind::Exists,
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
            Operator::Union(union) => {
                if union.operands.len() < 2 {
                    return error(union, "union requires at least two operands");
                }
                let mut row = of(&union.operands[0])?;
                let mut declared_types = BTreeMap::new();
                for operand in &union.operands {
                    let other = of(operand)?;
                    if row.bindings.keys().collect::<Vec<_>>()
                        != other.bindings.keys().collect::<Vec<_>>()
                    {
                        return error(operand, "union operand headings must be identical");
                    }
                    for (column, ty) in &other.types {
                        if !other.computed.contains(column) {
                            if declared_types
                                .get(column)
                                .is_some_and(|previous| previous != ty)
                            {
                                return error(
                                    operand,
                                    format!(
                                        "union interface `{column}` has different declared types"
                                    ),
                                );
                            }
                            declared_types.insert(column.clone(), ty.clone());
                        }
                    }
                    row.computed.extend(other.computed);
                    deps.push(symbol_name(operand));
                    analysis.scanned.insert(symbol_name(operand));
                }
                if let Some(origin) = &union.origin {
                    origin.occurrence.check()?;
                    let mut expected: BTreeSet<_> = origin
                        .free
                        .iter()
                        .chain(&origin.interface)
                        .map(symbol_name)
                        .collect();
                    if let Some(over) = &origin.over {
                        expected.extend(of(&over.prefix)?.bindings.into_keys());
                    }
                    if expected != row.bindings.keys().cloned().collect() {
                        return error(
                            origin,
                            "block union must have exactly its context and interface heading",
                        );
                    }
                    if origin.interface.is_empty() != union.witness.is_some() {
                        return error(
                            union,
                            "test blocks record any/all; binding blocks have no witness choice",
                        );
                    }
                } else if union.witness.is_some() && !row.bindings.is_empty() {
                    return error(
                        union,
                        "a witness union needs a block's test interface or an empty heading",
                    );
                }
                if union.fused.is_some()
                    && union
                        .origin
                        .as_ref()
                        .and_then(|o| o.over.as_ref())
                        .is_none()
                {
                    return error(union, "fused union requires a recorded prefix");
                }
                row
            }

            Operator::Unit(_) => Row::default(),
            Operator::Rename(r) => {
                r.occurrence.check()?;
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
            Operator::Probe(_, k)
            | Operator::Merge(_, k)
            | Operator::Semi(_, k)
            | Operator::Anti(_, k) => {
                let valid = match &definition.operator {
                    Operator::Probe(..) => {
                        matches!(k.choice, Some(Choice::First(_) | Choice::Smallest(_)))
                    }
                    Operator::Semi(..) | Operator::Anti(..) => {
                        matches!(k.choice, Some(Choice::Eager(_) | Choice::Lazy(_)))
                    }
                    Operator::Merge(..) => k.choice.is_none() && !k.columns.is_empty(),
                    _ => unreachable!(),
                };
                if !valid {
                    return error(
                        k,
                        "probe requires first/smallest; semi requires eager/lazy; merge requires a nonempty ordered key",
                    );
                }
                let left = of(&k.left)?;
                let right = of(&k.right)?;
                let lh = left.bindings.keys().cloned().collect::<Vec<_>>();
                let rh = right.bindings.keys().cloned().collect::<Vec<_>>();
                let columns = if matches!(definition.operator, Operator::Anti(..)) {
                    let key: BTreeSet<_> = k.columns.iter().map(symbol_name).collect();
                    if key.len() != k.columns.len()
                        || !key.iter().all(|c| lh.contains(c) && rh.contains(c))
                    {
                        return error(k, "anti key must list distinct columns in both operands");
                    }
                    key.into_iter().collect()
                } else {
                    join_columns(k, &lh, &rh)?
                };
                let columns = Ok::<_, syn::Error>(columns).map_err(|e| {
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
                    check_leibniz(&left.types[c], c, &definition.name)?;
                    check_leibniz(&right.types[c], c, &definition.name)?;
                    if matches!(definition.operator, Operator::Merge(..)) {
                        let ident = syn::Ident::new(c, proc_macro2::Span::call_site());
                        cq_syntax::algebra::check_ord_type(&left.types[c], &ident)?;
                        cq_syntax::algebra::check_ord_type(&right.types[c], &ident)?;
                    }
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
                if matches!(definition.operator, Operator::Semi(..) | Operator::Anti(..)) {
                    left
                } else {
                    let mut row = right;
                    row.bindings.extend(left.bindings);
                    row.computed.extend(left.computed);
                    row.types.extend(left.types);
                    row
                }
            }
            Operator::Select(s) | Operator::Point(_, s) | Operator::Range(_, s) => {
                s.condition.check()?;
                let row = of(&s.input)?;
                for c in s.condition.columns() {
                    let Some(ty) = row.types.get(&c) else {
                        return error(s, format!("select `{name}` uses unavailable column `{c}`"));
                    };
                    check_leibniz(ty, &c, &definition.name)?;
                }
                for c in &s.condition.constraints {
                    for p in c.parameters() {
                        if row.types.contains_key(&symbol_name(p)) {
                            return error(p, "relational parameter names an input column");
                        }
                    }
                    for v in c.ordered_variables() {
                        cq_syntax::algebra::check_ord_type(&row.types[&symbol_name(v)], v)?;
                    }
                }
                if matches!(definition.operator, Operator::Point(..)) {
                    let Some(keys) = s.condition.point_columns() else {
                        return error(
                            s,
                            "point read requires a nonempty conjunction of column = constant or literal membership constraints",
                        );
                    };
                    let columns: Vec<_> = keys.into_iter().collect();
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
                // Formal rule PH-Range: A window demand is constant order bounds on exactly one column.
                if matches!(definition.operator, Operator::Range(..)) {
                    let Some((column, _)) = s.condition.range_bounds() else {
                        return error(s, "range read requires constant order bounds on one column");
                    };
                    analysis
                        .keyed_reads
                        .entry(symbol_name(&s.input))
                        .or_default()
                        .push((name.clone(), vec![column], cq_syntax::SupplyKind::Range));
                }
                deps.push(symbol_name(&s.input));
                row
            }
            Operator::Guard(g) => {
                let row = of(&g.input)?;
                if g.environment.is_none() {
                    check_uses(&g.condition, &row)?;
                }
                if let Some(environment) = &g.environment {
                    environment.check(&row.bindings.keys().cloned().collect(), &g.condition)?;
                }
                deps.push(symbol_name(&g.input));
                row
            }
            Operator::Bind(b) => {
                let mut row = of(&b.input)?;
                if b.environment.is_none() {
                    check_uses(&b.expression, &row)?;
                }
                if let Some(environment) = &b.environment {
                    environment.check(&row.bindings.keys().cloned().collect(), &b.expression)?;
                }
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
                if matches!(definition.operator, Operator::Union(_)) {
                    for name in input_chain {
                        if !chain.contains(name) {
                            chain.push(name.clone());
                        }
                    }
                    continue;
                }
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
            if chain != written_chain
                && !program
                    .definitions
                    .iter()
                    .any(|d| matches!(d.operator, Operator::Union(_)))
            {
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
                check_leibniz(ty, c, &definition.name)?;
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
        // Ordered merge reads must not erase a scan required by another reader.
        if !matches!(definition.operator, Operator::Merge(..)) {
            if let Some(input) = definition.operator.input() {
                analysis.scanned.insert(symbol_name(input));
            }
        }
        let mode = ModeKind::from(&definition.mode);
        if mode == ModeKind::Exists && !analysis.headings[&name].is_empty() {
            return error(&definition.name, "exists mode requires an empty heading");
        }
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
        let Operator::Union(union) = &definition.operator else {
            continue;
        };
        if union.fused.is_none() {
            continue;
        }
        let prefix = symbol_name(
            &union
                .origin
                .as_ref()
                .expect("origin checked")
                .over
                .as_ref()
                .expect("prefix checked")
                .prefix,
        );
        let mut dependent = BTreeSet::from([prefix.clone()]);
        let mut members = BTreeSet::new();
        for candidate in &program.definitions {
            let name = symbol_name(&candidate.name);
            if name == symbol_name(&definition.name) {
                break;
            }
            if name != prefix
                && operands[&name]
                    .iter()
                    .any(|input| dependent.contains(input))
            {
                dependent.insert(name.clone());
                members.insert(name);
            }
        }
        if !union
            .operands
            .iter()
            .all(|id| members.contains(&symbol_name(id)))
        {
            return error(union, "every fused arm must follow its recorded prefix");
        }
        analysis
            .fused
            .insert(symbol_name(&definition.name), Fused { prefix, members });
    }
    for definition in &program.definitions {
        let name = symbol_name(&definition.name);
        let mode = analysis.modes[&name];
        let reads = readers.get(&name).map_or(0, Vec::len);
        match mode {
            ModeKind::Pipe
                if !analysis.hosts.contains_key(&name) || analysis.canon.contains(&name) =>
            {
                let shared = analysis.fused.values().any(|group| {
                    group.prefix == name
                        && readers.get(&name).is_some_and(|uses| {
                            uses.iter().all(|reader| group.members.contains(reader))
                        })
                });
                if reads != 1 && !shared {
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
            ModeKind::Let | ModeKind::Exists if reads == 0 => {
                return error(&definition.name, format!("`{name}` is never read"));
            }
            _ => (),
        }
    }
    for (position, definition) in program.definitions.iter().enumerate() {
        if matches!(&definition.operator, Operator::Probe(_, k) if matches!(k.choice, Some(Choice::Smallest(_))))
            && program
                .definitions
                .get(position + 1)
                .and_then(|next| binary_intersection(definition, next, &analysis))
                .is_none()
        {
            return error(
                &definition.name,
                "smallest requires an admissible adjacent binary intersection",
            );
        }
        if let Operator::Semi(_, k) | Operator::Anti(_, k) = &definition.operator {
            if matches!(k.choice, Some(Choice::Lazy(_))) {
                let right = symbol_name(&k.right);
                if !analysis.hosts.contains_key(&right) || analysis.modes[&right] != ModeKind::Pipe
                {
                    return error(
                        &definition.name,
                        "lazy semi requires a reusable host rename",
                    );
                }
                if program
                    .definitions
                    .iter()
                    .any(|reader| match &reader.operator {
                        Operator::Probe(_, other) => {
                            other.right == k.right && other.columns == k.columns
                        }
                        Operator::Semi(_, other) | Operator::Anti(_, other) => {
                            other.right == k.right
                                && other.columns == k.columns
                                && !matches!(other.choice, Some(Choice::Lazy(_)))
                        }
                        _ => false,
                    })
                {
                    return error(
                        &definition.name,
                        "lazy semi requires exclusively lazy membership reads on its shared key",
                    );
                }
            }
        }
    }
    analysis.disjoint = constants::disjoint(module);
    for definition in &program.definitions {
        if definition.canon.is_some()
            && matches!(definition.operator, Operator::Union(_))
            && analysis.disjoint.contains(&symbol_name(&definition.name))
        {
            return error(
                &definition.name,
                "disjoint union arms defer canonicalization to their consumers; cross-branch dedup is unnecessary",
            );
        }
    }
    Ok(analysis)
}
mod constants;

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

/// Shared syntax used by this language.
pub use cq_syntax as syntax;

/// Reject known failures of Leibniz admissibility; rustc checks necessary bounds.
fn check_leibniz(ty: &syn::Type, column: &str, at: &impl quote::ToTokens) -> syn::Result<()> {
    let column = syn::Ident::new(column, proc_macro2::Span::call_site());
    cq_syntax::algebra::check_leibniz_type(ty, &column)
        .map_err(|e| syn::Error::new_spanned(at, e.to_string()))
}

/// Physical rewrites preserve the named logical graph and satisfy their read contracts.
/// Operator choices, retention modes and canonicalization may change while
/// declarations, operands, relational predicates and Rust payloads remain fixed.
/// Relational definitions may commute in dependency order and move earlier
/// across Rust nodes. The Rust chain retains its written relative order, and
/// every source Before obligation survives: no relational definition moves
/// later across a Rust node.
pub fn check_rewrite(source: &Program, target: &Program) -> syn::Result<()> {
    contract::check(source)?;
    contract::check(target)?;
    let mut target_chain = Vec::new();
    let target_definitions: BTreeMap<_, _> = target
        .program
        .definitions
        .iter()
        .map(|definition| {
            let name = symbol_name(&definition.name);
            let rust_before = target_chain.len();
            if is_rust(&definition.operator) {
                target_chain.push(name.clone());
            }
            (name, (definition, rust_before))
        })
        .collect();
    if source.program.declarations != target.program.declarations
        || source.program.head != target.program.head
        || source.program.returned != target.program.returned
        || source.program.definitions.len() != target.program.definitions.len()
        || source.program.definitions.iter().any(|definition| {
            target_definitions
                .get(&symbol_name(&definition.name))
                .is_none_or(|(target, _)| {
                    !same_logical_operator(&definition.operator, &target.operator)
                })
        })
    {
        return error(
            &target.program.returned,
            "Physical rewrite changes RA erasure",
        );
    }

    if source
        .program
        .definitions
        .iter()
        .any(|d| matches!(d.operator, Operator::Union(_)))
    {
        let scopes = branch_scopes(source);
        let positions: BTreeMap<_, _> = target
            .program
            .definitions
            .iter()
            .enumerate()
            .map(|(i, d)| (symbol_name(&d.name), i))
            .collect();
        for (i, rust) in source
            .program
            .definitions
            .iter()
            .enumerate()
            .filter(|(_, d)| is_rust(&d.operator))
        {
            let name = symbol_name(&rust.name);
            let scope = &scopes[&name];
            for before in &source.program.definitions[..i] {
                let previous = symbol_name(&before.name);
                // A sibling union arm has its own chain. Ancestor and local
                // definitions retain every written Before obligation.
                if scope.starts_with(&scopes[&previous]) && positions[&previous] > positions[&name]
                {
                    return error(
                        &rust.name,
                        format!(
                            "Physical rewrite loses Before result `{previous}` at Rust `{name}`"
                        ),
                    );
                }
            }
        }
        return Ok(());
    }

    // The contracts give unique names, topological operand edges and Rust
    // ancestry. With the same graph and Rust chain, a relational node retains
    // all source Before obligations exactly when its preceding Rust prefix
    // does not grow. Reordering within a relational region is unrestricted.
    let mut source_rust_before = 0;
    for definition in &source.program.definitions {
        let name = symbol_name(&definition.name);
        let (target_definition, target_rust_before) = target_definitions[&name];
        if is_rust(&definition.operator) {
            if target_chain.get(source_rust_before) != Some(&name) {
                return error(
                    &target_definition.name,
                    "Physical rewrite changes Rust bind/guard written order",
                );
            }
            source_rust_before += 1;
        } else if target_rust_before > source_rust_before {
            return error(
                &target_definition.name,
                format!(
                    "Physical rewrite moves relational result `{name}` later across Rust node `{}` and loses a source Before obligation",
                    target_chain[source_rust_before]
                ),
            );
        }
    }
    Ok(())
}

// Given union operands assign branch scopes by graph edges, cutting the
// recorded prefix and every negative reference. This is structural traversal
// of named nodes; it performs no occurrence matching or search.
fn branch_scopes(program: &Program) -> BTreeMap<String, Vec<String>> {
    let definitions: BTreeMap<_, _> = program
        .program
        .definitions
        .iter()
        .map(|d| (symbol_name(&d.name), d))
        .collect();
    let mut scopes: BTreeMap<_, Vec<String>> = definitions
        .keys()
        .map(|name| (name.clone(), Vec::new()))
        .collect();
    for definition in &program.program.definitions {
        let Operator::Union(union) = &definition.operator else {
            continue;
        };
        let Some(origin) = &union.origin else {
            continue;
        };
        let boundary = origin
            .over
            .as_ref()
            .map(|o| symbol_name(&o.prefix))
            .or_else(|| origin.domain.as_ref().map(|d| symbol_name(&d.input)));
        for (j, operand) in union.operands.iter().enumerate() {
            let mut scope = origin.occurrence.0.clone();
            scope.push(j.to_string());
            let mut visited = BTreeSet::new();
            let mut pending = vec![symbol_name(operand)];
            while let Some(name) = pending.pop() {
                if boundary.as_ref() == Some(&name) || !visited.insert(name.clone()) {
                    continue;
                }
                if scope.len() > scopes[&name].len() {
                    scopes.insert(name.clone(), scope.clone());
                }
                let operator = &definitions[&name].operator;
                match operator {
                    Operator::Anti(_, anti) => pending.push(symbol_name(&anti.left)),
                    _ => pending.extend(operator.operands().into_iter().map(symbol_name)),
                }
            }
        }
    }
    scopes
}

fn is_rust(operator: &Operator) -> bool {
    matches!(operator, Operator::Bind(_) | Operator::Guard(_))
}

// Compare the logical content directly, without importing another language.
// Probe/merge implement the same join; point/range retain the same selection.
fn same_logical_operator(source: &Operator, target: &Operator) -> bool {
    match (source, target) {
        (Operator::Union(s), Operator::Union(t)) => {
            let mut source = s.logical();
            let mut target = t.logical();
            if source.origin.as_ref().is_none_or(|o| o.fallback.is_none())
                && target.origin.as_ref().is_none_or(|o| o.fallback.is_none())
            {
                let mut a: Vec<_> = source.operands.into_iter().collect();
                let mut b: Vec<_> = target.operands.into_iter().collect();
                a.sort_by_key(symbol_name);
                b.sort_by_key(symbol_name);
                source.operands = a.into_iter().collect();
                target.operands = b.into_iter().collect();
            }
            source == target
        }
        (Operator::Unit(s), Operator::Unit(t)) => s == t,
        (Operator::Rename(s), Operator::Rename(t)) => s == t,
        (
            Operator::Select(s) | Operator::Point(_, s) | Operator::Range(_, s),
            Operator::Select(t) | Operator::Point(_, t) | Operator::Range(_, t),
        ) => s == t,
        (Operator::Bind(s), Operator::Bind(t)) => s == t,
        (Operator::Guard(s), Operator::Guard(t)) => s == t,
        (Operator::Project(s), Operator::Project(t)) => s == t,
        (
            Operator::Probe(_, s) | Operator::Merge(_, s),
            Operator::Probe(_, t) | Operator::Merge(_, t),
        )
        | (Operator::Semi(_, s), Operator::Semi(_, t))
        | (Operator::Anti(_, s), Operator::Anti(_, t)) => {
            s.left == t.left && s.right == t.right && s.columns == t.columns
        }
        _ => false,
    }
}

impl cq_stage::Language for Program {
    fn check(&self) -> syn::Result<()> {
        contract::check(self).map(|_| ())
    }
    fn check_rewrite(source: &Self, target: &Self) -> syn::Result<()> {
        check_rewrite(source, target)
    }
}
