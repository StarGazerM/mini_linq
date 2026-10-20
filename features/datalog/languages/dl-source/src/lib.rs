//! DL-Source denotes the stratified least fixpoint of set-valued relations.
//! Rust clauses are pure partial computations; panic is bottom and termination
//! remains a caller premise. Surface sugar is not part of this language.
use cq_datalog_support::{Fresh, ensure};
pub use cq_lang_cq::{BodyItem, Query};
use cq_stage::Language;
use cq_syntax::{RelationDecl, rust, symbol_name};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use std::collections::{BTreeMap, BTreeSet};
use syn::{
    Expr, Ident, Token,
    parse::{Parse, ParseStream},
};
pub mod kw {
    syn::custom_keyword!(output);
    syn::custom_keyword!(observe);
    syn::custom_keyword!(model);
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pub name: Ident,
    pub declarations: Vec<RelationDecl>,
    pub outputs: Vec<Ident>,
    pub rules: Vec<Query>,
    pub observer: Option<Expr>,
    pub models: BTreeMap<String, Ident>,
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
        let mut models: BTreeMap<String, Ident> = declarations
            .iter()
            .map(|d: &RelationDecl| (symbol_name(&d.name), syn::parse_quote!(set)))
            .collect();
        let mut explicit = BTreeSet::new();
        while input.peek(kw::model) {
            input.parse::<kw::model>()?;
            let relation: Ident = input.parse()?;
            let model: Ident = input.parse()?;
            input.parse::<Token![;]>()?;
            ensure(
                models.contains_key(&symbol_name(&relation)),
                &relation,
                "undeclared model relation",
            )?;
            ensure(
                explicit.insert(symbol_name(&relation)),
                &relation,
                "duplicate relation model",
            )?;
            models.insert(symbol_name(&relation), model);
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
        Ok(Self {
            name,
            declarations,
            outputs,
            rules,
            observer,
            models,
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
            models,
        } = self;
        if let Some(e) = observer {
            tokens.extend(quote!(observe = #e;));
        }
        tokens.extend(quote!(struct #name; #(#declarations)*));
        for declaration in declarations {
            let relation = &declaration.name;
            if let Some(model) = models.get(&symbol_name(relation)) {
                tokens.extend(quote!(model #relation #model;));
            }
        }
        tokens.extend(quote!(output #(#outputs),*; #(#rules)*));
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
    pub fn index(&self, id: &Ident) -> syn::Result<usize> {
        self.names
            .iter()
            .position(|s| *s == symbol_name(id))
            .ok_or_else(|| syn::Error::new_spanned(id, "undeclared relation"))
    }
    pub fn new(p: &Program) -> syn::Result<Self> {
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
                    | BodyItem::For(_)
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
        check_query(p, q, &p.declarations)?;
    }
    Ok(g)
}
#[derive(Clone, Debug)]
pub struct Analysis {
    pub graph: Graph,
    pub positions: BTreeMap<String, usize>,
    pub dependencies: Vec<(usize, usize, bool)>,
}
pub mod contract {
    use super::*;
    /// Formal rule WF-DL-Source: check schemas, scope, models and stratifiability.
    pub fn check(program: &Program) -> syn::Result<Analysis> {
        let graph = check_source(program)?;
        ensure(
            program.models.len() == program.declarations.len(),
            program,
            "models must cover exactly the declared relations",
        )?;
        for declaration in &program.declarations {
            ensure(
                program
                    .models
                    .get(&symbol_name(&declaration.name))
                    .is_some_and(|m| m == "set"),
                declaration,
                "base Datalog supports only the set relation model",
            )?;
        }
        let positions = graph
            .names
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, n)| (n, i))
            .collect();
        Ok(Analysis {
            dependencies: graph.edges.clone(),
            graph,
            positions,
        })
    }
}
/// Rewrites preserve the complete checked source and its model assignment.
pub fn check_rewrite(source: &Program, target: &Program) -> syn::Result<()> {
    contract::check(source)?;
    contract::check(target)?;
    ensure(
        source == target,
        target,
        "Datalog rewrite changed source rules or declarations",
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
pub fn check_query(p: &Program, q: &Query, declarations: &[RelationDecl]) -> syn::Result<()> {
    let head = declarations
        .iter()
        .find(|d| symbol_name(&d.name) == symbol_name(&q.head.relation))
        .ok_or_else(|| syn::Error::new_spanned(&q.head, "undeclared rule head"))?;
    ensure(
        q.head.args.len() == head.arity(),
        &q.head,
        "head arity mismatch",
    )?;
    let mut query = q.clone();
    for item in &query.body {
        if let BodyItem::Filter(filter) = item {
            ensure(
                !matches!(rust::unparen(&filter.condition), Expr::Let(_)),
                item,
                "if let requires a surface expansion before DL-Source",
            )?;
        }
    }
    let bound = cq_lang_cq::blocks::bound(&query.body);
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
