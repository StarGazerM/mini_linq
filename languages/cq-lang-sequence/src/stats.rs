//! BE-StatsRead: read-only, typed references to completed build publications.
//!
//! Statistic guards are advice only: they cannot occur in lengths, capacities,
//! uniqueness premises, or truncation bounds. Missing values retain the explicit
//! checked fallback. This first contract admits host scalar reads only; device
//! transfer/visibility records belong to the device publication implementation.

// DESIGN build-statistics
// STR-BuildStats owns representation truth; BE-StatsRead owns readiness.
// Source: doc/STORAGE.md §13; doc/PLANNING-REDESIGN.md P7.1.

use cq_lang_structures::stats::Reference;
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::parse::{Parse, ParseStream};
use syn::{Ident, LitInt, Token, parenthesized};

pub(crate) mod kw {
    syn::custom_keyword!(stat);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Comparison {
    Lt,
    Ge,
}

/// A scalar comparison with its publication identity and dominating Store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Guard {
    pub ready: Ident,
    pub reference: Reference,
    pub comparison: Comparison,
    pub threshold: u64,
}

impl Parse for Guard {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<kw::stat>()?;
        let ready = input.parse()?;
        let body;
        parenthesized!(body in input);
        let reference = body.parse()?;
        if !body.is_empty() {
            return Err(body.error("expected a complete statistic reference"));
        }
        let comparison = if input.peek(Token![<]) {
            input.parse::<Token![<]>()?;
            Comparison::Lt
        } else {
            input.parse::<Token![>=]>()?;
            Comparison::Ge
        };
        let threshold = input.parse::<LitInt>()?.base10_parse()?;
        Ok(Self {
            ready,
            reference,
            comparison,
            threshold,
        })
    }
}

impl ToTokens for Guard {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let Self {
            ready,
            reference,
            comparison,
            threshold,
        } = self;
        let threshold = LitInt::new(&threshold.to_string(), proc_macro2::Span::call_site());
        let comparison = match comparison {
            Comparison::Lt => quote!(<),
            Comparison::Ge => quote!(>=),
        };
        tokens.extend(quote!(stat #ready (#reference) #comparison #threshold));
    }
}
