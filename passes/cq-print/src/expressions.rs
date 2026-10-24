//! Printing central relational predicates; Rust guards are emitted verbatim.
//! Only language-recognized comparisons and Boolean combinations reach this
//! constraint printer; arbitrary host predicates remain opaque Rust guards.
use cq_lang_structures::syntax::{
    EqualityValue,
    algebra::Theta,
    constraint::{Comparison, Constraint, Term},
};
use proc_macro2::TokenStream;
use quote::quote;
use std::collections::BTreeMap;
use syn::Ident;

pub fn term_reference(term: &Term, parameters: &BTreeMap<String, Ident>) -> TokenStream {
    term.reference(parameters)
}

pub fn relational_constraint(c: &Constraint, parameters: &BTreeMap<String, Ident>) -> TokenStream {
    let compare = |left: &Term, comparison: Comparison, right: &Term| {
        let l = term_reference(left, parameters);
        let r = term_reference(right, parameters);
        comparison.expression(l, r, left.string_literal() || right.string_literal())
    };
    match c {
        Constraint::Compare {
            left,
            comparison,
            right,
        } => compare(left, *comparison, right),
        Constraint::Between { term, low, high } => {
            let l = compare(term, Comparison::Ge(Default::default()), low);
            let h = compare(term, Comparison::Le(Default::default()), high);
            quote!(#l && #h)
        }
        Constraint::In { term, literals } => {
            let tests = literals.iter().map(|literal| {
                compare(
                    term,
                    Comparison::Eq(Default::default()),
                    &Term::Literal(Box::new(literal.clone())),
                )
            });
            quote!(false #(|| #tests)*)
        }
        Constraint::Or(branches) => {
            let branches = branches.iter().map(|branch| {
                let tests = branch.iter().map(|c| relational_constraint(c, parameters));
                quote!((true #(&& (#tests))*))
            });
            quote!(false #(|| #branches)*)
        }
    }
}

pub fn relational_condition_with(
    theta: &Theta,
    parameters: &BTreeMap<String, Ident>,
) -> TokenStream {
    let tests = theta
        .equalities
        .iter()
        .map(|e| {
            let left = &e.left;
            let right = match &e.right {
                EqualityValue::Variable(v) => quote!(#v),
                EqualityValue::Literal(v) => quote!(&(#v)),
            };
            if matches!(
                &e.right,
                EqualityValue::Literal(syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(_),
                    ..
                }))
            ) {
                quote!(::cq_storage::relational_string_eq(#left, #right))
            } else {
                quote!(::cq_storage::relational_eq(#left, #right))
            }
        })
        .chain(theta.constraints.iter().map(|c| {
            let test = relational_constraint(c, parameters);
            quote!((#test))
        }));
    quote!(true #(&& #tests)*)
}
