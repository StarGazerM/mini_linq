//! The erasure of a relational plan: the normal-form query it computes.
//!
//! A `rename` of a host relation is an atom whose variables are the rename's
//! targets in declared column order; the same rename (relation and mapping)
//! defined again is the same source occurrence, seen through a `project`
//! that restricts it to the columns one step binds. A `select` is a guard
//! and an `extend` a binding, read back from the `{ let pat: T = e; (v, …) }`
//! block elaboration emits or, for any other value, as `let (columns) = value`.
//! Joins, semijoins, projections and `unit` say how, not what, and erase to
//! nothing; the output head is the query head.
//!
//! Erasure reconstructs atoms, guards and bindings from RA definitions, orders
//! matches by the separately supplied CQ program's body, and appends unmatched
//! reconstructed items in definition order. [`Kernel`] then compares that
//! structural content with variable renaming; this is not a general semantic
//! equivalence checker.

use std::collections::BTreeSet;

use proc_macro2::{TokenStream, TokenTree};
use quote::ToTokens;
use syn::Ident;

use crate::symbol_name;
use cq_lang_ra::{Extend, Operator, Program, Rename};
use cq_lang_vo::cq::{self, normal::Kernel};

/// The kernel a plan computes; equal to the kernel of its query when the
/// plan is an elaboration of a valuation of that query.
pub fn kernel(module: &Program, source: &cq::Program) -> Kernel {
    Kernel::of(&erase(module, source))
}

/// Reconstruct a CQ body and head from RA definitions within a clone of the
/// separately supplied source CQ program.
pub fn erase(module: &Program, source: &cq::Program) -> cq::Program {
    let program = source;
    let mut items: Vec<(String, cq::BodyItem)> = Vec::new();
    let mut occurrences: BTreeSet<String> = BTreeSet::new();
    for definition in &module.plan.definitions {
        let item = match &definition.operator {
            Operator::Rename(rename) => cq::BodyItem::Positive {
                atom: atom(rename, &module.plan.declarations),
            },
            Operator::Select(select) => cq::BodyItem::Filter(cq::Filter {
                if_token: Default::default(),
                condition: select.condition.clone(),
            }),
            Operator::Extend(extend) => cq::BodyItem::Let(Box::new(binding(extend))),
            Operator::Unit(_)
            | Operator::NaturalJoin(_)
            | Operator::Semijoin(_)
            | Operator::Project(_) => continue,
        };
        let key = key(&item);
        if matches!(item, cq::BodyItem::Positive { .. }) && !occurrences.insert(key.clone()) {
            continue;
        }
        items.push((key, item));
    }
    let mut body: Vec<cq::BodyItem> = Vec::new();
    for written in &program.query.body {
        let key = key(written);
        if let Some(index) = items.iter().position(|(k, _)| *k == key) {
            body.push(items.remove(index).1);
        }
    }
    body.extend(items.into_iter().map(|(_, item)| item));
    let mut erased = program.clone();
    erased.query.head = module.plan.output.head.clone();
    erased.query.body = body.into_iter().collect();
    erased
}

/// The atom a rename is: its targets, in the declared order of its sources.
fn atom(rename: &Rename, declarations: &[cq::RelationDecl]) -> cq::Atom {
    let declaration = declarations
        .iter()
        .find(|d| symbol_name(&d.name) == symbol_name(&rename.relation));
    let target = |source: &Ident| {
        rename
            .mapping
            .iter()
            .find(|m| symbol_name(&m.source) == symbol_name(source))
            .map(|m| &m.target)
    };
    let targets: Vec<&Ident> = match declaration {
        Some(declaration)
            if declaration
                .columns
                .iter()
                .all(|c| target(&c.name).is_some()) =>
        {
            declaration
                .columns
                .iter()
                .map(|c| target(&c.name).expect("every column is mapped"))
                .collect()
        }
        _ => rename.mapping.iter().map(|m| &m.target).collect(),
    };
    cq::Atom {
        relation: rename.relation.clone(),
        paren_token: Default::default(),
        args: targets.into_iter().map(path).collect(),
    }
}

/// The binding an extend is: the `let` of its value block when the block
/// returns the extended columns, else all columns bound to the whole value.
fn binding(extend: &Extend) -> cq::Let {
    if let Some(binding) = block_binding(extend) {
        return binding;
    }
    let elems = extend
        .columns
        .iter()
        .map(|column| {
            syn::Pat::Ident(syn::PatIdent {
                attrs: Vec::new(),
                by_ref: None,
                mutability: None,
                ident: column.name.clone(),
                subpat: None,
            })
        })
        .collect();
    cq::Let {
        let_token: Default::default(),
        pattern: syn::Pat::Tuple(syn::PatTuple {
            attrs: Vec::new(),
            paren_token: Default::default(),
            elems,
        }),
        annotation: None,
        eq_token: Default::default(),
        expression: extend.value.clone(),
    }
}

fn block_binding(extend: &Extend) -> Option<cq::Let> {
    let syn::Expr::Block(block) = &extend.value else {
        return None;
    };
    let (syn::Stmt::Local(local), tail) = block.block.stmts.split_first()? else {
        return None;
    };
    let init = local.init.as_ref().filter(|init| init.diverge.is_none())?;
    let returned: Vec<String> = match tail {
        [] => Vec::new(),
        [syn::Stmt::Expr(syn::Expr::Tuple(tuple), None)] => tuple
            .elems
            .iter()
            .map(|e| cq::rust::variable(e).map(symbol_name))
            .collect::<Option<_>>()?,
        _ => return None,
    };
    let columns: Vec<String> = extend
        .columns
        .iter()
        .map(|c| symbol_name(&c.name))
        .collect();
    if returned != columns {
        return None;
    }
    let (pattern, annotation) = match &local.pat {
        syn::Pat::Type(typed) => (
            (*typed.pat).clone(),
            Some(cq::TypeAnnotation {
                colon_token: typed.colon_token,
                ty: (*typed.ty).clone(),
            }),
        ),
        pattern => (pattern.clone(), None),
    };
    Some(cq::Let {
        let_token: local.let_token,
        pattern,
        annotation,
        eq_token: init.eq_token,
        expression: (*init.expr).clone(),
    })
}

fn path(ident: &Ident) -> syn::Expr {
    syn::Expr::Path(syn::ExprPath {
        attrs: Vec::new(),
        qself: None,
        path: ident.clone().into(),
    })
}

/// A body item's text with raw identifiers unrawed: `R(r#a)` and `R(a)` are
/// the same item.
fn key(item: &cq::BodyItem) -> String {
    fn unraw(tokens: TokenStream) -> TokenStream {
        tokens
            .into_iter()
            .map(|token| match token {
                TokenTree::Ident(id) => TokenTree::Ident(Ident::new(&symbol_name(&id), id.span())),
                TokenTree::Group(g) => {
                    TokenTree::Group(proc_macro2::Group::new(g.delimiter(), unraw(g.stream())))
                }
                other => other,
            })
            .collect()
    }
    unraw(item.to_token_stream()).to_string()
}
