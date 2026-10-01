//! CQ normal form for `T X = bottom + Set X`.
//!
//! Atoms contain only variables. Relational equality is Leibniz identity,
//! decided by an admissible column's `==`. Rust lets and guards are opaque
//! partial functions under functional extensionality. The reference is
//! written order. Preserving the Rust chain and including every source
//! relational predecessor before each Rust step is a sufficient refinement
//! condition. I/O, mutation, counters and evaluation counts are outside the
//! meaning; no compiler law depends on those effects.

use super::{Atom, BodyItem, Equality, EqualityValue, Let, Program, Query, rust};
use crate::symbol_name;
use proc_macro2::{Span, TokenStream, TokenTree};
use quote::{ToTokens, format_ident, quote};
use std::collections::{BTreeMap, BTreeSet};
use syn::Ident;

/// Formal rule WF-NF: Check CQ, distinct bare atom arguments and variable-only output.
pub fn check(module: &Program) -> syn::Result<()> {
    super::contract::check(module)?;
    for item in &module.query.body {
        match item {
            BodyItem::Positive { atom } if atom.is_normalized() => {}
            BodyItem::Positive { atom } => {
                return Err(syn::Error::new_spanned(
                    atom,
                    format!(
                        "normal form: `{}` atom arguments are distinct variables",
                        atom.relation
                    ),
                ));
            }
            BodyItem::Equality(equality) => equality.check()?,
            BodyItem::Constraint(constraint) => constraint.check()?,
            BodyItem::Filter(_) | BodyItem::Let(_) => {}
            BodyItem::Negation(_) | BodyItem::Aggregate(_) | BodyItem::Extension { .. } => {
                return Err(syn::Error::new_spanned(
                    item,
                    "normal form covers positive queries with relational equalities, guards and bindings",
                ));
            }
        }
    }
    if module
        .query
        .head
        .args
        .iter()
        .any(|argument| rust::variable(argument).is_none())
    {
        return Err(syn::Error::new_spanned(
            &module.query.head,
            "normal form: output arguments are variables",
        ));
    }
    let bound: BTreeSet<_> = module
        .query
        .body
        .iter()
        .flat_map(|item| match item {
            BodyItem::Positive { atom } => atom.variables().map(symbol_name).collect::<Vec<_>>(),
            BodyItem::Let(binding) => rust::bindings(&binding.pattern)
                .iter()
                .map(symbol_name)
                .collect(),
            _ => Vec::new(),
        })
        .collect();
    if let Some(variable) = module
        .query
        .head
        .variables()
        .find(|v| !bound.contains(&symbol_name(v)))
    {
        return Err(syn::Error::new_spanned(
            variable,
            "normal form: output variable is not bound by the body",
        ));
    }
    check_leibniz_types(module)
}

/// Canonicalize at each atom's written position.
/// Formal rule A-Variable: Keep the first occurrence within this atom, including shared query names.
/// Formal rule A-Repeat: Replace only a variable repeated within one atom and append identity.
/// Formal rule A-Wildcard: Replace a wildcard with an unused witness variable.
/// Formal rule A-Constant: Replace a literal and append relational literal equality.
/// Formal rule A-Prefix: Place an opaque binding immediately before its atom.
/// Formal rule A-Local: Place a dependent binding and relational equality after its atom.
pub fn normalize(module: &Program) -> syn::Result<Program> {
    super::contract::check(module)?;
    let declarations: BTreeMap<_, _> = module.inputs().map(|d| (symbol_name(&d.name), d)).collect();
    let mut names = reserved(module);
    let mut body = Vec::new();
    let mut bound = BTreeSet::new();
    for (clause, item) in module.query.body.iter().enumerate() {
        match item {
            BodyItem::Positive { atom } => {
                let mut atom_seen = BTreeSet::new();
                let declaration = declarations[&symbol_name(&atom.relation)];
                let local: BTreeSet<_> = atom
                    .variables()
                    .map(symbol_name)
                    .filter(|v| !bound.contains(v))
                    .collect();
                let mut args = Vec::new();
                let mut after = Vec::new();
                for (position, (argument, column)) in
                    atom.args.iter().zip(&declaration.columns).enumerate()
                {
                    if let Some(variable) = rust::variable(argument) {
                        if atom_seen.insert(symbol_name(variable)) {
                            args.push(argument.clone());
                        } else {
                            let key = fresh(&mut names, clause, position);
                            after.push(equality(
                                variable.clone(),
                                EqualityValue::Variable(key.clone()),
                            ));
                            args.push(path(&key));
                        }
                        continue;
                    }
                    let key = fresh(&mut names, clause, position);
                    if matches!(argument, syn::Expr::Infer(_)) {
                        // A wildcard binds an unconstrained, unused witness.
                    } else if super::syntax::equality::literal(argument) {
                        after.push(equality(
                            key.clone(),
                            EqualityValue::Literal(argument.clone()),
                        ));
                    } else if rust::free_variables(argument)
                        .dependencies(&local)
                        .is_empty()
                    {
                        body.push(binding(&key, &column.ty, argument.clone()));
                    } else {
                        let value = fresh(&mut names, clause, position);
                        after.push(binding(&value, &column.ty, argument.clone()));
                        after.push(equality(key.clone(), EqualityValue::Variable(value)));
                    }
                    atom_seen.insert(symbol_name(&key));
                    args.push(path(&key));
                }
                bound.extend(atom.variables().map(symbol_name));
                body.push(BodyItem::Positive {
                    atom: Atom {
                        relation: atom.relation.clone(),
                        paren_token: atom.paren_token,
                        args: args.into_iter().collect(),
                    },
                });
                body.extend(after);
            }
            BodyItem::Equality(_) | BodyItem::Constraint(_) | BodyItem::Filter(_) => {
                body.push(item.clone())
            }
            BodyItem::Let(binding) => {
                bound.extend(rust::bindings(&binding.pattern).iter().map(symbol_name));
                body.push(item.clone());
            }
            BodyItem::Negation(_) | BodyItem::Aggregate(_) | BodyItem::Extension { .. } => {
                return Err(syn::Error::new_spanned(
                    item,
                    "normal form covers positive queries with relational equalities, guards and bindings",
                ));
            }
        }
    }
    let mut normal = module.clone();
    for (position, argument) in normal.query.head.args.iter_mut().enumerate() {
        if rust::variable(argument).is_some_and(|variable| bound.contains(&symbol_name(variable))) {
            continue;
        }
        let key = fresh(&mut names, module.query.body.len(), position);
        let column = &module
            .output()
            .expect("computed output types checked")
            .columns[position];
        body.push(binding(&key, &column.ty, argument.clone()));
        *argument = path(&key);
    }
    normal.query.body = body.into_iter().collect();
    check(&normal)?;
    Ok(normal)
}

fn equality(left: Ident, right: EqualityValue) -> BodyItem {
    BodyItem::Equality(Equality {
        left,
        eq_token: Default::default(),
        right,
    })
}

fn binding(key: &Ident, ty: &syn::Type, expression: syn::Expr) -> BodyItem {
    BodyItem::Let(Box::new(Let {
        let_token: Default::default(),
        pattern: syn::Pat::Ident(syn::PatIdent {
            attrs: Vec::new(),
            by_ref: None,
            mutability: None,
            ident: key.clone(),
            subpat: None,
        }),
        annotation: Some(super::TypeAnnotation {
            colon_token: Default::default(),
            ty: ty.clone(),
        }),
        eq_token: Default::default(),
        expression,
    }))
}

fn path(ident: &Ident) -> syn::Expr {
    syn::parse2(quote!(#ident)).expect("identifier expression")
}

fn check_leibniz_types(module: &Program) -> syn::Result<()> {
    let schemas: BTreeMap<_, _> = module.inputs().map(|d| (symbol_name(&d.name), d)).collect();
    let mut columns: BTreeMap<String, Vec<(&syn::Type, String)>> = BTreeMap::new();
    let mut required = BTreeSet::new();
    let mut let_bound = BTreeSet::new();
    for item in &module.query.body {
        match item {
            BodyItem::Positive { atom } => {
                for (variable, column) in atom
                    .variables()
                    .zip(&schemas[&symbol_name(&atom.relation)].columns)
                {
                    let name = symbol_name(variable);
                    let entries = columns.entry(name.clone()).or_default();
                    if !entries.is_empty() || let_bound.contains(&name) {
                        required.insert(name);
                    }
                    entries.push((&column.ty, format!("{}.{}", atom.relation, column.name)));
                }
            }
            BodyItem::Equality(equality) => required.extend(equality.variables().map(symbol_name)),
            BodyItem::Constraint(constraint) => {
                required.extend(constraint.variables().map(symbol_name))
            }
            BodyItem::Let(binding) => {
                let_bound.extend(rust::bindings(&binding.pattern).iter().map(symbol_name))
            }
            _ => {}
        }
    }
    let ordered: BTreeSet<_> = module
        .query
        .body
        .iter()
        .filter_map(|item| match item {
            BodyItem::Constraint(c) => Some(c.ordered_variables()),
            _ => None,
        })
        .flatten()
        .map(symbol_name)
        .collect();
    for item in &module.query.body {
        if let BodyItem::Let(binding) = item {
            for (name, ty) in binding.binding_types() {
                if ordered.contains(&symbol_name(&name)) {
                    super::syntax::algebra::check_ord_type(&ty, &name)?;
                }
            }
        }
    }
    if let Some(output) = module.output() {
        for column in &output.columns {
            super::syntax::algebra::check_leibniz_type(&column.ty, &column.name)?;
        }
    }
    for item in &module.query.body {
        if let BodyItem::Let(binding) = item {
            for (name, ty) in binding.binding_types() {
                if module
                    .query
                    .head
                    .variables()
                    .any(|v| symbol_name(v) == symbol_name(&name))
                    || required.contains(&symbol_name(&name))
                {
                    super::syntax::algebra::check_leibniz_type(&ty, &name)?;
                }
            }
        }
    }
    required.extend(module.query.head.variables().map(symbol_name));
    for variable in required {
        if let Some(entries) = columns.get(&variable) {
            for (ty, column) in entries {
                super::syntax::algebra::check_leibniz_type(
                    ty,
                    &Ident::new(&variable, Span::call_site()),
                )
                .map_err(|error| {
                    syn::Error::new_spanned(ty, format!("column `{column}`: {error}"))
                })?;
            }
        }
    }
    for item in &module.query.body {
        if let BodyItem::Constraint(c) = item {
            for variable in c.ordered_variables() {
                if let Some(entries) = columns.get(&symbol_name(variable)) {
                    for (ty, _) in entries {
                        super::syntax::algebra::check_ord_type(ty, variable)?;
                    }
                }
            }
        }
    }
    Ok(())
}

/// Every identifier the module mentions, so fresh names never capture.
fn reserved(module: &Program) -> BTreeSet<String> {
    fn visit(tokens: TokenStream, names: &mut BTreeSet<String>) {
        for token in tokens {
            match token {
                TokenTree::Ident(id) => {
                    names.insert(symbol_name(&id));
                }
                TokenTree::Group(g) => visit(g.stream(), names),
                _ => (),
            }
        }
    }
    let mut names = BTreeSet::new();
    visit(module.to_token_stream(), &mut names);
    names
}

/// `__cq_<clause>_<argument>`, extended until unused.
fn fresh(names: &mut BTreeSet<String>, clause: usize, argument: usize) -> Ident {
    let mut name = format!("__cq_{clause}_{argument}");
    while !names.insert(name.clone()) {
        name.push('_');
    }
    format_ident!("{name}", span = Span::call_site())
}

/// Structural query content compared by elaboration after RA erasure.
///
/// Variables are renamed by first occurrence (`v0`, `v1`, …) so kernels
/// compare relational structure up to alpha-renaming. The opaque Rust chain
/// retains exact payload tokens: alpha rewriting arbitrary Rust tokens could
/// capture host names or change fields and local binders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kernel {
    /// `(relation, variables)` per positive atom, in body order.
    pub atoms: Vec<(String, Vec<String>)>,
    /// Guard conditions, in body order.
    pub guards: Vec<String>,
    /// `(bound variables, type annotation, expression)` per binding.
    pub lets: Vec<(Vec<String>, String, String)>,
    /// Relational equalities as canonical token text.
    pub equalities: Vec<String>,
    /// One chain across both opaque Rust kinds, including pattern and type.
    pub chain: Vec<String>,
    /// Relational predecessors of each corresponding Rust chain step.
    pub before: Vec<BTreeSet<String>>,
    /// Head arguments as token text.
    pub head: Vec<String>,
}

impl Kernel {
    pub fn of(module: &Program) -> Kernel {
        Kernel::of_query(&module.query)
    }

    /// Structural sufficient condition for refinement; this is deliberately
    /// incomplete for all identities between relational programs.
    pub fn refines(&self, source: &Self) -> bool {
        let mut atoms = self.atoms.clone();
        let mut source_atoms = source.atoms.clone();
        atoms.sort();
        source_atoms.sort();
        let mut equalities = self.equalities.clone();
        let mut source_equalities = source.equalities.clone();
        equalities.sort();
        source_equalities.sort();
        atoms == source_atoms
            && equalities == source_equalities
            && self.head == source.head
            && self.chain == source.chain
            && self.before.len() == source.before.len()
            && source
                .before
                .iter()
                .zip(&self.before)
                .all(|(earlier, later)| earlier.is_subset(later))
    }

    fn of_query(query: &Query) -> Kernel {
        let mut renaming = Renaming::default();
        // Atom variables are named first, in atom order, so a plan that moves
        // a binding earlier or later than the query wrote it keeps its kernel.
        for item in &query.body {
            if let BodyItem::Positive { atom } = item {
                for variable in atom.variables() {
                    renaming.name(variable);
                }
            }
        }
        let mut kernel = Kernel {
            atoms: Vec::new(),
            guards: Vec::new(),
            lets: Vec::new(),
            equalities: Vec::new(),
            chain: Vec::new(),
            before: Vec::new(),
            head: Vec::new(),
        };
        let mut relational = BTreeSet::new();
        for item in &query.body {
            match item {
                BodyItem::Positive { atom } => {
                    kernel.atoms.push((
                        symbol_name(&atom.relation),
                        atom.variables().map(|v| renaming.name(v)).collect(),
                    ));
                    relational.insert(format!("atom {}", renaming.text(atom.to_token_stream())));
                }
                BodyItem::Equality(equality) => {
                    let text = renaming.text(equality.to_token_stream());
                    kernel.equalities.push(text.clone());
                    relational.insert(format!("eq {text}"));
                }
                BodyItem::Constraint(constraint) => {
                    let text = renaming.text(constraint.to_token_stream());
                    kernel.equalities.push(text.clone());
                    relational.insert(format!("constraint {text}"));
                }
                BodyItem::Filter(filter) => {
                    kernel
                        .guards
                        .push(renaming.text(filter.condition.to_token_stream()));
                    kernel.chain.push(filter.to_token_stream().to_string());
                    kernel.before.push(relational.clone());
                }
                BodyItem::Let(binding) => {
                    let bound = rust::bindings(&binding.pattern)
                        .iter()
                        .map(|v| renaming.name(v))
                        .collect();
                    let annotation = binding
                        .annotation
                        .as_ref()
                        .map(|a| renaming.text(a.ty.to_token_stream()))
                        .unwrap_or_default();
                    kernel.chain.push(binding.to_token_stream().to_string());
                    kernel.before.push(relational.clone());
                    kernel.lets.push((
                        bound,
                        annotation,
                        renaming.text(binding.expression.to_token_stream()),
                    ));
                }
                BodyItem::Negation(_) | BodyItem::Aggregate(_) | BodyItem::Extension { .. } => {}
            }
        }
        kernel.head = query
            .head
            .args
            .iter()
            .map(|a| renaming.text(a.to_token_stream()))
            .collect();
        kernel
    }
}

#[derive(Default)]
struct Renaming {
    names: BTreeMap<String, String>,
}

impl Renaming {
    fn name(&mut self, variable: &Ident) -> String {
        let next = format!("v{}", self.names.len());
        self.names
            .entry(symbol_name(variable))
            .or_insert(next)
            .clone()
    }

    fn text(&self, tokens: TokenStream) -> String {
        fn rewrite(tokens: TokenStream, names: &BTreeMap<String, String>) -> TokenStream {
            tokens
                .into_iter()
                .map(|token| match token {
                    TokenTree::Ident(id) => match names.get(&symbol_name(&id)) {
                        Some(name) => TokenTree::Ident(Ident::new(name, id.span())),
                        None => TokenTree::Ident(id),
                    },
                    TokenTree::Group(g) => {
                        let mut group =
                            proc_macro2::Group::new(g.delimiter(), rewrite(g.stream(), names));
                        group.set_span(g.span());
                        TokenTree::Group(group)
                    }
                    other => other,
                })
                .collect()
        }
        rewrite(tokens, &self.names).to_string()
    }
}
