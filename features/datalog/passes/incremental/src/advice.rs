//! The shared body grammar and lossless printer for authored `plan(...)` advice.
//! Macro front ends and offline advice tools use this same parser.
use crate::{Known, OrderAtom, RulePlan};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::{Token, parse::{Parse, ParseStream}};

mod kw {
    syn::custom_keyword!(delta);
    syn::custom_keyword!(order);
}

impl Parse for RulePlan {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let rule = input.parse()?;
        input.parse::<Token![,]>()?;
        let delta = if input.peek(kw::delta) {
            input.parse::<kw::delta>()?;
            let selector;
            syn::parenthesized!(selector in input);
            let delta = selector.parse()?;
            if !selector.is_empty() {
                return Err(selector.error("expected one delta(Relation@clause)"));
            }
            input.parse::<Token![,]>()?;
            Some(delta)
        } else { None };
        let order = if input.peek(kw::order) {
            input.parse::<kw::order>()?;
            let selectors;
            syn::parenthesized!(selectors in input);
            let order = selectors.parse_terminated(OrderAtom::parse, Token![,])?
                .into_iter().collect::<Vec<_>>();
            if order.is_empty() { return Err(input.error("plan order(...) names no atom")); }
            if !input.is_empty() { input.parse::<Token![,]>()?; }
            order
        } else { Vec::new() };
        let mut smallest = Vec::new();
        let mut known = None;
        while !input.is_empty() {
            let name: syn::Ident = input.parse()?;
            let site;
            syn::parenthesized!(site in input);
            if name == "smallest" && !order.is_empty() {
                smallest.push(site.parse_terminated(OrderAtom::parse, Token![,])?.into_iter().collect());
            } else if name == "known" {
                if known.is_some() {
                    return Err(syn::Error::new_spanned(name, "duplicate or conflicting known(...) advice for one rule version"));
                }
                known = Some(site.parse::<Known>()?);
                if !site.is_empty() { return Err(site.error("expected known(admit) or known(at(level k))")); }
            } else if name == "smallest" {
                return Err(syn::Error::new_spanned(name, "smallest(...) requires order(...) advice"));
            } else {
                return Err(syn::Error::new_spanned(name, "expected order(..), smallest(Relation@clause, ...) or known(admit | at(level k))"));
            }
            if !input.is_empty() { input.parse::<Token![,]>()?; }
        }
        if order.is_empty() && known.is_none() { return Err(input.error("plan advice needs order(...) or known(...)")); }
        Ok(Self { rule, delta, order, smallest, known })
    }
}
fn selector(selector: &OrderAtom) -> TokenStream {
    let relation = &selector.relation;
    let occurrence = selector.occurrence.map(|index| {
        let index = syn::Index::from(index);
        quote!(@ #index)
    });
    let columns = selector.columns.as_ref().map(|columns| quote!((#(#columns),*)));
    quote!(#relation #occurrence #columns)
}
impl ToTokens for RulePlan {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let rule = &self.rule;
        let mut clauses = Vec::new();
        if let Some(delta) = &self.delta {
            let delta = selector(delta);
            clauses.push(quote!(delta(#delta)));
        }
        if !self.order.is_empty() {
            let order = self.order.iter().map(selector);
            clauses.push(quote!(order(#(#order),*)));
        }
        for site in &self.smallest {
            let selectors = site.iter().map(selector);
            clauses.push(quote!(smallest(#(#selectors),*)));
        }
        if let Some(known) = self.known { clauses.push(quote!(known(#known))); }
        tokens.extend(quote!(#rule, #(#clauses),*));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_advice_body_roundtrips_without_losing_occurrence_or_phase() {
        for body in [
            "Q, order(R@0,S@2)",
            "Q, delta(R@2), order(R@2,S@0(x),S@0), smallest(R@2,S@0), known(at(level 2))",
            "Q, delta(R@1), known(admit)",
        ] {
            let plan: RulePlan = syn::parse_str(body).unwrap();
            assert_eq!(syn::parse2::<RulePlan>(plan.to_token_stream()).unwrap(), plan);
        }
        for body in ["Q, order()", "Q, smallest(R@0,S@1)", "Q, known(admit), known(admit)"] {
            assert!(syn::parse_str::<RulePlan>(body).is_err());
        }
    }
}
