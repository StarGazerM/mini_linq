use cq_ir::cq;
use proc_macro::TokenStream;
use quote::quote;
pub(crate) fn expand(input: TokenStream) -> TokenStream {
    let source_text = input.to_string();
    let source: cq::Module = match syn::parse(input) {
        Ok(source) => source,
        Err(error) => return error.into_compile_error().into(),
    };
    let catalog = quote! {
        pub(crate) const RUST_PATH: &str = file!();
        pub(crate) const SOURCE: &str = #source_text;

        pub(crate) fn module() -> ::syn::Result<::cq_ir::cq::Module> {
            ::syn::parse_str(SOURCE)
        }
    };

    #[cfg(feature = "compiled-workloads")]
    {
        let program =
            match cq_compose::compile_program(&source, &syn::parse_quote!(mini_linq::pull)) {
                Ok(program) => program,
                Err(error) => return error.into_compile_error().into(),
            };

        quote!(#catalog #program).into()
    }

    #[cfg(not(feature = "compiled-workloads"))]
    {
        let _ = source;
        catalog.into()
    }
}
