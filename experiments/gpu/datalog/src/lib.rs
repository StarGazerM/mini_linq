//! Device realization of the ordinary Datalog Storage and Round records.
pub mod storage;
pub mod print;
pub mod bindings;

#[cfg(test)]
mod print_tests;

use quote::ToTokens;

/// Bind only the input layouts explicitly supplied by the checked Round invocation.
/// This is a Structures refinement, not a query planner or an index builder.
pub fn bind_inputs(source: &cq_lang_structures::Program, args: proc_macro2::TokenStream)
    -> syn::Result<cq_lang_structures::Program>
{
    struct Bindings(std::collections::BTreeMap<String, syn::Type>);
    impl syn::parse::Parse for Bindings {
        fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
            let mut bindings = std::collections::BTreeMap::new();
            while !input.is_empty() {
                let name: syn::Ident = input.parse()?;
                input.parse::<syn::Token![=>]>()?;
                let ty = input.parse()?;
                input.parse::<syn::Token![;]>()?;
                if bindings.insert(name.to_string(), ty).is_some() {
                    return Err(syn::Error::new_spanned(name, "duplicate device input binding"));
                }
            }
            Ok(Self(bindings))
        }
    }
    let bindings = syn::parse2::<Bindings>(args)?.0;
    let analysis = cq_lang_structures::contract::check(source)?;
    let mut target = source.clone();
    for entry in &mut target.structures.entries {
        if !matches!(entry.structure, cq_lang_structures::Structure::Host { .. }) { continue; }
        let name = &entry.name;
        let host = &analysis.physical.hosts[&cq_syntax::symbol_name(name)];
        let ty = bindings.get(host).ok_or_else(|| syn::Error::new_spanned(
            entry.to_token_stream(), "GPU Datalog input has no recorded device layout"))?;
        if let cq_lang_structures::Structure::Host { keyed, .. } = &mut entry.structure {
            keyed.ty = ty.clone();
        }
    }
    cq_lang_structures::contract::check(&target)?;
    Ok(target)
}
