//! Device realization of the ordinary Datalog Storage and Round records.
pub mod bindings;
pub mod domains;
pub mod encode;
pub mod lanes;
pub mod print;
pub mod storage;

#[cfg(test)]
mod print_tests;

use quote::ToTokens;

/// Bind only the input layouts explicitly supplied by the checked Round invocation.
/// This is a Structures refinement, not a query planner or an index builder.
pub fn bind_inputs(
    physical: &cq_lang_structures::physical::Program,
    source: &cq_lang_structures::Program,
    args: proc_macro2::TokenStream,
) -> syn::Result<cq_lang_structures::Program> {
    struct Bindings {
        providers: cq_lang_structures::Providers,
        inputs: std::collections::BTreeMap<String, cq_lang_structures::Provider>,
    }
    impl syn::parse::Parse for Bindings {
        fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
            let providers = input.parse()?;
            let mut bindings = std::collections::BTreeMap::new();
            while !input.is_empty() {
                let name: syn::Ident = input.parse()?;
                input.parse::<syn::Token![=>]>()?;
                let provider = input.parse()?;
                input.parse::<syn::Token![;]>()?;
                if bindings.insert(name.to_string(), provider).is_some() {
                    return Err(syn::Error::new_spanned(
                        name,
                        "duplicate device input binding",
                    ));
                }
            }
            Ok(Self {
                providers,
                inputs: bindings,
            })
        }
    }
    let bindings = syn::parse2::<Bindings>(args)?;
    let analysis = cq_lang_structures::contract::check(physical, source)?;
    let mut target = source.clone();
    for entry in &mut target.structures.entries {
        if !matches!(entry.structure, cq_lang_structures::Structure::Host { .. }) {
            continue;
        }
        let name = &entry.name;
        let host = &analysis.physical.hosts[&cq_syntax::symbol_name(name)];
        let provider = bindings.inputs.get(host).ok_or_else(|| {
            syn::Error::new_spanned(
                entry.to_token_stream(),
                "GPU Datalog input has no recorded device layout",
            )
        })?;
        if let cq_lang_structures::Structure::Host { keyed, .. } = &mut entry.structure {
            keyed.provider = provider.clone();
        }
    }
    target
        .structures
        .declare(&bindings.providers.declarations)?;
    cq_lang_structures::contract::check(physical, &target)?;
    Ok(target)
}
