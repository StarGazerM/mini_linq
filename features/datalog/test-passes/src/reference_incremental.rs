use cq_datalog_support::Fresh;
use cq_lang_datalog_incremental as incremental;
use cq_syntax::symbol_name;
use proc_macro2::TokenStream;
use quote::quote;
use std::collections::BTreeMap;

/// Execute the recorded covering variants, not a newly selected cover.
pub fn emit(program: &incremental::Program) -> syn::Result<TokenStream> {
    let analysis = incremental::contract::check(program)?;
    let source = &program.strata.source;
    let mut fresh = Fresh::new(program);
    let state = fresh.id("reference_state");
    let component = fresh.id("reference_component");
    let seed = fresh.id("reference_seed");
    let changed = fresh.id("reference_changed");
    let mut positions = analysis.positions.clone();
    let mut initial = Vec::new();
    for declaration in &source.declarations {
        let types = declaration.columns.iter().map(|c| &c.ty);
        let tuple = quote!((#(#types,)*));
        initial.push(if let Some(value) = &declaration.value {
            let value = &value.expression;
            let row = fresh.id("reference_input");
            quote!(::std::iter::IntoIterator::into_iter(#value)
                .map(|#row| ::std::borrow::Borrow::<#tuple>::borrow(&#row).clone())
                .collect::<::std::collections::HashSet<#tuple>>())
        } else {
            quote!(::std::collections::HashSet::<#tuple>::new())
        });
    }
    for delta in &program.deltas {
        positions.insert(symbol_name(&delta.name), initial.len());
        let declaration = &source.declarations[analysis.positions[&symbol_name(&delta.relation)]];
        let types = declaration.columns.iter().map(|c| &c.ty);
        initial.push(quote!(::std::collections::HashSet::<(#(#types,)*)>::new()));
    }
    let mut arms = Vec::new();
    for (index, members) in analysis.components.iter().enumerate() {
        let candidates: BTreeMap<_, _> = members
            .iter()
            .map(|i| (*i, fresh.id("reference_candidates")))
            .collect();
        let declarations = members.iter().map(|i| {
            let name = &candidates[i];
            let types = source.declarations[*i].columns.iter().map(|c| &c.ty);
            quote!(let mut #name = ::std::collections::HashSet::<(#(#types,)*)>::new();)
        });
        let mut variants = Vec::new();
        for variant in &program.variants {
            let head = analysis.positions[&symbol_name(&variant.query.head.relation)];
            if !members.contains(&head) {
                continue;
            }
            let is_seed = variant.delta.is_none();
            let body = super::reference::body(
                &variant.query.body.iter().collect::<Vec<_>>(),
                &variant.query.head.args.iter().collect::<Vec<_>>(),
                &candidates[&head],
                &state,
                &positions,
                &[],
                BTreeMap::new(),
                &mut fresh,
            )?;
            variants.push(quote!(if #seed == #is_seed { #body }));
        }
        let publication = members.iter().map(|i| {
            let full = syn::Index::from(*i);
            let delta = syn::Index::from(
                positions
                    [&symbol_name(&analysis.deltas[&symbol_name(&source.declarations[*i].name)])],
            );
            let candidates = &candidates[i];
            quote! {
                #candidates.retain(|row| !#state.#full.contains(row));
                #changed |= !#candidates.is_empty();
                #state.#full.extend(#candidates.iter().cloned());
                #state.#delta = #candidates;
            }
        });
        arms.push(quote!(#index => {
            #(#declarations)*
            #(#variants)*
            let mut #changed = false;
            #(#publication)*
            #changed
        }));
    }
    let count = analysis.components.len();
    let outputs: Vec<_> = source
        .outputs
        .iter()
        .map(|name| {
            let field = syn::Index::from(analysis.positions[&symbol_name(name)]);
            quote!(#state.#field)
        })
        .collect();
    let answer = if outputs.len() == 1 {
        outputs[0].clone()
    } else {
        quote!((#(#outputs,)*))
    };
    Ok(quote!({
        let #state = ::cq_datalog_testkit::eval_incremental(
            || (#(#initial,)*), #count,
            |#state, #component, #seed| match #component {
                #(#arms,)*
                _ => unreachable!("reference component index"),
            },
        );
        #answer
    }))
}
