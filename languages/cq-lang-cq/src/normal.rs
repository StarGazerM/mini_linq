//! CQ normal form: distinct atom variables/literals and variable-only heads.
//! Rust expressions remain unchanged and are opaque, pure and total by premise.
//! Only explicit `where` predicates and atom restrictions are relational.
//!
//! DESIGN contract7-normal-form
//! Owns CQ normalization, preserving `Constraint` operands `Term::Closed(syn::Expr)` unchanged,
//! and owns the one decision that a query-closed `let`/`if`/head operand is a host-once
//! capture (A-Capture, A-CaptureRef); later IRs carry `captures` unchanged.
//! Contract 7, CQ-Closed/WF-NF; source: doc/PLANNING-REDESIGN.md Contract and Architecture.
//! Source: doc/formal/CONFORMANCE.md CQ-Where/CQ-Closed; the shared Term lives in cq-syntax.
//! DO NOT turn closed Rust into recognized arithmetic, relational facts, or type-directed rewrites.
//! Atom restrictions and explicit `where` are relational; computed Rust remains total by premise.

use super::{Atom, BodyItem, Equality, EqualityValue, Let, Program, rust};
use crate::symbol_name;
use proc_macro2::{Span, TokenStream, TokenTree};
use quote::{ToTokens, format_ident, quote};
use std::collections::{BTreeMap, BTreeSet};
use syn::Ident;

/// Formal rule WF-NF: Check CQ, distinct variable or literal atom arguments, variable-only
/// output bound by the body or a capture, no query-closed top-level `let`/`if` (A-Capture),
/// and translated code-domain operands (WF-Domain).
pub fn check(module: &Program) -> syn::Result<()> {
    super::contract::check(module)?;
    for item in super::blocks::walk(&module.query.body) {
        match item {
            BodyItem::Positive { atom } if atom.is_normalized() => {}
            BodyItem::Positive { atom } => {
                return Err(syn::Error::new_spanned(
                    atom,
                    format!(
                        "normal form: `{}` atom arguments are distinct variables or literals",
                        atom.relation
                    ),
                ));
            }
            BodyItem::Equality(equality) => equality.check()?,
            BodyItem::Constraint(constraint) => constraint.check()?,
            BodyItem::Negation(_) => {}
            BodyItem::Filter(_) | BodyItem::Let(_) | BodyItem::For(_) | BodyItem::Block(_) => {}
            BodyItem::Aggregate(_) | BodyItem::Extension { .. } => {
                return Err(syn::Error::new_spanned(
                    item,
                    "normal form covers relational atoms and negation, guards and bindings",
                ));
            }
        }
    }
    if let Some((_, item)) = module
        .query
        .body
        .iter()
        .enumerate()
        .find(|(index, item)| capturable(item, &module.query.body, *index))
    {
        return Err(syn::Error::new_spanned(
            item,
            "normal form: a query-closed binding or guard is a capture",
        ));
    }
    let logical = super::blocks::bound(&module.query.body)
        .into_keys()
        .collect();
    if let Some(item) = closed_branch_item(&module.query.body, &logical) {
        return Err(syn::Error::new_spanned(
            item,
            "normal form: a query-closed branch binding or guard reads a capture",
        ));
    }
    for item in super::blocks::walk(&module.query.body) {
        let (reference, expression) = match item {
            BodyItem::Let(binding) => (binding.capture.as_ref(), &binding.expression),
            BodyItem::Filter(guard) => (guard.capture.as_ref(), &guard.condition),
            _ => continue,
        };
        cq_syntax::capture::check_reference(reference, expression, &module.query.captures)?;
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
    let mut bound: BTreeSet<_> = super::blocks::bound(&module.query.body)
        .into_keys()
        .collect();
    if let Some(group) = &module.query.group {
        bound.extend(
            group
                .aggregates
                .iter()
                .map(|aggregate| symbol_name(&aggregate.name)),
        );
    }
    bound.extend(module.query.captures.names());
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
    super::domain::check(module)
}

/// Canonicalize at each atom's written position.
/// Formal rule A-Variable: Keep the first occurrence within this atom, including shared query names.
/// Formal rule A-Repeat: Replace only a variable repeated within one atom and append identity.
/// Formal rule A-Wildcard: Replace a wildcard with an unused witness variable.
/// Formal rule A-Prefix: Place an opaque binding immediately before its atom.
/// Formal rule A-Closed: Keep a query-closed atom operand as a relational restriction.
/// Formal rule A-Local: Place a dependent binding and relational equality after its atom.
/// Formal rule A-Capture: Record a query-closed `let`/`if` or head operand once as a host capture.
/// Formal rule A-Domain: Translate a closed operand compared with a code-domain column once on the host.
pub fn normalize(module: &Program) -> syn::Result<Program> {
    super::contract::check(module)?;
    let mut normal = module.clone();
    cq_syntax::intern_domains(&mut normal.declarations);
    let declarations: BTreeMap<_, _> = normal
        .declarations
        .iter()
        .map(|d| (symbol_name(&d.name), d))
        .collect();
    let mut names = reserved(module);
    if module.query.group.is_some() {
        super::domain::translate(
            &mut normal.query.body,
            &declarations,
            &mut normal.query.captures,
            &mut names,
        )?;
        check(&normal)?;
        return Ok(normal);
    }
    hoist(&mut normal.query);
    let mut bound = BTreeSet::new();
    let logical: BTreeSet<String> = super::blocks::bound(&normal.query.body)
        .into_keys()
        .collect();
    let captured = normal.query.captures.names();
    let closed = |expression: &syn::Expr| {
        rust::free_variables(expression)
            .dependencies(&logical)
            .is_empty()
    };
    let mut branch_captures = Vec::new();
    let body = normalize_body(
        &normal.query.body,
        &declarations,
        &logical,
        &mut names,
        &mut bound,
        &mut branch_captures,
    )?;
    normal.query.captures.items.extend(branch_captures);
    let mut translated: super::CommaList<BodyItem> = body.into_iter().collect();
    super::domain::translate(
        &mut translated,
        &declarations,
        &mut normal.query.captures,
        &mut names,
    )?;
    let mut body: Vec<BodyItem> = translated.into_iter().collect();
    let grouped = module
        .query
        .head
        .args
        .iter()
        .map(super::group::aggregate)
        .collect::<syn::Result<Vec<_>>>()?
        .iter()
        .any(Option::is_some);
    if grouped {
        let mut valuation: BTreeMap<_, _> = super::blocks::bound(&normal.query.body);
        let mut keys = Vec::new();
        let mut key_names = BTreeSet::new();
        let mut aggregates = Vec::new();
        let mut captures = Vec::new();
        for (position, argument) in normal.query.head.args.iter_mut().enumerate() {
            let Some((kind, expression)) = super::group::aggregate(argument)? else {
                let variable = rust::variable(argument).expect("group key checked");
                if key_names.insert(symbol_name(variable)) {
                    keys.push(variable.clone());
                }
                continue;
            };
            let name = fresh(&mut names, module.query.body.len(), position);
            let ty = module.output().expect("aggregate output checked").columns[position]
                .ty
                .clone();
            let mut aggregate = super::syntax::algebra::AggregateBinding {
                name: name.clone(),
                colon_token: Default::default(),
                ty,
                eq_token: Default::default(),
                kind,
                paren_token: Default::default(),
                expression: expression.cloned(),
                semi_token: Default::default(),
            };
            if let Some(expression) = &aggregate.expression {
                if !rust::variable(expression).is_some_and(|v| {
                    bound.contains(&symbol_name(v)) || captured.contains(&symbol_name(v))
                }) {
                    let value = fresh(&mut names, module.query.body.len(), position);
                    let item = binding(&value, &syn::parse_quote!(_), expression.clone());
                    if closed(expression) {
                        captures.push(item);
                    } else {
                        body.push(item);
                        valuation.insert(symbol_name(&value), value.clone());
                    }
                    aggregate.expression = Some(path(&value));
                }
            }
            aggregates.push(aggregate);
            *argument = path(&name);
        }
        normal
            .query
            .captures
            .items
            .extend(captures.into_iter().map(capture));
        normal.query.group = Some(super::group::HeadGroup {
            group_token: Default::default(),
            valuation_bracket: Default::default(),
            valuation: valuation.into_values().collect(),
            by_token: Default::default(),
            keys_bracket: Default::default(),
            keys: keys.into_iter().collect(),
            brace_token: Default::default(),
            aggregates,
        });
        normal.query.body = body.into_iter().collect();
        check(&normal)?;
        return Ok(normal);
    }
    let mut captures = Vec::new();
    for (position, argument) in normal.query.head.args.iter_mut().enumerate() {
        if rust::variable(argument).is_some_and(|variable| {
            bound.contains(&symbol_name(variable)) || captured.contains(&symbol_name(variable))
        }) {
            continue;
        }
        let key = fresh(&mut names, module.query.body.len(), position);
        let column = &module
            .output()
            .expect("computed output types checked")
            .columns[position];
        let item = binding(&key, &column.ty, argument.clone());
        if closed(argument) {
            captures.push(item);
        } else {
            body.push(item);
        }
        *argument = path(&key);
    }
    normal
        .query
        .captures
        .items
        .extend(captures.into_iter().map(capture));
    normal.query.body = body.into_iter().collect();
    solve_aliases(&mut normal.query);
    check(&normal)?;
    Ok(normal)
}

/// Formal rule A-Alias: Solve a relational equality between two atom generators by renaming one to the other, unless a block occurs, either name is protected, or an atom would repeat a name.
///
/// Protected names are the valuation (head) variables, negated atom columns,
/// Rust binders and the generator inputs of every Rust expression: their
/// spelling is observed outside relational algebra. The solved equality
/// becomes a shared join name and is dropped; every other occurrence of the
/// replaced name in atoms and relational predicates is renamed. The result is
/// recorded only as plain shared names in the normal form.
fn solve_aliases(query: &mut super::Query) {
    use super::syntax::constraint::{Constraint, Term};
    if super::blocks::walk(&query.body)
        .iter()
        .any(|item| matches!(item, BodyItem::Block(_)))
    {
        return;
    }
    let atoms: Vec<&Atom> = query.body.iter().filter_map(BodyItem::atom).collect();
    let let_bound: BTreeSet<String> = query
        .body
        .iter()
        .filter_map(|item| match item {
            BodyItem::Let(binding) => Some(rust::bindings(&binding.pattern)),
            BodyItem::For(binding) => Some(rust::bindings(&binding.pattern)),
            _ => None,
        })
        .flatten()
        .map(|v| symbol_name(&v))
        .collect();
    let generators: BTreeSet<String> = atoms
        .iter()
        .flat_map(|atom| atom.variables().map(symbol_name))
        .filter(|v| !let_bound.contains(v))
        .collect();
    let mut protected: BTreeSet<String> = query.valuation_variables().map(symbol_name).collect();
    for item in &query.body {
        let expression = match item {
            BodyItem::Negation(negation) => {
                protected.extend(negation.atom.variables().map(symbol_name));
                continue;
            }
            BodyItem::Filter(filter) => &filter.condition,
            BodyItem::Let(binding) => {
                protected.extend(rust::bindings(&binding.pattern).iter().map(symbol_name));
                &binding.expression
            }
            BodyItem::For(binding) => {
                protected.extend(rust::bindings(&binding.pattern).iter().map(symbol_name));
                &binding.expression
            }
            _ => continue,
        };
        protected.extend(rust::free_variables(expression).dependencies(&generators));
    }
    let canonical = |id: &Ident, aliases: &BTreeMap<String, Ident>| {
        aliases.get(&symbol_name(id)).unwrap_or(id).clone()
    };
    let mut aliases = BTreeMap::<String, Ident>::new();
    for equality in query.body.iter().flat_map(|item| match item {
        BodyItem::Equality(equality) => vec![equality.clone()],
        BodyItem::Constraint(constraint) => constraint.equalities(),
        _ => Vec::new(),
    }) {
        let EqualityValue::Variable(right) = &equality.right else {
            continue;
        };
        if !generators.contains(&symbol_name(&equality.left))
            || !generators.contains(&symbol_name(right))
        {
            continue;
        }
        let left = canonical(&equality.left, &aliases);
        let right = canonical(right, &aliases);
        if left == right {
            continue;
        }
        let (keep, replace) = if protected.contains(&symbol_name(&right)) {
            (right, left)
        } else {
            (left, right)
        };
        if protected.contains(&symbol_name(&replace)) {
            continue;
        }
        let mut proposed = aliases.clone();
        for value in proposed.values_mut() {
            if symbol_name(value) == symbol_name(&replace) {
                *value = keep.clone();
            }
        }
        proposed.insert(symbol_name(&replace), keep);
        if atoms.iter().any(|atom| {
            let names: BTreeSet<_> = atom
                .variables()
                .map(|v| symbol_name(&canonical(v, &proposed)))
                .collect();
            names.len() != atom.variables().count()
        }) {
            continue;
        }
        aliases = proposed;
    }
    if aliases.is_empty() {
        return;
    }
    let rename = |id: &mut Ident| {
        if let Some(target) = aliases.get(&symbol_name(id)) {
            *id = target.clone();
        }
    };
    let trivial = |constraint: &Constraint| {
        constraint.equality().is_some_and(|e| {
            matches!(&e.right, EqualityValue::Variable(v) if symbol_name(v) == symbol_name(&e.left))
        })
    };
    let body = std::mem::take(&mut query.body);
    query.body = body
        .into_iter()
        .filter_map(|mut item| {
            match &mut item {
                BodyItem::Positive { atom } => {
                    for argument in atom.args.iter_mut() {
                        if let Some(target) = rust::variable(argument)
                            .and_then(|variable| aliases.get(&symbol_name(variable)))
                        {
                            *argument = path(target);
                        }
                    }
                }
                BodyItem::Equality(equality) => {
                    rename(&mut equality.left);
                    if let EqualityValue::Variable(right) = &mut equality.right {
                        rename(right);
                        if symbol_name(right) == symbol_name(&equality.left) {
                            return None;
                        }
                    }
                }
                BodyItem::Constraint(constraint) => {
                    constraint.map_terms(&mut |term| {
                        if let Term::Variable(variable) = term {
                            rename(variable);
                        }
                    });
                    if trivial(constraint) {
                        return None;
                    }
                    if let Constraint::Or(branches) = constraint {
                        if let [conjunction] = branches.as_mut_slice() {
                            conjunction.retain(|conjunct| !trivial(conjunct));
                            if conjunction.is_empty() {
                                return None;
                            }
                        }
                    }
                }
                _ => (),
            }
            Some(item)
        })
        .collect();
}

pub(super) fn capture(item: BodyItem) -> cq_syntax::Capture {
    match item {
        BodyItem::Let(binding) => cq_syntax::Capture::Let(binding),
        BodyItem::Filter(guard) => cq_syntax::Capture::Guard(guard),
        _ => unreachable!("only bindings and guards are captured"),
    }
}

/// A-Capture: a top-level `let` or `if` is a capture when its
/// expression reads no query variable and its binders are bound nowhere else.
/// Captures are found in dependency order; references to them then resolve
/// like any host name (`where` operands become closed terms).
fn capturable(item: &BodyItem, body: &super::CommaList<BodyItem>, position: usize) -> bool {
    let logical: BTreeSet<_> = super::blocks::bound(body).into_keys().collect();
    let closed = |expression: &syn::Expr| {
        rust::free_variables(expression)
            .dependencies(&logical)
            .is_empty()
    };
    match item {
        BodyItem::Filter(guard) => closed(&guard.condition),
        BodyItem::Let(binding) => {
            let others: super::CommaList<BodyItem> = body
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != position)
                .map(|(_, item)| item.clone())
                .collect();
            let elsewhere = super::blocks::bound(&others);
            closed(&binding.expression)
                && rust::bindings(&binding.pattern)
                    .iter()
                    .all(|binder| !elsewhere.contains_key(&symbol_name(binder)))
        }
        _ => false,
    }
}

/// A-Capture: positions of the top-level body items that are
/// captures, in dependency order. Normalization records them once; reference
/// interpreters evaluate them before their loops.
pub fn capture_positions(body: &super::CommaList<BodyItem>) -> Vec<usize> {
    let mut remaining: Vec<(usize, BodyItem)> = body.iter().cloned().enumerate().collect();
    let mut positions = Vec::new();
    loop {
        let current: super::CommaList<BodyItem> =
            remaining.iter().map(|(_, item)| item.clone()).collect();
        let Some(index) = current
            .iter()
            .enumerate()
            .position(|(index, item)| capturable(item, &current, index))
        else {
            return positions;
        };
        positions.push(remaining.remove(index).0);
    }
}

fn hoist(query: &mut super::Query) {
    let positions = capture_positions(&query.body);
    let mut items: Vec<Option<BodyItem>> = std::mem::take(&mut query.body)
        .into_iter()
        .map(Some)
        .collect();
    for position in positions {
        let item = items[position]
            .take()
            .expect("each position is captured once");
        query.captures.items.push(capture(item));
    }
    query.body = items.into_iter().flatten().collect();
    let logical = super::blocks::bound(&query.body).into_keys().collect();
    super::blocks::resolve_constraints(&mut query.body, &logical, &query.captures.names());
}

/// Formal rule A-CaptureRef: a query-closed branch `let`/`if` is
/// evaluated once as a fresh query capture; the branch keeps its binder (or
/// guard) as a capture reference, so union interfaces are unchanged.
fn hoist_branch(
    body: Vec<BodyItem>,
    logical: &BTreeSet<String>,
    names: &mut BTreeSet<String>,
    captures: &mut Vec<cq_syntax::Capture>,
) -> Vec<BodyItem> {
    let closed = |expression: &syn::Expr| {
        rust::free_variables(expression)
            .dependencies(logical)
            .is_empty()
    };
    body.into_iter()
        .map(|item| match item {
            BodyItem::Let(mut binding)
                if binding.capture.is_none() && closed(&binding.expression) =>
            {
                let name = fresh_capture(names);
                let annotation = match &binding.pattern {
                    syn::Pat::Ident(pattern) if pattern.by_ref.is_none() => {
                        binding.annotation.clone()
                    }
                    _ => None,
                };
                let value = std::mem::replace(&mut binding.expression, path(&name));
                captures.push(cq_syntax::Capture::Let(Box::new(Let {
                    pattern: ident_pattern(&name),
                    annotation,
                    expression: value,
                    ..(*binding).clone()
                })));
                binding.capture = Some(Default::default());
                BodyItem::Let(binding)
            }
            BodyItem::Filter(mut guard) if guard.capture.is_none() && closed(&guard.condition) => {
                let name = fresh_capture(names);
                let value = std::mem::replace(&mut guard.condition, path(&name));
                let BodyItem::Let(binding) = binding(&name, &syn::parse_quote!(bool), value) else {
                    unreachable!("binding builds a let")
                };
                captures.push(cq_syntax::Capture::Let(binding));
                guard.capture = Some(Default::default());
                BodyItem::Filter(guard)
            }
            item => item,
        })
        .collect()
}

/// WF-NF: the first branch `let`/`if` that A-Capture would still hoist.
fn closed_branch_item<'a>(
    body: &'a super::CommaList<BodyItem>,
    logical: &BTreeSet<String>,
) -> Option<&'a BodyItem> {
    body.iter().find_map(|item| {
        let BodyItem::Block(block) = item else {
            return None;
        };
        block.branches.iter().find_map(|branch| {
            let logical: BTreeSet<String> = logical
                .iter()
                .cloned()
                .chain(super::blocks::bound(&branch.body).into_keys())
                .collect();
            let closed = |expression: &syn::Expr| {
                rust::free_variables(expression)
                    .dependencies(&logical)
                    .is_empty()
            };
            branch
                .body
                .iter()
                .find(|item| match item {
                    BodyItem::Let(binding) => {
                        binding.capture.is_none() && closed(&binding.expression)
                    }
                    BodyItem::Filter(guard) => guard.capture.is_none() && closed(&guard.condition),
                    _ => false,
                })
                .or_else(|| closed_branch_item(&branch.body, &logical))
        })
    })
}

fn normalize_body(
    source: &super::CommaList<BodyItem>,
    declarations: &BTreeMap<String, &super::RelationDecl>,
    logical: &BTreeSet<String>,
    names: &mut BTreeSet<String>,
    bound: &mut BTreeSet<String>,
    captures: &mut Vec<cq_syntax::Capture>,
) -> syn::Result<Vec<BodyItem>> {
    let mut body = Vec::new();
    for (clause, item) in source.iter().enumerate() {
        match item {
            BodyItem::Block(block) => {
                let mut block = block.clone();
                for branch in &mut block.branches {
                    let mut local = bound.clone();
                    let branch_logical = logical
                        .iter()
                        .cloned()
                        .chain(super::blocks::bound(&branch.body).into_keys())
                        .collect();
                    let normal = normalize_body(
                        &branch.body,
                        declarations,
                        &branch_logical,
                        names,
                        &mut local,
                        captures,
                    )?;
                    branch.body = hoist_branch(normal, &branch_logical, names, captures)
                        .into_iter()
                        .collect();
                }
                bound.extend(block.analyze(bound).interface.into_keys());
                body.push(BodyItem::Block(block));
            }

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
                            let key = fresh(names, clause, position);
                            after.push(equality(
                                variable.clone(),
                                EqualityValue::Variable(key.clone()),
                            ));
                            args.push(path(&key));
                        }
                        continue;
                    }
                    if super::syntax::equality::literal(argument) {
                        args.push(argument.clone());
                        continue;
                    }
                    let key = fresh(names, clause, position);
                    if matches!(argument, syn::Expr::Infer(_)) {
                        // A wildcard binds an unconstrained, unused witness.
                    } else if rust::free_variables(argument)
                        .dependencies(logical)
                        .is_empty()
                    {
                        // This is an atom restriction, not row computation.
                        // Preserve its Rust expression as a once-bound host operand.
                        after.push(BodyItem::Constraint(
                            super::syntax::constraint::Constraint::Compare {
                                left: super::syntax::constraint::Term::Variable(key.clone()),
                                comparison: super::syntax::constraint::Comparison::Eq(
                                    Default::default(),
                                ),
                                right: super::syntax::constraint::Term::Closed(argument.clone()),
                            },
                        ));
                    } else if rust::free_variables(argument)
                        .dependencies(&local)
                        .is_empty()
                    {
                        body.push(binding(&key, &column.ty, argument.clone()));
                    } else {
                        let value = fresh(names, clause, position);
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
            BodyItem::Filter(_)
            | BodyItem::Equality(_)
            | BodyItem::Constraint(_)
            | BodyItem::Negation(_) => body.push(item.clone()),
            BodyItem::Let(binding) => {
                bound.extend(rust::bindings(&binding.pattern).iter().map(symbol_name));
                body.push(item.clone());
            }
            BodyItem::For(binding) => {
                bound.extend(rust::bindings(&binding.pattern).iter().map(symbol_name));
                body.push(item.clone());
            }
            BodyItem::Aggregate(_) | BodyItem::Extension { .. } => {
                return Err(syn::Error::new_spanned(
                    item,
                    "normal form covers relational atoms and negation, guards and bindings",
                ));
            }
        }
    }
    Ok(body)
}

fn equality(left: Ident, right: EqualityValue) -> BodyItem {
    BodyItem::Equality(Equality {
        left,
        eq_token: Default::default(),
        right,
    })
}

pub(super) fn binding(key: &Ident, ty: &syn::Type, expression: syn::Expr) -> BodyItem {
    BodyItem::Let(Box::new(Let {
        let_token: Default::default(),
        pattern: ident_pattern(key),
        annotation: Some(super::TypeAnnotation {
            colon_token: Default::default(),
            ty: ty.clone(),
        }),
        eq_token: Default::default(),
        capture: None,
        expression,
    }))
}

fn ident_pattern(key: &Ident) -> syn::Pat {
    syn::Pat::Ident(syn::PatIdent {
        attrs: Vec::new(),
        by_ref: None,
        mutability: None,
        ident: key.clone(),
        subpat: None,
    })
}

pub(super) fn path(ident: &Ident) -> syn::Expr {
    syn::parse2(quote!(#ident)).expect("identifier expression")
}

/// `__cq_c_<n>`, the first unused.
pub(super) fn fresh_capture(names: &mut BTreeSet<String>) -> Ident {
    let name = (0..)
        .map(|n| format!("__cq_c_{n}"))
        .find(|name| !names.contains(name))
        .expect("an unused capture name");
    names.insert(name.clone());
    format_ident!("{name}", span = Span::mixed_site())
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
pub(super) fn fresh(names: &mut BTreeSet<String>, clause: usize, argument: usize) -> Ident {
    let mut name = format!("__cq_{clause}_{argument}");
    while !names.insert(name.clone()) {
        name.push('_');
    }
    format_ident!("{name}", span = Span::mixed_site())
}
