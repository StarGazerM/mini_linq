//! Structural erasure of RA prime. Opaque Rust chain nodes remain in their
//! actual definition order; erasure never repairs a reordered target by sorting
//! it against the source. Relational equality and legal earlier movement are
//! established independently by the per-region tableau check.

use std::collections::BTreeSet;

use proc_macro2::{TokenStream, TokenTree};
use quote::ToTokens;
use syn::Ident;

use crate::symbol_name;
use cq_lang_ra::{Operator, Program, Rename};
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
    let mut body = Vec::<cq::BodyItem>::new();
    let mut occurrences = BTreeSet::new();
    for definition in &module.plan.definitions {
        match &definition.operator {
            Operator::Rename(rename) => {
                let item = cq::BodyItem::Positive {
                    atom: atom(rename, &module.plan.declarations),
                };
                if occurrences.insert(key(&item)) {
                    body.push(item);
                }
            }
            Operator::Select(select) => body.extend(
                select
                    .condition
                    .equalities
                    .iter()
                    .cloned()
                    .map(cq::BodyItem::Equality),
            ),
            Operator::Guard(guard) => body.push(cq::BodyItem::Filter(cq::Filter {
                if_token: guard.if_token,
                condition: guard.condition.clone(),
            })),
            Operator::Bind(bind) => body.push(cq::BodyItem::Let(Box::new(bind.binding()))),
            Operator::Unit(_)
            | Operator::NaturalJoin(_)
            | Operator::Semijoin(_)
            | Operator::Project(_) => {}
        }
    }
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
