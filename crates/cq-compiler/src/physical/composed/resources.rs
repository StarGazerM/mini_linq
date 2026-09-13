//! Sharing is scoped to the current version, key, recipe and collection type.
//! Construction sees only a projected source, so no consumer context can change
//! the meaning or representation of the shared resource.
use super::{
    PhysicalOptions,
    layout::{Layout, local, separator},
};
use cq_ir::{relational_plan as ra, symbol_name};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use std::collections::{BTreeMap, BTreeSet};
pub(super) struct Resource {
    pub name: syn::Ident,
    pub recipe: syn::Path,
    pub keys: syn::Path,
    pub first_use: usize,
    pub projected: syn::Ident,
    pub columns: Vec<String>,
}
pub(super) struct Resources {
    pub entries: Vec<Resource>,
    pub uses: BTreeMap<String, usize>,
}
pub(super) fn analyze(
    source: &ra::Module,
    layout: &Layout,
    options: &PhysicalOptions,
) -> Resources {
    let mut entries = Vec::new();
    let mut uses = BTreeMap::new();
    let mut groups = BTreeMap::new();
    let mut occupied: BTreeSet<_> = source
        .program
        .inputs()
        .map(|r| symbol_name(&r.name))
        .chain(
            source
                .plan
                .definitions
                .iter()
                .map(|d| symbol_name(&d.result)),
        )
        .collect();
    let mut fresh = 0;

    for (i, d) in source.plan.definitions.iter().enumerate() {
        let ra::Operator::Semijoin(op) = &d.operator else {
            continue;
        };
        let result = symbol_name(&d.result);
        let right = layout.view(&op.right);
        let key = separator(layout.view(&op.left), right);
        let recipe = options.recipe(&result);
        let keys = options.keys(&result);
        let group = (
            symbol_name(&op.right),
            key.clone(),
            recipe.to_token_stream().to_string(),
            keys.to_token_stream().to_string(),
        );
        let index = *groups.entry(group).or_insert_with(|| {
            let n = entries.len();
            let projected = loop {
                let candidate = local("keys", fresh);
                fresh += 1;
                if occupied.insert(symbol_name(&candidate)) {
                    break candidate;
                }
            };
            entries.push(Resource {
                name: local("resource", n),
                recipe: recipe.clone(),
                keys: keys.clone(),
                first_use: i,
                projected,
                columns: key.clone(),
            });
            n
        });
        uses.insert(result, index);
    }
    Resources { entries, uses }
}
pub(super) fn emit(
    resource: &Resource,
    source: &ra::Module,
    layout: &Layout,
    wiring: &super::wiring::Wiring,
) -> syn::Result<TokenStream> {
    let definition = &source.plan.definitions[resource.first_use];
    let ra::Operator::Semijoin(op) = &definition.operator else {
        unreachable!()
    };
    let right = &op.right;
    let projected = &resource.projected;
    let columns = resource
        .columns
        .iter()
        .map(|name| super::lower::logical_ident(name));
    let projection: ra::Definition = syn::parse_quote! {
        #projected = project #right keep { #(#columns),* };
    };
    let invocation = super::lower::context(&projection, &[(right, layout.view(right))], wiring)?;
    cq_ir::physical_context::contract::check_project(&invocation)?;
    let name = &resource.name;
    let recipe = wiring.recipe(&resource.recipe, Some(&resource.keys));
    Ok(quote! { let #name = #recipe! { #invocation }; })
}
