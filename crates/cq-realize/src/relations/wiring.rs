//! Observed hosts add only their explicit observer argument to configured paths.
//! Plain hosts call caller bindings directly; leaf configuration stays outside.
use super::{
    RelationOptions,
    layout::{Layout, local},
};
use cq_ir::{relational_plan as ra, symbol_name};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct Wiring {
    aliases: BTreeMap<String, (syn::Path, syn::Ident)>,
    work: Option<syn::Expr>,
}
fn spelling(path: &syn::Path) -> String {
    path.to_token_stream().to_string()
}
impl Wiring {
    pub fn new(
        source: &ra::Module,
        layout: &Layout,
        options: &RelationOptions,
        work: Option<syn::Expr>,
    ) -> syn::Result<Self> {
        let mut paths: Vec<&syn::Path> = layout.views.values().map(|v| &v.provider).collect();
        for definition in &source.plan.definitions {
            match &definition.operator {
                ra::Operator::Semijoin(_) => paths.extend([
                    options.scan_for(&definition.result)?,
                    options.keys_for(&definition.result)?,
                ]),
                ra::Operator::NaturalJoin(_) => paths.push(options.join_for(&definition.result)?),
                _ => {}
            }
        }
        let mut occupied: BTreeSet<_> = paths
            .iter()
            .copied()
            .chain(options.access.iter())
            .filter_map(|path| path.get_ident().map(symbol_name))
            .collect();
        let mut aliases = BTreeMap::new();
        if work.is_some() {
            let mut next = 0;
            for path in paths {
                aliases.entry(spelling(path)).or_insert_with(|| {
                    loop {
                        let name = local("observed", next);
                        next += 1;
                        if occupied.insert(symbol_name(&name)) {
                            break (path.clone(), name);
                        }
                    }
                });
            }
        }
        Ok(Self { aliases, work })
    }
    pub fn provider(&self, path: &syn::Path) -> syn::Path {
        self.access(path)
    }
    pub fn access(&self, path: &syn::Path) -> syn::Path {
        match self.aliases.get(&spelling(path)) {
            Some((_, name)) => syn::parse_quote!(#name),
            None => path.clone(),
        }
    }
    pub fn emit(&self) -> TokenStream {
        let bindings = self.aliases.values().map(|(path, name)| {
            let work = self
                .work
                .as_ref()
                .expect("observed aliases have an observer");
            quote! {
                macro_rules! #name {
                    ($($source:tt)*) => { #path! { work (#work); $($source)* } };
                }
            }
        });
        quote!(#(#bindings)*)
    }
}
