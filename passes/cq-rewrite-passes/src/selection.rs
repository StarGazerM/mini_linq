use std::collections::BTreeSet;

use cq_lang_physical::symbol_name;
use proc_macro2::TokenStream;
use syn::parse::{Parse, ParseStream};
use syn::{Ident, Token};

/// Empty arguments and `all` select every admissible occurrence. Explicit
/// names must exist, have the right operator, and be admissible as requested.
pub(crate) enum Results {
    All,
    Named(BTreeSet<String>),
}

impl Parse for Results {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if input.is_empty() {
            return Ok(Self::All);
        }
        let names = input.parse_terminated(Ident::parse, Token![,])?;
        if names.len() == 1 && names[0] == "all" {
            return Ok(Self::All);
        }
        let mut selected = BTreeSet::new();
        for name in names {
            if name == "all" || !selected.insert(symbol_name(&name)) {
                return Err(syn::Error::new_spanned(name, "ambiguous result selection"));
            }
        }
        Ok(Self::Named(selected))
    }
}

impl Results {
    pub(crate) fn parse(tokens: TokenStream) -> syn::Result<Self> {
        syn::parse2(tokens)
    }

    pub(crate) fn includes(&self, name: &Ident) -> bool {
        match self {
            Self::All => true,
            Self::Named(names) => names.contains(&symbol_name(name)),
        }
    }

    pub(crate) fn require_matches(&self, matched: &BTreeSet<String>) -> syn::Result<()> {
        if let Self::Named(names) = self {
            if let Some(name) = names.difference(matched).next() {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    format!("result `{name}` is missing or inadmissible for this rewrite"),
                ));
            }
        }
        Ok(())
    }
}
