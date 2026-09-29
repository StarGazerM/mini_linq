use cq_lang_cq as cq;
use proc_macro::TokenStream;
use quote::quote;
pub(crate) fn expand(input: TokenStream) -> TokenStream {
    let source_text = input.to_string();
    let source: cq::Program = match syn::parse(input) {
        Ok(source) => source,
        Err(error) => return error.into_compile_error().into(),
    };
    let catalog = quote! {
        pub(crate) const RUST_PATH: &str = file!();
        pub(crate) const SOURCE: &str = #source_text;

        pub(crate) fn module() -> ::syn::Result<::cq_lang_cq::Program> {
            ::syn::parse_str(SOURCE)
        }
    };

    let _ = source;
    catalog.into()
}
