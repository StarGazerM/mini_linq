//! Relational equality printing. Embedded Rust is emitted verbatim elsewhere.
//! Equalities use the column type's Eq; no Rust expression is inspected.
use cq_lang_structures::syntax::{EqualityValue, algebra::Theta};
use proc_macro2::TokenStream;
use quote::quote;

pub fn relational_condition(theta: &Theta) -> TokenStream {
    let tests = theta.equalities.iter().map(|e| {
        let left = &e.left;
        let right = match &e.right {
            EqualityValue::Variable(v) => quote!(#v),
            EqualityValue::Literal(v) => quote!(&(#v)),
        };
        quote!(::cq_storage::relational_eq(#left, #right))
    });
    quote!(true #(&& #tests)*)
}
