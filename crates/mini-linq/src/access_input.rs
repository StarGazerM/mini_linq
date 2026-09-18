use syn::parse::{Parse, ParseStream};
mod kw {
    syn::custom_keyword!(access);
}

pub(crate) struct AccessInput<S> {
    pub(crate) access: syn::Path,
    pub(crate) source: S,
}
impl<S: Parse> Parse for AccessInput<S> {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let access = if input.peek(kw::access) {
            input.parse::<kw::access>()?;
            let path = input.parse()?;
            input.parse::<syn::Token![;]>()?;
            path
        } else {
            syn::parse_quote!(mini_linq::access_to_set)
        };
        Ok(Self {
            access,
            source: input.parse()?,
        })
    }
}
