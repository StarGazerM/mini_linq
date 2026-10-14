//! Printing central relational predicates; Rust guards are emitted verbatim.
use cq_lang_structures::syntax::{
    EqualityValue,
    algebra::Theta,
    constraint::{Comparison, Constraint, Term},
    symbol_name,
};
use proc_macro2::TokenStream;
use quote::quote;
use std::collections::BTreeMap;
use syn::Ident;

pub fn term_reference(term: &Term, parameters: &BTreeMap<String, Ident>) -> TokenStream {
    match term {
        Term::Variable(v) => quote!(#v),
        Term::Parameter(v) => match parameters.get(&symbol_name(v)) {
            Some(p) => quote!(#p),
            None => quote!(&#v),
        },
        Term::Literal(v) => quote!(&(#v)),
    }
}

pub fn relational_constraint(c: &Constraint, parameters: &BTreeMap<String, Ident>) -> TokenStream {
    let compare = |left: &Term, comparison: Comparison, right: &Term| {
        let l = term_reference(left, parameters);
        let r = term_reference(right, parameters);
        let string = |t: &Term| matches!(t, Term::Literal(l) if matches!(l.as_ref(), syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(_), .. })));
        let string_literal = string(left) || string(right);
        let equality = if string_literal {
            quote!(::cq_storage::relational_string_eq(#l, #r))
        } else {
            quote!(::cq_storage::relational_eq(#l, #r))
        };
        match comparison {
            Comparison::Eq(_) => equality,
            Comparison::Ne(_) => quote!(!#equality),
            _ => {
                let ordering = if string_literal {
                    quote!(::cq_storage::relational_str_cmp(#l, #r))
                } else {
                    quote!(::cq_storage::relational_cmp(#l, #r))
                };
                match comparison {
                    Comparison::Lt(_) => quote!(#ordering.is_lt()),
                    Comparison::Le(_) => quote!(!#ordering.is_gt()),
                    Comparison::Gt(_) => quote!(#ordering.is_gt()),
                    Comparison::Ge(_) => quote!(!#ordering.is_lt()),
                    _ => unreachable!(),
                }
            }
        }
    };
    match c {
        Constraint::Total {
            expression,
            bindings,
        } => {
            let names = bindings.iter().map(|(name, _)| name);
            let values = bindings
                .iter()
                .map(|(_, term)| term_reference(term, parameters));
            quote!({ let (#(#names,)*) = (#(#values,)*); #expression })
        }
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
