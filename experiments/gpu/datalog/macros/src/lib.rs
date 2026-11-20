//! Named pipeline adapters for checked GPU Datalog realization.
use proc_macro::TokenStream;

#[proc_macro]
pub fn device_storage(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_datalog_storage::Program>(
        input.into(),
        cq_gpu_datalog_core::storage::compile_with,
    )
    .into()
}

/// `print_rounds` or `print_rounds(manual("rule@delta" => path, ...))`: the
/// named rule versions call a hand-written function in place of the generated
/// hole (`cq_gpu_datalog_core::print::Manual`).
#[proc_macro]
pub fn print_rounds(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_datalog_round::Program, syn::Expr>(input.into(), |source, args| {
        cq_gpu_datalog_core::print::compile_with(source, &manual(args)?)
    })
    .into()
}

fn manual(args: proc_macro2::TokenStream) -> syn::Result<cq_gpu_datalog_core::print::Manual> {
    use syn::parse::{ParseStream, Parser};
    let mut manual = cq_gpu_datalog_core::print::Manual::new();
    if args.is_empty() {
        return Ok(manual);
    }
    let entries = |input: ParseStream| {
        let name: syn::Ident = input.parse()?;
        if name != "manual" {
            return Err(syn::Error::new_spanned(name, "expected manual(\"rule@delta\" => path, ...)"));
        }
        let content;
        syn::parenthesized!(content in input);
        let mut entries = Vec::new();
        while !content.is_empty() {
            let label: syn::LitStr = content.parse()?;
            content.parse::<syn::Token![=>]>()?;
            let path: syn::Path = content.parse()?;
            entries.push((label, path));
            if !content.is_empty() {
                content.parse::<syn::Token![,]>()?;
            }
        }
        Ok(entries)
    };
    for (label, path) in entries.parse2(args)? {
        if manual.insert(label.value(), path).is_some() {
            return Err(syn::Error::new_spanned(label, "rule version replaced twice"));
        }
    }
    Ok(manual)
}

#[proc_macro]
pub fn bind_inputs(input: TokenStream) -> TokenStream {
    cq_stage::Structures::<cq_lang_structures::physical::Program, cq_lang_structures::Program>::rewrite(
        input.into(),
        |physical: &cq_lang_structures::physical::Program, source: &cq_lang_structures::Program, args| {
            let target = cq_gpu_datalog_core::bind_inputs(physical, source, args)?;
            Ok((physical.clone(), target))
        },
    )
    .into()
}

#[proc_macro]
pub fn bind_round(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_datalog_round::Program>(
        input.into(),
        cq_gpu_datalog_core::bindings::compile_with,
    )
    .into()
}
