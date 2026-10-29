//! DL-Source denotes the stratified least fixpoint of set-valued relations.
//! Rust clauses are opaque, pure and total by premise. Relational `where`
//! clauses use CQ predicates; parsing, printing and checking share its language.
//! Optional `invariants { hoist N using Helper { original_rule; } }` after
//! the rules records a bounded frozen filter and its independently checked
//! erasure. Helpers preserve one positive input's bindings and cardinality.
//! A rule may carry an optional label, `Label: head :- body;`. Labels are
//! names for advice only: they are unique, have no denotation, and rewrites
//! that add rules leave the new rules unlabelled.
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
pub mod invariants;
pub mod kw {
    syn::custom_keyword!(output);
    syn::custom_keyword!(observe);
    syn::custom_keyword!(model);
    syn::custom_keyword!(invariants);
    syn::custom_keyword!(hoist);
    syn::custom_keyword!(using);
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pub name: Ident,
    pub declarations: Vec<RelationDecl>,
    pub outputs: Vec<Ident>,
    pub rules: Vec<Query>,
    /// Optional advice label per rule, parallel to `rules`.
    pub labels: Vec<Option<Ident>>,
    pub observer: Option<Expr>,
    pub models: BTreeMap<String, Ident>,
    pub invariants: Vec<Invariant>,
}
/// A checked frozen-filter factorization retaining the original rule for erasure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invariant {
    pub rule: usize,
    pub helper: Ident,
    pub source: Query,
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
        let mut labels = Vec::new();
        loop {
            let label = if input.peek(Ident) && input.peek2(Token![:]) && !input.peek2(Token![::]) {
                let label = input.parse()?;
                input.parse::<Token![:]>()?;
                Some(label)
            } else if input.peek(Ident) && input.peek2(syn::token::Paren) {
                None
            } else {
                break;
            };
            rules.push(input.parse()?);
            labels.push(label);
        }
        let mut invariants = Vec::new();
        if input.peek(kw::invariants) {
            input.parse::<kw::invariants>()?;
            let body;
            syn::braced!(body in input);
            while !body.is_empty() {
                body.parse::<kw::hoist>()?;
                let rule = body.parse::<syn::LitInt>()?.base10_parse()?;
                body.parse::<kw::using>()?;
                let helper = body.parse()?;
                let original;
                syn::braced!(original in body);
                let source = original.parse()?;
                if !original.is_empty() {
                    return Err(original.error("unexpected invariant source text"));
                }
                invariants.push(Invariant {
                    rule,
                    helper,
                    source,
                });
            }
        }
        Ok(Self {
            name,
            declarations,
            outputs,
            rules,
            labels,
            observer,
            models,
            invariants,
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
            labels,
            observer,
            models,
            invariants,
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
        let rules = rules.iter().zip(labels).map(|(rule, label)| match label {
            Some(label) => quote!(#label: #rule),
            None => quote!(#rule),
        });
        tokens.extend(quote!(output #(#outputs),*; #(#rules)*));
        if !invariants.is_empty() {
            let entries = invariants.iter().map(|entry| {
                let rule = syn::Index::from(entry.rule);
                let helper = &entry.helper;
                let source = &entry.source;
                quote!(hoist #rule using #helper { #source })
            });
            tokens.extend(quote!(invariants { #(#entries)* }));
        }
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
        let positions: BTreeMap<_, _> = g
            .names
            .iter()
            .enumerate()
            .map(|(index, name)| (name.as_str(), index))
            .collect();
        let index = |id: &Ident| {
            positions
                .get(symbol_name(id).as_str())
                .copied()
                .ok_or_else(|| syn::Error::new_spanned(id, "undeclared relation"))
        };
        for q in &p.rules {
            let head = index(&q.head.relation)?;
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
                            "Datalog bodies support positive atoms, Rust clauses, where predicates and !R atoms; extensions, aggregates and nested blocks are not supported",
                        ));
                    }
                };
                if let Some((atom, strict)) = atom {
                    let input = index(&atom.relation)?;
                    ensure(
                        atom.args.len() == p.declarations[input].arity(),
                        atom,
                        "atom arity mismatch",
                    )?;
                    g.edges.push((head, input, strict));
                }
            }
        }
        // Nonempty-path reachability: only cycles make the diagonal true.
        // Reuse a DFS stack rather than scanning every triple of declarations.
        let mut successors = vec![Vec::new(); g.names.len()];
        for &(head, input, _) in &g.edges {
            successors[head].push(input);
        }
        let mut stack = Vec::new();
        for (start, reached) in g.reach.iter_mut().enumerate() {
            stack.extend(successors[start].iter().copied());
            while let Some(node) = stack.pop() {
                if !reached[node] {
                    reached[node] = true;
                    stack.extend(successors[node].iter().copied());
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
    ensure(
        p.labels.len() == p.rules.len(),
        &p.name,
        "rule labels must be parallel to rules",
    )?;
    let mut labels = BTreeSet::new();
    for label in p.labels.iter().flatten() {
        ensure(
            labels.insert(symbol_name(label)),
            label,
            "duplicate rule label",
        )?;
    }
    let mut queries = QueryChecker::new(p, &p.declarations);
    for q in &p.rules {
        queries.check(q)?;
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
        invariants::check(program)?;
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
/// Check source well-formedness and preserve its externally visible interface.
///
/// Rules may change. The rewrite author must separately establish denotational
/// refinement: the target returns the same output sets under the pure, total
/// Rust premise and the column representation's equality/order laws.
/// This structural check cannot prove equivalence of opaque Rust computations.
pub fn check_rewrite(source: &Program, target: &Program) -> syn::Result<()> {
    contract::check(source)?;
    contract::check(target)?;
    check_rewrite_admitted(source, target)
}

// The stage runner has already checked both operands.
fn check_rewrite_admitted(source: &Program, target: &Program) -> syn::Result<()> {
    if !source.invariants.is_empty() || !target.invariants.is_empty() {
        ensure(
            source.invariants.is_empty() || source.invariants == target.invariants,
            target,
            "Datalog rewrite changed existing invariant decisions",
        )?;
        return check_rewrite_admitted(&invariants::erase(source)?, &invariants::erase(target)?);
    }
    ensure(
        source.name == target.name
            && source.declarations == target.declarations
            && source.outputs == target.outputs
            && source.models == target.models
            && source.observer == target.observer,
        target,
        "Datalog rewrite changed name, declarations, EDB bindings, outputs, models or observer",
    )
}
/// The rewrite callback assumes `cq_stage::run` checked both contracts.
/// Standalone callers must use the public free [`check_rewrite`] function.
impl Language for Program {
    fn check(&self) -> syn::Result<()> {
        contract::check(self).map(|_| ())
    }
    fn check_rewrite(source: &Self, target: &Self) -> syn::Result<()> {
        check_rewrite_admitted(source, target)
    }
}
/// Reusable rule checker with one source-wide fresh-name allocator and schema.
///
/// All declarations are validated, including unused ones. Each CQ projection
/// carries only its referenced declarations, so unrelated schemas are not
/// cloned, printed, parsed or checked again for every rule. CQ still performs
/// its own scope checks and relational-predicate validation.
pub struct QueryChecker<'a> {
    name: &'a Ident,
    checked_head: Ident,
    declarations: BTreeMap<String, (usize, &'a RelationDecl, TokenStream)>,
    declaration_check: syn::Result<()>,
}

impl<'a> QueryChecker<'a> {
    pub fn new(p: &'a Program, declarations: &'a [RelationDecl]) -> Self {
        let declaration_check = cq_syntax::check_input_declarations(declarations).map(|_| ());
        let mut context = BTreeMap::new();
        for (index, declaration) in declarations.iter().enumerate() {
            context
                .entry(symbol_name(&declaration.name))
                .or_insert_with(|| (index, declaration, declaration.to_token_stream()));
        }
        Self {
            name: &p.name,
            checked_head: Fresh::new(p).id("checked_head"),
            declarations: context,
            declaration_check,
        }
    }

    /// Check a rule independently of other rules, retaining standalone errors.
    pub fn check(&mut self, q: &Query) -> syn::Result<()> {
        let head = self
            .declarations
            .get(&symbol_name(&q.head.relation))
            .map(|(_, declaration, _)| *declaration)
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
        // Each projection is an independent CQ program, so reuse precisely the
        // name that a standalone check would allocate from the source tokens.
        query.head.relation = self.checked_head.clone();
        let mut output = head.clone();
        output.name = self.checked_head.clone();
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
        self.declaration_check.clone()?;
        let mut referenced = BTreeSet::new();
        // Include a colliding synthetic name from a caller-supplied schema, just as
        // the full-schema standalone projection did (Fresh reserves source tokens).
        referenced.insert(symbol_name(&output.name));
        for item in cq_lang_cq::blocks::walk(&query.body) {
            if let Some(atom) = item.atom() {
                referenced.insert(symbol_name(&atom.relation));
            }
        }
        let mut declarations: Vec<_> = referenced
            .iter()
            .filter_map(|name| self.declarations.get(name))
            .collect();
        declarations.sort_unstable_by_key(|(index, _, _)| *index);
        let declarations = declarations.iter().map(|(_, _, tokens)| tokens);
        let name = self.name;
        let cq: cq_lang_cq::Program =
            syn::parse2(quote!(struct #name; #(#declarations)* #output #query))?;
        cq_lang_cq::contract::check(&cq)
    }
}

// A negative atom contributes no bindings. Once its safety is established,
// its positive validation projection exercises CQ declaration, arity and scope
// obligations. Rust types remain checked by rustc on the emitted CQ holes.
pub fn check_query(p: &Program, q: &Query, declarations: &[RelationDecl]) -> syn::Result<()> {
    QueryChecker::new(p, declarations).check(q)
}
