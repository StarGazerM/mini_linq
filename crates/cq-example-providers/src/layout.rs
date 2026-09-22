//! Representation-specific field correspondence, separate from IX analysis.

pub use cq_ir::physical::lookup::{definition, heading, index_provider, relation_provider, stored};
use cq_ir::{physical, relational_plan, symbol_name};
use syn::Ident;

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
