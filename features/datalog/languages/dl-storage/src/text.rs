//! Delimited, lossless text for the checked execution records.
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::parse::ParseStream;

pub trait Text: Sized {
    fn parse_text(input: ParseStream<'_>) -> syn::Result<Self>;
    fn text(&self) -> TokenStream;
}
pub fn field<T: Text>(input: ParseStream<'_>, expected: &str) -> syn::Result<T> {
    let name: syn::Ident = input.parse()?;
    if name != expected {
        return Err(syn::Error::new_spanned(
            name,
            format!("expected {expected}"),
        ));
    }
    let body;
    syn::braced!(body in input);
    let value = T::parse_text(&body)?;
    if !body.is_empty() {
        return Err(body.error("unexpected record contents"));
    }
    Ok(value)
}
impl Text for usize {
    fn parse_text(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<syn::LitInt>()?.base10_parse()
    }
    fn text(&self) -> TokenStream {
        let n = syn::LitInt::new(&self.to_string(), proc_macro2::Span::call_site());
        quote!(#n)
    }
}
impl<T: Text> Text for Vec<T> {
    fn parse_text(input: ParseStream<'_>) -> syn::Result<Self> {
        let list;
        syn::bracketed!(list in input);
        let mut values = Vec::new();
        while !list.is_empty() {
            let item;
            syn::braced!(item in list);
            values.push(T::parse_text(&item)?);
            if !item.is_empty() {
                return Err(item.error("unexpected item contents"));
            }
        }
        Ok(values)
    }
    fn text(&self) -> TokenStream {
        let values = self.iter().map(Text::text);
        quote!([#({#values})*])
    }
}
impl<T: Text> Text for Option<T> {
    fn parse_text(input: ParseStream<'_>) -> syn::Result<Self> {
        let value;
        syn::bracketed!(value in input);
        if value.is_empty() {
            Ok(None)
        } else {
            let result = T::parse_text(&value)?;
            if !value.is_empty() {
                return Err(value.error("unexpected optional contents"));
            }
            Ok(Some(result))
        }
    }
    fn text(&self) -> TokenStream {
        let value = self.as_ref().map(Text::text);
        quote!([#value])
    }
}
impl Text for (usize, syn::Expr) {
    fn parse_text(input: ParseStream<'_>) -> syn::Result<Self> {
        let column = <usize as Text>::parse_text(input)?;
        input.parse::<syn::Token![=>]>()?;
        Ok((column, input.parse()?))
    }
    fn text(&self) -> TokenStream {
        let column = self.0.text();
        let value = &self.1;
        quote!(#column => #value)
    }
}
macro_rules! syntax_text { ($($ty:ty),* $(,)?) => {$ (
    impl Text for $ty {
        fn parse_text(input: ParseStream<'_>) -> syn::Result<Self> { input.parse() }
        fn text(&self) -> TokenStream { self.to_token_stream() }
    }
)*}; }
syntax_text!(
    syn::Ident,
    syn::Expr,
    syn::Type,
    cq_syntax::RelationDecl,
    cq_lang_cq::Program,
    cq_stage::Pipeline,
    crate::incremental::Program
);

#[macro_export]
macro_rules! record {
    ($name:ident { $($field:ident : $ty:ty),* $(,)? }) => {
        impl syn::parse::Parse for $name {
            fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
                Ok(Self { $($field: $crate::text::field::<$ty>(input, stringify!($field))?,)* })
            }
        }
        impl quote::ToTokens for $name {
            fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
                $(let $field = $crate::text::Text::text(&self.$field);)*
                tokens.extend(quote::quote!($( $field { #$field } )*));
            }
        }
        impl $crate::text::Text for $name {
            fn parse_text(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> { input.parse() }
            fn text(&self) -> proc_macro2::TokenStream { quote::ToTokens::to_token_stream(self) }
        }
    };
}
