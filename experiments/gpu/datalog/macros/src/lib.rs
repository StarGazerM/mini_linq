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

/// `print_rounds(manual("rule@delta" => path, ...), observe(...))`. Each
/// section is optional; [`cq_gpu_datalog_core::print::HoleHooks`] documents
/// the arguments and return type of each hook.
#[proc_macro]
pub fn print_rounds(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_datalog_round::Program, syn::Expr>(input.into(), |source, args| {
        cq_gpu_datalog_core::print::compile_with(source, &hooks(args)?)
    })
    .into()
}

fn hooks(args: proc_macro2::TokenStream) -> syn::Result<cq_gpu_datalog_core::print::HoleHooks> {
    use syn::parse::{ParseStream, Parser};
    let parse = |input: ParseStream| {
        let mut hooks = cq_gpu_datalog_core::print::HoleHooks::default();
        while !input.is_empty() {
            let name: syn::Ident = input.parse()?;
            let entries = if name == "manual" {
                &mut hooks.manual
            } else if name == "observe" {
                &mut hooks.observe
            } else {
                return Err(syn::Error::new_spanned(name, "expected manual(...) or observe(...)"));
            };
            let content;
            syn::parenthesized!(content in input);
            while !content.is_empty() {
                let label: syn::LitStr = content.parse()?;
                content.parse::<syn::Token![=>]>()?;
                let path: syn::Path = content.parse()?;
                if entries.insert(label.value(), path).is_some() {
                    return Err(syn::Error::new_spanned(label, format!("duplicate {name} hook")));
                }
                if !content.is_empty() {
                    content.parse::<syn::Token![,]>()?;
                }
            }
            if !input.is_empty() {
                input.parse::<syn::Token![,]>()?;
            }
        }
        Ok(hooks)
    };
    parse.parse2(args)
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
