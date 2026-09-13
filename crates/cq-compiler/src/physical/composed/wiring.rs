//! Lexical partial application of administrative macro arguments. These are
//! ordinary Rust macro definitions, not fields of the semantic operator input.
use super::{
    PhysicalOptions,
    layout::{Layout, local},
};
use cq_ir::{relational_plan as ra, symbol_name};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct Wiring {
    providers: BTreeMap<String, (syn::Path, syn::Ident)>,
    recipes: BTreeMap<(String, Option<String>), ConfiguredRecipe>,
}
struct ConfiguredRecipe {
    path: syn::Path,
    keys: Option<syn::Path>,
    name: syn::Ident,
}
fn spelling(path: &syn::Path) -> String {
    path.to_token_stream().to_string()
}
impl Wiring {
    pub fn new(source: &ra::Module, layout: &Layout, options: &PhysicalOptions) -> Self {
        // Macro names use textual scope even with mixed-site identifier spans.
        // Exclude all caller-selected bare names, including cross-role aliases.
        let mut occupied: BTreeSet<_> = options
            .inputs
            .values()
            .chain(options.results.values())
            .chain(options.semijoins.values())
            .chain(options.joins.values())
            .chain([
                &options.materialize,
                &options.semijoin,
                &options.join,
                &options.access,
            ])
            .filter_map(|p| p.get_ident().map(symbol_name))
            .collect();
        let mut providers = BTreeMap::new();
        for view in layout.views.values() {
            providers
                .entry(spelling(&view.provider))
                .or_insert_with(|| (view.provider.clone(), local("provider", 0)));
        }
        let mut next = 0;
        for (_, name) in providers.values_mut() {
            *name = fresh("provider", &mut next, &mut occupied);
        }
        let mut recipes = BTreeMap::new();
        for d in &source.plan.definitions {
            let result = symbol_name(&d.result);
            let (path, keys) = match &d.operator {
                ra::Operator::Semijoin(_) => (options.recipe(&result), Some(options.keys(&result))),
                ra::Operator::NaturalJoin(_) => {
                    (options.joins.get(&result).unwrap_or(&options.join), None)
                }
                _ => continue,
            };
            recipes
                .entry((spelling(path), keys.map(spelling)))
                .or_insert_with(|| ConfiguredRecipe {
                    path: path.clone(),
                    keys: keys.cloned(),
                    name: local("recipe", 0),
                });
        }
        let mut next = 0;
        for recipe in recipes.values_mut() {
            recipe.name = fresh("recipe", &mut next, &mut occupied);
        }
        Self { providers, recipes }
    }
    pub fn provider(&self, path: &syn::Path) -> syn::Path {
        let name = &self.providers[&spelling(path)].1;
        syn::parse_quote!(#name)
    }
    pub fn recipe(&self, path: &syn::Path, keys: Option<&syn::Path>) -> syn::Path {
        let name = &self.recipes[&(spelling(path), keys.map(spelling))].name;
        syn::parse_quote!(#name)
    }
    pub fn emit(&self, options: &PhysicalOptions, observed: bool) -> TokenStream {
        let providers = self
            .providers
            .values()
            .map(|(path, name)| wrapper(name, path, &options.storage, None, None, observed));
        let recipes = self
            .recipes
            .values()
            .map(|ConfiguredRecipe { path, keys, name }| {
                wrapper(
                    name,
                    path,
                    &options.storage,
                    Some(&options.access),
                    keys.as_ref(),
                    observed,
                )
            });
        quote!(#(#providers)* #(#recipes)*)
    }
}

/// Syntactic binding of one macro's configuration. The correspondence checker
/// independently checks its lexical definition, selected arguments and uses.
pub(super) fn wrapper(
    name: &syn::Ident,
    path: &syn::Path,
    storage: &syn::Path,
    access: Option<&syn::Path>,
    keys: Option<&syn::Path>,
    observed: bool,
) -> TokenStream {
    let access = access.map(|p| quote!(access #p;));
    let keys = keys.map(|p| quote!(keys #p;));
    let work = observed.then(|| quote!(work (__ml_work);));
    quote! {
        macro_rules! #name {
            ($($source:tt)*) => {
                #path! { storage #storage; #access #keys #work $($source)* }
            };
        }
    }
}

fn fresh(prefix: &str, next: &mut usize, occupied: &mut BTreeSet<String>) -> syn::Ident {
    loop {
        let candidate = local(prefix, *next);
        *next += 1;
        if occupied.insert(symbol_name(&candidate)) {
            return candidate;
        }
    }
}
