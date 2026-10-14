use cq_lang_cq as cq;
use proc_macro::TokenStream;
use quote::{format_ident, quote};
pub(crate) fn expand(input: TokenStream) -> TokenStream {
    let source_text = input.to_string();
    let tokens = proc_macro2::TokenStream::from(input.clone());
    let source: cq::Program = match syn::parse(input) {
        Ok(source) => source,
        Err(error) => return error.into_compile_error().into(),
    };
    let exported = format_ident!("__with_{}", source.name);
    let catalog = quote! {
        #[macro_export]
        macro_rules! #exported {
            ($($m:ident)::+) => { $($m)::+! { #tokens } };
            (:: $($m:ident)::+) => { ::$($m)::+! { #tokens } };
        }
        pub use #exported as with_query;
        pub(crate) const RUST_PATH: &str = file!();
        pub(crate) const SOURCE: &str = #source_text;

        pub(crate) fn module() -> ::syn::Result<::cq_lang_cq::Program> {
            ::syn::parse_str(SOURCE)
        }
    };

    catalog.into()
}
