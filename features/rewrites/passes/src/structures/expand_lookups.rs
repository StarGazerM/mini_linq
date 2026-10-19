use cq_lang_structures::{self as ir, physical::Operator, symbol_name};
use proc_macro2::TokenStream;

/// Record `stage = batch N expand` at each unstaged probe whose recorded build
/// exposes length-known ranges: a SortedRun lookup, or the first probe of a
/// recorded Smallest intersection with PrefixRun candidate builds on both sides.
/// Each candidate is kept only if the language's stage contract
/// (ST-Expand/ST-Stage) accepts it; Physical and provider decisions are unchanged.
pub fn expand_lookups(source: &ir::Program, arguments: TokenStream) -> syn::Result<ir::Program> {
    let size: syn::LitInt = syn::parse2(arguments)?;
    if size.base10_parse::<usize>()? == 0 {
        return Err(syn::Error::new_spanned(
            size,
            "expand_lookups batch size must be positive",
        ));
    }
    let facts = ir::contract::check(source)?;
    let mut target = source.clone();
    for definition in &source.physical.program.definitions {
        if !matches!(definition.operator, Operator::Probe(..)) {
            continue;
        }
        if facts.stages.contains_key(&symbol_name(&definition.name)) {
            continue;
        }
        let name = &definition.name;
        target
            .structures
            .entries
            .push(syn::parse_quote!(#name stage = batch #size expand;));
        if ir::contract::check(&target).is_err() {
            target.structures.entries.pop();
        }
    }
    ir::check_rewrite(source, &target)?;
    Ok(target)
}
