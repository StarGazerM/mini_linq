//! Read-only lookups of checked plan metadata and established representations.

use crate::{physical, provider_protocol as protocol, relational_plan, symbol_name};
use syn::{Expr, Ident, Path};

pub fn definition<'a>(
    plan: &'a physical::Module,
    name: &Ident,
) -> syn::Result<&'a relational_plan::Definition> {
    plan.index_requirements
        .relational_plan
        .plan
        .definitions
        .iter()
        .find(|d| symbol_name(&d.result) == symbol_name(name))
        .ok_or_else(|| syn::Error::new_spanned(name, "expected an RA definition"))
}

pub fn relation_provider<'a>(plan: &'a physical::Module, name: &Ident) -> syn::Result<&'a Path> {
    implementation(&plan.assignments.relations, name)
}

pub fn index_provider<'a>(plan: &'a physical::Module, name: &Ident) -> syn::Result<&'a Path> {
    implementation(&plan.assignments.indexes, name)
}

fn implementation<'a>(
    bindings: &'a crate::SemicolonList<physical::Binding>,
    name: &Ident,
) -> syn::Result<&'a Path> {
    bindings
        .iter()
        .find(|b| symbol_name(&b.name) == symbol_name(name))
        .map(|b| &b.implementation)
        .ok_or_else(|| syn::Error::new_spanned(name, "missing recorded provider assignment"))
}

pub fn stored<'a>(
    environment: &'a protocol::Environment,
    name: &Ident,
) -> syn::Result<(&'a Expr, &'a Path)> {
    let found = environment.bindings.iter().find(|b| match b {
        protocol::Binding::Stored { name: n, .. } => symbol_name(n) == symbol_name(name),
        _ => false,
    });
    match found {
        Some(protocol::Binding::Stored {
            value, operations, ..
        }) => Ok((value, operations)),
        _ => Err(syn::Error::new_spanned(
            name,
            "this provider requires an established stored relation; an Access region is not storage",
        )),
    }
}

pub fn heading(plan: &physical::Module, name: &Ident) -> syn::Result<Vec<Ident>> {
    let ra = &plan.index_requirements.relational_plan;
    let relations = relational_plan::check_in(ra.program.inputs(), &ra.plan)?;
    let info = relations
        .get(&symbol_name(name))
        .ok_or_else(|| syn::Error::new_spanned(name, "expected an RA relation heading"))?;
    info.heading
        .iter()
        .map(|s| {
            let mut ident = syn::parse_str::<Ident>(s)
                .or_else(|_| syn::parse_str::<Ident>(&format!("r#{s}")))?;
            ident.set_span(name.span());
            Ok(ident)
        })
        .collect()
}
