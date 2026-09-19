//! Representation-specific field correspondence, separate from IX analysis.

use cq_ir::{physical, provider_protocol as protocol, relational_plan, symbol_name};
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
    bindings: &'a cq_ir::SemicolonList<physical::Binding>,
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

/// For a stored host input or a direct Rename, map physical declaration order
/// to logical names. This is representation decoding, not index-demand analysis.
pub fn physical_columns(plan: &physical::Module, name: &Ident) -> syn::Result<Vec<Ident>> {
    let ra = &plan.index_requirements.relational_plan;
    if let Some(declaration) = ra
        .program
        .inputs()
        .find(|d| symbol_name(&d.name) == symbol_name(name))
    {
        return Ok(declaration.columns.iter().map(|c| c.name.clone()).collect());
    }
    let definition = definition(plan, name)?;
    let relational_plan::Operator::Rename(rename) = &definition.operator else {
        return Err(syn::Error::new_spanned(
            name,
            "this stored-input provider admits host inputs and direct Rename results only",
        ));
    };
    let declaration = ra
        .program
        .inputs()
        .find(|d| symbol_name(&d.name) == symbol_name(&rename.relation))
        .ok_or_else(|| syn::Error::new_spanned(name, "missing source declaration"))?;
    declaration
        .columns
        .iter()
        .map(|column| {
            rename
                .mapping
                .iter()
                .find(|m| symbol_name(&m.source) == symbol_name(&column.name))
                .map(|m| m.target.clone())
                .ok_or_else(|| {
                    syn::Error::new_spanned(column, "Rename does not map this source field")
                })
        })
        .collect()
}

pub fn logical_columns(plan: &physical::Module, name: &Ident) -> syn::Result<Vec<Ident>> {
    let mut fields = physical_columns(plan, name)?;
    fields.sort_by_key(symbol_name);
    Ok(fields)
}

pub fn physical_types(plan: &physical::Module, name: &Ident) -> syn::Result<Vec<syn::Type>> {
    let ra = &plan.index_requirements.relational_plan;
    let source = match definition(plan, name) {
        Ok(d) => match &d.operator {
            relational_plan::Operator::Rename(rename) => &rename.relation,
            _ => {
                return Err(syn::Error::new_spanned(
                    name,
                    "stored provider requires a direct input or Rename",
                ));
            }
        },
        Err(_) => name,
    };
    let declaration = ra
        .program
        .inputs()
        .find(|d| symbol_name(&d.name) == symbol_name(source))
        .ok_or_else(|| syn::Error::new_spanned(source, "missing host declaration"))?;
    Ok(declaration.columns.iter().map(|c| c.ty.clone()).collect())
}
