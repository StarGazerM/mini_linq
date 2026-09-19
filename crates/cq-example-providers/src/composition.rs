//! Logical binding composition used by the selected RA lowering rules.
//!
//! Providers explain one iterable item. The rule gives that explanation its
//! relation identity and places the binding in the dependent query sequence.

use std::collections::BTreeMap;

use cq_ir::{physical, provider_protocol::exact, rust_access_plan, symbol_name};
use quote::quote;
use syn::{Expr, Ident};

use crate::layout;

pub(crate) fn bind(
    plan: &physical::Module,
    relation: &Ident,
    reply: &exact::IteratorReply,
) -> syn::Result<rust_access_plan::Clause> {
    let heading = layout::heading(plan, relation)?;
    let fields = field_values(&reply.witness, &heading)?;
    let values = heading
        .iter()
        .map(|attribute| &fields[&symbol_name(attribute)]);
    let pattern = &reply.pattern;
    let source = &reply.source;
    syn::parse2(quote! { #relation(#(#values),*) => bind #pattern <- (#source) })
}

pub(crate) fn field_values(
    witness: &exact::Witness,
    heading: &[Ident],
) -> syn::Result<BTreeMap<String, Expr>> {
    let mut values = BTreeMap::new();
    for field in &witness.fields {
        let attribute = heading.get(field.column.index as usize).ok_or_else(|| {
            syn::Error::new_spanned(field, "provider witness field exceeds its logical heading")
        })?;
        if values
            .insert(symbol_name(attribute), ungroup(&field.value).clone())
            .is_some()
        {
            return Err(syn::Error::new_spanned(
                field,
                "provider witness repeats a logical field",
            ));
        }
    }
    if values.len() != heading.len() {
        return Err(syn::Error::new_spanned(
            witness,
            "provider witness must explain every logical field",
        ));
    }
    Ok(values)
}

// macro_rules expression fragments arrive in invisible groups. Parentheses
// and those groups preserve the represented value and must not turn a logical
// variable into an opaque expression in the following Access annotation.
fn ungroup(mut value: &Expr) -> &Expr {
    loop {
        value = match value {
            Expr::Group(group) => &group.expr,
            Expr::Paren(paren) => &paren.expr,
            _ => return value,
        };
    }
}
