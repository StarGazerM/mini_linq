//! Current-version key resources are shared independently of candidate scanning.
use super::{
    RelationOptions,
    layout::{Layout, local, positions, separator},
};
use cq_ir::{relational_plan as ra, rust_access_plan as access, symbol_name};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use std::collections::BTreeMap;

pub(super) struct Resource {
    pub name: syn::Ident,
    pub provider: syn::Path,
    pub first_use: usize,
    pub columns: Vec<String>,
}
pub(super) struct Resources {
    pub entries: Vec<Resource>,
    pub uses: BTreeMap<String, usize>,
}
pub(super) fn analyze(
    source: &ra::Module,
    layout: &Layout,
    options: &RelationOptions,
) -> syn::Result<Resources> {
    let mut entries = Vec::new();
    let mut uses = BTreeMap::new();
    let mut groups = BTreeMap::new();
    for (i, d) in source.plan.definitions.iter().enumerate() {
        let ra::Operator::Semijoin(op) = &d.operator else {
            continue;
        };
        let result = symbol_name(&d.result);
        let key = separator(layout.view(&op.left), layout.view(&op.right));
        let provider = options.keys_for(&d.result)?;
        let group = (
            symbol_name(&op.right),
            positions(&layout.view(&op.right).layout, &key),
            provider.to_token_stream().to_string(),
        );
        let index = *groups.entry(group).or_insert_with(|| {
            let n = entries.len();
            entries.push(Resource {
                name: local("resource", n),
                provider: provider.clone(),
                first_use: i,
                columns: key,
            });
            n
        });
        uses.insert(result, index);
    }
    Ok(Resources { entries, uses })
}
pub(super) fn emit(
    resource: &Resource,
    source: &ra::Module,
    layout: &Layout,
    wiring: &super::wiring::Wiring,
) -> syn::Result<TokenStream> {
    let ra::Operator::Semijoin(op) = &source.plan.definitions[resource.first_use].operator else {
        unreachable!()
    };
    let right = layout.view(&op.right);
    let provider = wiring.provider(&right.provider);
    let columns = positions(&right.layout, &resource.columns);
    let value = &right.name;
    let keys = wiring.access(&resource.provider);
    let fields = columns.iter().map(|column| {
        let field = super::expressions::field(right, quote!(__row), *column, wiring);
        quote!(&(#field))
    });
    let plan: access::Plan = syn::parse2(quote! {
        out(__row) => (#keys! { key (#(#fields,)*) }) :-
        Rows(__row) => for (__row,) in (#provider! { scan (__cq_right) }.map(|row| (row,)));
    })?;
    access::contract::check(&plan)?;
    let name = &resource.name;
    let keys = wiring.access(&resource.provider);
    Ok(quote! { let #name = { let __cq_right = &#value; #keys! { build { #plan } } }; })
}
