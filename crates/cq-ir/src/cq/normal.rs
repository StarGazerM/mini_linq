//! CQ-NF: the kernel form every plan erases to.
//!
//! A normal query has one variable per atom argument, distinct within the
//! atom; constants, repeated variables, wildcards and expression arguments
//! have been moved out of atoms into `let` bindings and `if` guards. That
//! makes every atom a relation applied to a tuple of distinct variables, so
//! each later language can be checked against this one by *erasure*: the
//! [`Kernel`] of a plan is its atoms, guards, bindings and head, and two
//! programs agree iff their kernels agree up to renaming of variables.
//!
//! [`normalize`] is the canonicalization: it records no decision. Where a
//! moved constraint is placed follows the source clause it came from — a
//! constant's binding goes before every atom (it is a join key), an
//! equality or expression guard right after its atom — and the order pass
//! may still move guards and bindings earlier, never later than their
//! variables.

use std::collections::{BTreeMap, BTreeSet};

use proc_macro2::{Span, TokenStream, TokenTree};
use quote::{ToTokens, format_ident, quote};
use syn::Ident;

use super::{Atom, BodyItem, Filter, Let, Module, Query, rust};
use crate::symbol_name;

/// Whether a module is in normal form.
pub fn check(module: &Module) -> syn::Result<()> {
    super::contract::check(module)?;
    let query = &module.program.query;
    for item in &query.body {
        match item {
            BodyItem::Positive { atom } if atom.is_normalized() => {}
            BodyItem::Positive { atom } => {
                return Err(syn::Error::new_spanned(
                    atom,
                    "normal form: atom arguments are distinct variables",
                ));
            }
            BodyItem::Filter(_) | BodyItem::Let(_) => {}
            BodyItem::Negation(_) | BodyItem::Aggregate(_) => {
                return Err(syn::Error::new_spanned(
                    item,
                    "normal form covers positive queries with guards and bindings",
                ));
            }
        }
    }
    if !query.head.is_normalized() && query.head.variables().count() == query.head.args.len() {
        return Err(syn::Error::new_spanned(
            &query.head,
            "normal form: head variables are distinct",
        ));
    }
    Ok(())
}

/// Canonicalize a positive query into normal form.
pub fn normalize(module: &Module) -> syn::Result<Module> {
    super::contract::check(module)?;
    let program = &module.program;
    let declarations: BTreeMap<String, &super::RelationDecl> = program
        .inputs()
        .map(|d| (symbol_name(&d.name), d))
        .collect();
    let mut names = reserved(module);
    let mut prefix: Vec<BodyItem> = Vec::new();
    let mut body: Vec<BodyItem> = Vec::new();
    let mut bound: BTreeSet<String> = BTreeSet::new();
    for (clause, item) in program.query.body.iter().enumerate() {
        match item {
            BodyItem::Positive { atom } => {
                let declaration = declarations[&symbol_name(&atom.relation)];
                let mut seen: BTreeSet<String> = BTreeSet::new();
                let mut args: Vec<syn::Expr> = Vec::new();
                let mut after: Vec<BodyItem> = Vec::new();
                let mut before: Vec<BodyItem> = Vec::new();
                for (position, (argument, column)) in
                    atom.args.iter().zip(&declaration.columns).enumerate()
                {
                    let ty = &column.ty;
                    if let Some(variable) = rust::variable(argument) {
                        if seen.insert(symbol_name(variable)) {
                            args.push(argument.clone());
                        } else {
                            let key = fresh(&mut names, clause, position);
                            after.push(guard(quote!(#key == #variable)));
                            args.push(path(&key));
                        }
                        continue;
                    }
                    let key = fresh(&mut names, clause, position);
                    if matches!(argument, syn::Expr::Infer(_)) {
                        // A wildcard is a distinct witness column.
                    } else if rust::literal(argument) {
                        prefix.push(binding(&key, ty, quote!(#argument)));
                    } else if rust::free_variables(argument)
                        .dependencies(&seen.difference(&bound).cloned().collect())
                        .is_empty()
                    {
                        before.push(binding(&key, ty, quote!(#argument)));
                    } else {
                        after.push(guard(quote!(#key.eq(&(#argument)))));
                    }
                    args.push(path(&key));
                }
                body.extend(before);
                body.push(BodyItem::Positive {
                    atom: Atom {
                        relation: atom.relation.clone(),
                        paren_token: atom.paren_token,
                        args: args.into_iter().collect(),
                    },
                });
                bound.extend(seen);
                body.extend(after);
            }
            BodyItem::Filter(_) | BodyItem::Let(_) => body.push(item.clone()),
            BodyItem::Negation(_) | BodyItem::Aggregate(_) => {
                return Err(syn::Error::new_spanned(
                    item,
                    "normal form covers positive queries with guards and bindings",
                ));
            }
        }
        if let BodyItem::Let(binding) = item {
            bound.extend(rust::bindings(&binding.pattern).iter().map(symbol_name));
        }
    }
    prefix.extend(body);
    let mut normal = module.clone();
    normal.program.query.body = prefix.into_iter().collect();
    check(&normal)?;
    Ok(normal)
}

fn guard(condition: TokenStream) -> BodyItem {
    BodyItem::Filter(Filter {
        if_token: Default::default(),
        condition: syn::parse2(condition).expect("a guard expression"),
    })
}

fn binding(key: &Ident, ty: &syn::Type, expression: TokenStream) -> BodyItem {
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
        expression: syn::parse2(expression).expect("an expression"),
    }))
}

fn path(ident: &Ident) -> syn::Expr {
    syn::parse2(quote!(#ident)).expect("an identifier expression")
}

/// Every identifier the module mentions, so fresh names never capture.
pub fn reserved(module: &Module) -> BTreeSet<String> {
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
pub fn fresh(names: &mut BTreeSet<String>, clause: usize, argument: usize) -> Ident {
    let mut name = format!("__cq_{clause}_{argument}");
    while !names.insert(name.clone()) {
        name.push('_');
    }
    format_ident!("{name}", span = Span::call_site())
}

/// The erasure of a plan: what it computes, independent of how.
///
/// Variables are renamed by first occurrence (`v0`, `v1`, …) so kernels
/// compare up to α-equivalence; embedded expressions are compared as token
/// text after that renaming.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kernel {
    /// `(relation, variables)` per positive atom, in body order.
    pub atoms: Vec<(String, Vec<String>)>,
    /// Guard conditions, in body order.
    pub guards: Vec<String>,
    /// `(bound variables, type annotation, expression)` per binding.
    pub lets: Vec<(Vec<String>, String, String)>,
    /// Head arguments as token text.
    pub head: Vec<String>,
}

impl Kernel {
    pub fn of(module: &Module) -> Kernel {
        Kernel::of_query(&module.program.query)
    }

    pub fn of_query(query: &Query) -> Kernel {
        let mut renaming = Renaming::default();
        let mut kernel = Kernel {
            atoms: Vec::new(),
            guards: Vec::new(),
            lets: Vec::new(),
            head: Vec::new(),
        };
        for item in &query.body {
            match item {
                BodyItem::Positive { atom } => kernel.atoms.push((
                    symbol_name(&atom.relation),
                    atom.variables().map(|v| renaming.name(v)).collect(),
                )),
                BodyItem::Filter(filter) => kernel
                    .guards
                    .push(renaming.text(filter.condition.to_token_stream())),
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
                    kernel.lets.push((
                        bound,
                        annotation,
                        renaming.text(binding.expression.to_token_stream()),
                    ));
                }
                BodyItem::Negation(_) | BodyItem::Aggregate(_) => {}
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
