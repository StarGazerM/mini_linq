use std::collections::BTreeSet;
use syn::{
    Ident, LitInt, Token,
    parse::{Parse, ParseStream},
};

/// Control choices do not alter any Structures representation decision.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Options {
    pub frontier: u32,
}
impl Default for Options {
    fn default() -> Self {
        Self { frontier: 8 }
    }
}
impl Parse for Options {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut options = Self::default();
        let mut seen = BTreeSet::new();
        while !input.is_empty() {
            let option: Ident = input.parse()?;
            if !seen.insert(option.to_string()) {
                return Err(syn::Error::new_spanned(
                    option,
                    "CQ_GPU_DUPLICATE_ARGUMENT: repeated printer option",
                ));
            }
            let args;
            syn::parenthesized!(args in input);
            match option.to_string().as_str() {
                "tile" => {
                    let threads: LitInt = args.parse()?;
                    args.parse::<Token![,]>()?;
                    let items: LitInt = args.parse()?;
                    if threads.base10_parse::<u32>()? != 256 || items.base10_parse::<u32>()? != 4 {
                        return Err(syn::Error::new_spanned(
                            option,
                            "CQ_GPU_TILE: only tile(256,4) is supported",
                        ));
                    }
                }
                "frontier" => {
                    options.frontier = args.parse::<LitInt>()?.base10_parse()?;
                }
                "directory" => {
                    let directory: Ident = args.parse()?;
                    if directory != "rank" {
                        return Err(syn::Error::new_spanned(
                            directory,
                            "CQ_GPU_DIRECTORY: only directory(rank) is supported",
                        ));
                    }
                }
                _ => {
                    return Err(syn::Error::new_spanned(
                        option,
                        "CQ_GPU_ARGUMENT: unknown printer option",
                    ));
                }
            }
            if !args.is_empty() {
                return Err(args.error("CQ_GPU_ARGUMENT: unexpected option arguments"));
            }
            if input.is_empty() {
                break;
            }
            input.parse::<Token![,]>()?;
        }
        Ok(options)
    }
}
