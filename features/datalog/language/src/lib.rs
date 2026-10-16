//! Finite, insert-only Boolean Datalog above CQ.
//!
//! Declare EDBs with host bindings, IDBs without bindings, and `output R;`.
//! `output R, S;` returns owned iterators in the written output order.
//! Schedule passes record dependency-ordered SCCs and one semi-naive variant
//! per changing positive position. All stages parse, print and check alone.
//! Recursive heads copy positive columns (`*x` or `(*x).clone()`). Opaque Rust
//! clauses retain their written order. Native `!R(x,_)` negation reads completed
//! lower strata and requires every named argument to be bound before the clause.
//! Negative constants (including closed Rust expressions) filter a projected
//! borrowed key set once per stratum; they cannot depend on rule-local bindings.
//! Extensions, aggregates and nested blocks are rejected.
//!
//! ```text
//! cq_datalog::datalog! {
//!     struct Closure;
//!     relation Edge(x:i32,y:i32) = (edges);
//!     relation Reach(x:i32,y:i32);
//!     output Reach;
//!     Reach(*x,*y) :- Edge(x,y);
//!     Reach(*x,*z) :- Reach(x,y), Reach(y,z);
//! }
//! ```
//!
//! `strata { component [R, S]; }` records exact dependency-ordered IDB SCCs.
//! `seminaive { insert_new hash_set_all_columns;
//! negation_keys hash_set_borrowed; delta R = DeltaR;
//! variant 0 seed { ... } variant 1 delta 0 { ... } }` records insertion,
//! borrowed HashSet negation keys, fresh delta declarations and the Boolean
//! cover of changing positive positions. Both policies are required, in this
//! order, and only these fixed representations are supported. Each
//! delta inherits its base relation's complete ordered schema. Every IDB has
//! exactly one delta declaration; EDBs have none. Non-delta reads use current
//! relations, not an exact bag-valued product rule.
//! Full positive reads record `layout R [0,1] key 1;` after deltas and before
//! variants. Columns form a complete permutation; the leading `key` columns
//! are variables bound by earlier positive atoms plus constant expressions.
//! Every variant retains written body order: a delta adds bindings only at its
//! actual position and has no maintained full layout. Wildcards and variables
//! first introduced in the current atom are not keys. Identity/key-zero layouts
//! represent scans. Decisions are deduplicated in rule/changing-position order,
//! independently of the written variant order; negation uses its separate policy.
//! Optional `observe = (|round, delta_rows| ...);` before `struct` observes
//! nonempty rounds; round numbering restarts for each component. Observation
//! effects, like all evaluation counts, are outside the query denotation.

pub use cq_lang_cq::{BodyItem, Query};
use cq_stage::Language;
use cq_syntax::{Atom, RelationDecl, rust, symbol_name};
use proc_macro2::{Span, TokenStream, TokenTree};
use quote::{ToTokens, quote};
mod layouts;
use std::collections::{BTreeMap, BTreeSet};
use syn::{
    Expr, Ident, Token,
    parse::{Parse, ParseStream},
};

pub mod kw {
    syn::custom_keyword!(output);
    syn::custom_keyword!(observe);
    syn::custom_keyword!(strata);
    syn::custom_keyword!(component);
    syn::custom_keyword!(seminaive);
    syn::custom_keyword!(variant);
    syn::custom_keyword!(seed);
    syn::custom_keyword!(delta);
    syn::custom_keyword!(insert_new);
    syn::custom_keyword!(negation_keys);
    syn::custom_keyword!(layout);
    syn::custom_keyword!(key);
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pub name: Ident,
    pub declarations: Vec<RelationDecl>,
    pub outputs: Vec<Ident>,
    pub rules: Vec<Query>,
    pub observer: Option<Expr>,
    pub strata: Option<Vec<Vec<Ident>>>,
    pub variants: Option<Vec<Variant>>,
    pub deltas: Vec<Delta>,
    pub layouts: Vec<Layout>,
    pub insertion: Option<Ident>,
    pub negation_keys: Option<Ident>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Variant {
    pub rule: usize,
    pub delta: Option<usize>,
    pub query: Query,
}
/// A fresh delta relation inheriting the named IDB's ordered columns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delta {
    pub relation: Ident,
    pub name: Ident,
}
/// A maintained full-relation grouping, with key columns preceding payload columns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub relation: Ident,
    pub columns: Vec<usize>,
    pub key: usize,
}
fn number(input: ParseStream<'_>) -> syn::Result<usize> {
    input.parse::<syn::LitInt>()?.base10_parse()
}
impl Parse for Program {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let observer = if input.peek(kw::observe) {
            input.parse::<kw::observe>()?;
            input.parse::<Token![=]>()?;
            let e = input.parse()?;
            input.parse::<Token![;]>()?;
            Some(e)
        } else {
            None
        };
        input.parse::<Token![struct]>()?;
        let name = input.parse()?;
        input.parse::<Token![;]>()?;
        let mut declarations = Vec::new();
        while input.peek(cq_syntax::kw::relation) {
            declarations.push(input.parse()?);
        }
        input.parse::<kw::output>()?;
        let mut outputs = vec![input.parse()?];
        while input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
            outputs.push(input.parse()?);
        }
        input.parse::<Token![;]>()?;
        let mut rules = Vec::new();
        while input.peek(Ident) && input.peek2(syn::token::Paren) {
            rules.push(input.parse()?);
        }
        let strata = if input.peek(kw::strata) {
            input.parse::<kw::strata>()?;
            let body;
            syn::braced!(body in input);
            let mut components = Vec::new();
            while !body.is_empty() {
                body.parse::<kw::component>()?;
                let members;
                syn::bracketed!(members in body);
                components.push(
                    members
                        .parse_terminated(Ident::parse, Token![,])?
                        .into_iter()
                        .collect(),
                );
                body.parse::<Token![;]>()?;
            }
            Some(components)
        } else {
            None
        };
        let (variants, insertion, negation_keys, deltas, layouts) = if input.peek(kw::seminaive) {
            input.parse::<kw::seminaive>()?;
            let body;
            syn::braced!(body in input);
            body.parse::<kw::insert_new>()?;
            let insertion = body.parse()?;
            body.parse::<Token![;]>()?;
            body.parse::<kw::negation_keys>()?;
            let negation_keys = body.parse()?;
            body.parse::<Token![;]>()?;
            let mut deltas = Vec::new();
            while body.peek(kw::delta) {
                body.parse::<kw::delta>()?;
                let relation = body.parse()?;
                body.parse::<Token![=]>()?;
                let name = body.parse()?;
                body.parse::<Token![;]>()?;
                deltas.push(Delta { relation, name });
            }
            let mut layouts = Vec::new();
            while body.peek(kw::layout) {
                body.parse::<kw::layout>()?;
                let relation = body.parse()?;
                let columns;
                syn::bracketed!(columns in body);
                let columns = columns
                    .parse_terminated(number, Token![,])?
                    .into_iter()
                    .collect();
                body.parse::<kw::key>()?;
                let key = number(&body)?;
                body.parse::<Token![;]>()?;
                layouts.push(Layout {
                    relation,
                    columns,
                    key,
                });
            }
            let mut variants = Vec::new();
            while !body.is_empty() {
                body.parse::<kw::variant>()?;
                let rule = number(&body)?;
                let delta = if body.peek(kw::seed) {
                    body.parse::<kw::seed>()?;
                    None
                } else {
                    body.parse::<kw::delta>()?;
                    Some(number(&body)?)
                };
                let query;
                syn::braced!(query in body);
                variants.push(Variant {
                    rule,
                    delta,
                    query: query.parse()?,
                });
                if !query.is_empty() {
                    return Err(query.error("unexpected variant text"));
                }
            }
            (
                Some(variants),
                Some(insertion),
                Some(negation_keys),
                deltas,
                layouts,
            )
        } else {
            (None, None, None, Vec::new(), Vec::new())
        };
        Ok(Self {
            name,
            declarations,
            outputs,
            rules,
            observer,
            strata,
            variants,
            deltas,
            layouts,
            insertion,
            negation_keys,
        })
    }
}
impl ToTokens for Program {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let Self {
            name,
            declarations,
            outputs,
            rules,
            observer,
            strata,
            variants,
            deltas,
            layouts,
            insertion,
            negation_keys,
        } = self;
        if let Some(e) = observer {
            tokens.extend(quote!(observe = #e;));
        }
        tokens.extend(quote!(struct #name; #(#declarations)* output #(#outputs),*; #(#rules)*));
        if let Some(components) = strata {
            let components = components
                .iter()
                .map(|members| quote!(component [#(#members),*];));
            tokens.extend(quote!(strata { #(#components)* }));
        }
        if let Some(variants) = variants {
            let variants = variants.iter().map(|v| {
                let rule = syn::Index::from(v.rule);
                let query = &v.query;
                let kind = v
                    .delta
                    .map(|p| {
                        let p = syn::Index::from(p);
                        quote!(delta #p)
                    })
                    .unwrap_or(quote!(seed));
                quote!(variant #rule #kind { #query })
            });
            let deltas = deltas.iter().map(|d| {
                let Delta { relation, name } = d;
                quote!(delta #relation = #name;)
            });
            let layouts = layouts.iter().map(|layout| {
                let relation = &layout.relation;
                let columns = layout.columns.iter().copied().map(syn::Index::from);
                let key = syn::Index::from(layout.key);
                quote!(layout #relation [#(#columns),*] key #key;)
            });
            tokens.extend(quote!(seminaive {
                insert_new #insertion;
                negation_keys #negation_keys;
                #(#deltas)* #(#layouts)* #(#variants)*
            }));
        }
    }
}
fn ensure(ok: bool, at: impl ToTokens, message: &str) -> syn::Result<()> {
    if ok {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(at, message))
    }
}
fn active_column(e: &Expr) -> Option<&Ident> {
    match rust::unparen(e) {
        Expr::Unary(e) if matches!(e.op, syn::UnOp::Deref(_)) => {
            rust::variable(rust::unparen(&e.expr))
        }
        Expr::MethodCall(e)
            if e.method == "clone" && e.args.is_empty() && e.turbofish.is_none() =>
        {
            active_column(&e.receiver)
        }
        _ => None,
    }
}
/// Dependency graph indexed by declaration order; an edge is (head, input, negative).
#[derive(Clone, Debug)]
pub struct Graph {
    pub names: Vec<String>,
    pub edges: Vec<(usize, usize, bool)>,
    pub reach: Vec<Vec<bool>>,
}
impl Graph {
    fn index(&self, id: &Ident) -> syn::Result<usize> {
        self.names
            .iter()
            .position(|s| *s == symbol_name(id))
            .ok_or_else(|| syn::Error::new_spanned(id, "undeclared relation"))
    }
    fn new(p: &Program) -> syn::Result<Self> {
        cq_syntax::check_input_declarations(&p.declarations)?;
        let names: Vec<_> = p
            .declarations
            .iter()
            .map(|d| symbol_name(&d.name))
            .collect();
        let mut g = Self {
            reach: vec![vec![false; names.len()]; names.len()],
            names,
            edges: Vec::new(),
        };
        for q in &p.rules {
            let head = g.index(&q.head.relation)?;
            ensure(
                p.declarations[head].value.is_none(),
                &q.head,
                "EDB relations cannot have rules",
            )?;
            for item in &q.body {
                let atom = match item {
                    BodyItem::Positive { atom } => Some((atom, false)),
                    BodyItem::Filter(_)
                    | BodyItem::Let(_)
                    | BodyItem::Equality(_)
                    | BodyItem::Constraint(_) => None,
                    BodyItem::Negation(negation) => Some((&negation.atom, true)),
                    _ => {
                        return Err(syn::Error::new_spanned(
                            item,
                            "Datalog bodies support positive atoms, Rust clauses and !R atoms; extensions, aggregates and nested blocks are not supported",
                        ));
                    }
                };
                if let Some((atom, strict)) = atom {
                    let input = g.index(&atom.relation)?;
                    ensure(
                        atom.args.len() == p.declarations[input].arity(),
                        atom,
                        "atom arity mismatch",
                    )?;
                    g.edges.push((head, input, strict));
                    g.reach[head][input] = true;
                }
            }
        }
        for k in 0..g.names.len() {
            for i in 0..g.names.len() {
                for j in 0..g.names.len() {
                    g.reach[i][j] |= g.reach[i][k] && g.reach[k][j];
                }
            }
        }
        for &(h, b, strict) in &g.edges {
            ensure(
                !strict || !g.reach[b][h],
                &p.name,
                "unstratified negative dependency inside an SCC",
            )?;
        }
        Ok(g)
    }
}
fn check_source(p: &Program) -> syn::Result<Graph> {
    let g = Graph::new(p)?;
    ensure(
        !p.rules.is_empty() || p.declarations.iter().all(|d| d.value.is_some()),
        &p.name,
        "Datalog requires at least one rule when IDBs are declared",
    )?;
    ensure(
        !p.outputs.is_empty(),
        &p.name,
        "Datalog requires at least one output",
    )?;
    let mut outputs = BTreeSet::new();
    for output in &p.outputs {
        ensure(
            outputs.insert(symbol_name(output)),
            output,
            "duplicate output relation",
        )?;
        g.index(output)?;
    }
    for q in &p.rules {
        let head = g.index(&q.head.relation)?;
        let columns: BTreeSet<_> = q
            .body
            .iter()
            .filter_map(|i| match i {
                BodyItem::Positive { atom } => Some(atom),
                _ => None,
            })
            .flat_map(Atom::variables)
            .map(symbol_name)
            .collect();
        for arg in &q.head.args {
            if g.reach[head][head] {
                let id = active_column(arg).ok_or_else(|| {
                    syn::Error::new_spanned(
                        arg,
                        "recursive heads must copy an active-domain column: *x or (*x).clone()",
                    )
                })?;
                ensure(
                    columns.contains(&symbol_name(id)),
                    arg,
                    "recursive head column must come from a positive atom",
                )?;
            }
        }
        check_query(p, q, &p.declarations)?;
    }
    Ok(g)
}
/// Formal rule DL-Strata: Certify the exact SCC partition and dependency order of the visible graph.
fn check_strata(p: &Program, g: &Graph) -> syn::Result<()> {
    let Some(components) = &p.strata else {
        return Ok(());
    };
    let mut positions = BTreeMap::new();
    for (c, members) in components.iter().enumerate() {
        ensure(!members.is_empty(), &p.name, "empty SCC")?;
        for member in members {
            let i = g.index(member)?;
            ensure(
                p.declarations[i].value.is_none() && positions.insert(i, c).is_none(),
                member,
                "SCCs must partition IDB relations exactly",
            )?;
        }
    }
    for (i, d) in p.declarations.iter().enumerate() {
        ensure(
            d.value.is_some() || positions.contains_key(&i),
            d,
            "missing IDB SCC",
        )?;
    }
    for (&i, &ci) in &positions {
        for (&j, &cj) in &positions {
            ensure(
                (ci == cj) == (i == j || (g.reach[i][j] && g.reach[j][i])),
                &p.name,
                "recorded component is not an SCC",
            )?;
        }
    }
    for &(h, b, strict) in &g.edges {
        if let (Some(&h), Some(&b)) = (positions.get(&h), positions.get(&b)) {
            ensure(
                if strict { b < h } else { b <= h },
                &p.name,
                "SCCs are not in dependency order",
            )?;
        }
    }
    Ok(())
}
/// Formal rule DL-Delta: Check variant erasure and exactly one delta read per changing position.
fn check_variants(p: &Program, analysis: &Analysis) -> syn::Result<()> {
    let Some(variants) = &p.variants else {
        return ensure(
            p.insertion.is_none()
                && p.negation_keys.is_none()
                && p.deltas.is_empty()
                && p.layouts.is_empty(),
            p,
            "insertion and negation key policies, deltas and layouts require seminaive variants",
        );
    };
    ensure(p.strata.is_some(), p, "seminaive requires strata")?;
    ensure(
        p.insertion
            .as_ref()
            .is_some_and(|i| i == "hash_set_all_columns"),
        p,
        "insert_new requires the canonical all-column HashSet",
    )?;
    ensure(
        p.negation_keys
            .as_ref()
            .is_some_and(|i| i == "hash_set_borrowed"),
        p,
        "negation_keys requires the borrowed-key HashSet",
    )?;
    let mut seen = BTreeSet::new();
    let mut declarations = p.declarations.clone();
    for delta in &p.deltas {
        let mut declaration =
            p.declarations[analysis.positions[&symbol_name(&delta.relation)]].clone();
        declaration.name = delta.name.clone();
        declarations.push(declaration);
    }
    for v in variants {
        ensure(
            v.rule < p.rules.len() && seen.insert((v.rule, v.delta)),
            p,
            "duplicate or invalid variant",
        )?;
        let positions = &analysis.changing_positions[v.rule];
        ensure(
            match v.delta {
                Some(i) => positions.contains(&i),
                None => positions.is_empty(),
            },
            p,
            "incorrect seed/delta classification",
        )?;
        // Check complete declared schemas and CQ scope independently of erasure.
        check_query(p, &v.query, &declarations)?;
        let mut erased = v.query.clone();
        if let Some(i) = v.delta {
            let original = &p.rules[v.rule].body[i];
            let BodyItem::Positive { atom: original } = original else {
                unreachable!()
            };
            let Some(BodyItem::Positive { atom }) = erased.body.iter_mut().nth(i) else {
                return Err(syn::Error::new_spanned(p, "missing delta atom"));
            };
            ensure(
                atom.relation == analysis.deltas[&symbol_name(&original.relation)],
                atom.clone(),
                "delta read name changed",
            )?;
            atom.relation = original.relation.clone();
        }
        ensure(
            erased == p.rules[v.rule],
            &v.query,
            "variant erasure differs from the rule",
        )?;
    }
    for i in 0..p.rules.len() {
        let positions = &analysis.changing_positions[i];
        if positions.is_empty() {
            ensure(seen.contains(&(i, None)), p, "missing seed variant")?;
        }
        for position in positions {
            ensure(
                seen.contains(&(i, Some(*position))),
                p,
                "missing changing-position variant",
            )?;
        }
    }
    Ok(())
}
fn source_only(p: &Program) -> Program {
    let mut p = p.clone();
    p.strata = None;
    p.variants = None;
    p.insertion = None;
    p.negation_keys = None;
    p.deltas.clear();
    p.layouts.clear();
    p
}
/// Facts established by the language contract, indexed in declaration/rule order.
#[derive(Clone, Debug)]
pub struct Analysis {
    pub graph: Graph,
    pub positions: BTreeMap<String, usize>,
    pub dependencies: Vec<(usize, usize, bool)>,
    pub components: Vec<Vec<usize>>,
    /// Recorded component for each declaration; EDBs and unscheduled IDBs are None.
    pub component: Vec<Option<usize>>,
    pub changing_positions: Vec<Vec<usize>>,
    pub deltas: BTreeMap<String, Ident>,
}

/// Check a complete source, stratified, or semi-naive program.
///
/// Source rules and variants each receive a complete CQ schema/scope contract
/// check, with delta input schemas inherited from their explicit declarations
/// and a fresh output declaration. For native negation only, this uses a
/// non-executable positive validation projection after checking bound-before
/// safety. Stratification separately certifies completed negative inputs.
/// This does not claim that raw negative variants satisfy the core CQ contract:
/// core CQ still rejects negation, and fork must check its actual lowered holes.
/// Exact variant erasure is an additional, independent obligation.
pub mod contract {
    use super::*;

    pub fn check(program: &Program) -> syn::Result<Analysis> {
        let graph = check_source(program)?;
        check_strata(program, &graph)?;
        let positions: BTreeMap<_, _> = graph
            .names
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, name)| (name, index))
            .collect();
        let components: Vec<Vec<usize>> = program
            .strata
            .iter()
            .flatten()
            .map(|members| {
                members
                    .iter()
                    .map(|id| positions[&symbol_name(id)])
                    .collect()
            })
            .collect();
        let mut component = vec![None; program.declarations.len()];
        for (index, members) in components.iter().enumerate() {
            for &member in members {
                component[member] = Some(index);
            }
        }
        let changing_positions = program
            .rules
            .iter()
            .map(|query| {
                let head = positions[&symbol_name(&query.head.relation)];
                query
                    .body
                    .iter()
                    .enumerate()
                    .filter_map(|(index, item)| {
                        if let BodyItem::Positive { atom } = item {
                            let input = positions[&symbol_name(&atom.relation)];
                            if component[head].is_some() && component[head] == component[input] {
                                return Some(index);
                            }
                        }
                        None
                    })
                    .collect()
            })
            .collect();
        let mut deltas = BTreeMap::new();
        let mut fresh = Fresh::new(source_only(program));
        for delta in &program.deltas {
            let index = graph.index(&delta.relation)?;
            ensure(
                program.declarations[index].value.is_none(),
                &delta.relation,
                "delta declarations must name IDB relations",
            )?;
            ensure(
                deltas
                    .insert(symbol_name(&delta.relation), delta.name.clone())
                    .is_none(),
                &delta.relation,
                "duplicate delta declaration for relation",
            )?;
            ensure(
                fresh.used.insert(symbol_name(&delta.name)),
                &delta.name,
                "delta relation name must be fresh in the complete source",
            )?;
        }
        if program.variants.is_some() {
            for declaration in &program.declarations {
                ensure(
                    declaration.value.is_some()
                        || deltas.contains_key(&symbol_name(&declaration.name)),
                    declaration,
                    "missing IDB delta declaration",
                )?;
            }
        }
        let analysis = Analysis {
            dependencies: graph.edges.clone(),
            graph,
            positions,
            components,
            component,
            changing_positions,
            deltas,
        };
        check_variants(program, &analysis)?;
        layouts::check(program, &analysis)?;
        Ok(analysis)
    }
}

/// A rewrite preserves source meaning and every previously recorded decision.
pub fn check_rewrite(source: &Program, target: &Program) -> syn::Result<()> {
    contract::check(source)?;
    contract::check(target)?;
    ensure(
        source_only(source) == source_only(target),
        target,
        "Datalog pass changed source rules or declarations",
    )?;
    ensure(
        source.strata.is_none() || source.strata == target.strata,
        target,
        "Datalog pass changed existing strata",
    )?;
    ensure(
        source.variants.is_none()
            || (source.variants == target.variants
                && source.deltas == target.deltas
                && source.insertion == target.insertion
                && source.layouts == target.layouts
                && source.negation_keys == target.negation_keys),
        target,
        "Datalog pass changed existing semi-naive decisions",
    )
}

impl Language for Program {
    fn check(&self) -> syn::Result<()> {
        contract::check(self).map(|_| ())
    }
    fn check_rewrite(source: &Self, target: &Self) -> syn::Result<()> {
        check_rewrite(source, target)
    }
}

// A negative atom contributes no bindings. Once its safety is established,
// its positive validation projection exercises CQ declaration, arity and scope
// obligations. Rust types remain checked by rustc on the emitted CQ holes.
fn check_query(p: &Program, q: &Query, declarations: &[RelationDecl]) -> syn::Result<()> {
    let head = declarations
        .iter()
        .find(|d| symbol_name(&d.name) == symbol_name(&q.head.relation))
        .ok_or_else(|| syn::Error::new_spanned(&q.head, "undeclared rule head"))?;
    ensure(
        q.head.args.len() == head.arity(),
        &q.head,
        "head arity mismatch",
    )?;
    let bound = cq_lang_cq::blocks::bound(&q.body);
    for arg in &q.head.args {
        if let Some(id) = active_column(arg).or_else(|| rust::variable(arg)) {
            ensure(
                bound.contains_key(&symbol_name(id)),
                arg,
                "head variable must be range restricted",
            )?;
        }
    }
    let mut fresh = Fresh::new(p);
    let name = fresh.id("checked_head");
    let mut query = q.clone();
    query.head.relation = name.clone();
    let mut declarations = declarations.to_vec();
    let mut output = head.clone();
    output.name = name;
    declarations.push(output);
    let locals = bound.keys().cloned().collect();
    let mut scope = BTreeSet::new();
    for item in &mut query.body {
        match item {
            BodyItem::Negation(negative) => {
                for arg in &negative.atom.args {
                    if matches!(rust::unparen(arg), Expr::Infer(_)) {
                        continue;
                    }
                    if let Some(id) = rust::variable(arg) {
                        ensure(
                            scope.contains(&symbol_name(id)),
                            arg,
                            "negation variable must be bound by a positive atom before the clause",
                        )?;
                    } else {
                        let free = rust::free_variables(arg);
                        ensure(
                            !free.opaque && free.dependencies(&locals).is_empty(),
                            arg,
                            "negation constants must not depend on rule-local bindings",
                        )?;
                    }
                }
                *item = BodyItem::Positive {
                    atom: negative.atom.clone(),
                };
            }
            BodyItem::Positive { atom } => scope.extend(atom.variables().map(symbol_name)),
            _ => {}
        }
    }
    let name = &p.name;
    let cq: cq_lang_cq::Program = syn::parse2(quote!(struct #name; #(#declarations)* #query))?;
    cq_lang_cq::contract::check(&cq)
}
struct Fresh {
    used: BTreeSet<String>,
}
impl Fresh {
    fn new(source: impl ToTokens) -> Self {
        fn collect(t: TokenStream, used: &mut BTreeSet<String>) {
            for t in t {
                match t {
                    TokenTree::Ident(id) => {
                        used.insert(symbol_name(&id));
                    }
                    TokenTree::Group(g) => collect(g.stream(), used),
                    _ => {}
                }
            }
        }
        let mut used = BTreeSet::new();
        collect(source.into_token_stream(), &mut used);
        Self { used }
    }
    fn id(&mut self, role: &str) -> Ident {
        for n in 0.. {
            let name = format!("__cq_datalog_{role}_{n}");
            if self.used.insert(name.clone()) {
                return Ident::new(&name, Span::mixed_site());
            }
        }
        unreachable!()
    }
}
