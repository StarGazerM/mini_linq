//! Observed hosts own counters and forward an explicit work expression to bindings.
use super::super::{
    ObservedOptions, check_observed_program, check_observed_query, layout, statements,
};
use super::*;
use quote::format_ident;

fn statistics(name: &syn::Ident, visibility: &syn::Visibility, storage: &syn::Path) -> TokenStream {
    quote! {
        #[derive(Debug, Default)]
        #visibility struct #name {
            pub steps: ::std::vec::Vec<(&'static ::core::primitive::str, ::core::primitive::usize, #storage::Work)>,
            pub output: #storage::Work,
        }
    }
}
fn body(
    source: &ra::Module,
    options: &ObservedOptions,
    layout: &Layout,
    initializer: &syn::Expr,
) -> syn::Result<TokenStream> {
    let storage = &options.counter_storage;
    let wiring = Wiring::new(
        source,
        layout,
        &options.relations,
        Some(syn::parse_quote!(__ml_work)),
    )?;
    let bindings = wiring.emit();
    let statements = statements(source, &options.relations, layout, &wiring)?;
    let statements = statements
        .iter()
        .map(|(statement, checkpoint)| quote!(#statement #checkpoint));
    let answer = answer(source, layout, &wiring);
    let row = row(layout);
    Ok(quote! {
        let __ml_work = &#storage::Counters::default();
        let mut __ml_stats = #initializer;
        #bindings
        #(#statements)*
        #answer
        __ml_stats.output = __ml_work.take();
        (__ml_answer.into_iter().collect::<::std::vec::Vec<#row>>(), __ml_stats)
    })
}

pub fn compile_observed_program(
    source: &ra::Module,
    options: &ObservedOptions,
) -> syn::Result<syn::File> {
    ra::contract::check(source)?;
    options.relations.validate(source)?;
    let layout = layout::analyze(source, &options.relations)?;
    let parameters = parameters(source, &layout, &options.relations);
    let row = row(&layout);
    let (visibility, program) = (&source.program.visibility, &source.program.name);
    let stats = format_ident!("{}Stats", symbol_name(program));
    let definition = statistics(&stats, visibility, &options.counter_storage);
    let initializer = syn::parse_quote!(#stats::default());
    let body = body(source, options, &layout, &initializer)?;
    let target = syn::parse2(quote! {
        #visibility struct #program;
        #definition
        impl #program {
            pub fn run(#(#parameters),*) -> (::std::vec::Vec<#row>, #stats) { #body }
        }
    })?;
    check_observed_program(source, &target, options)?;
    Ok(target)
}

pub fn compile_observed_query(
    source: &ra::Module,
    options: &ObservedOptions,
) -> syn::Result<syn::Expr> {
    ra::contract::check(source)?;
    options.relations.validate(source)?;
    let layout = layout::analyze(source, &options.relations)?;
    let parameters = parameters(source, &layout, &options.relations);
    let storage = &options.counter_storage;
    // The helper type exists only in the constructor's callee block. Caller
    // storage paths and configured macros expand outside that scope, including
    // identifiers hidden in caller macro bodies that this compiler cannot see.
    let initializer = syn::parse_quote! {
        ({
            #[derive(Debug)]
            struct __MlStatistics<W> {
                pub steps: ::std::vec::Vec<(&'static ::core::primitive::str, ::core::primitive::usize, W)>,
                pub output: W,
            }
            |output| __MlStatistics { steps: ::std::vec::Vec::new(), output }
        })(<#storage::Work as ::core::default::Default>::default())
    };
    let body = body(source, options, &layout, &initializer)?;
    let target = syn::parse2(quote! {
        move |#(#parameters),*| { #body }
    })?;
    check_observed_query(source, &target, options)?;
    Ok(target)
}
