//! Recorded upstream ordering inputs: planner options (`PlanOptions` `ordering(...)`)
//! recorded unchanged in VO. CQ has no ordering syntax; these facts rank nothing.
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::{Ident, Token, parse::{Parse, ParseStream}};

pub mod kw {
    syn::custom_keyword!(ordering);
    syn::custom_keyword!(seed);
    syn::custom_keyword!(delta);
    syn::custom_keyword!(driver);
    syn::custom_keyword!(permutation);
    syn::custom_keyword!(shared_demand);
    syn::custom_keyword!(none);
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DriverPolicy {
    #[default]
    WrittenOrder,
    DeltaFirst,
}
impl Parse for DriverPolicy {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let name: Ident = input.parse()?;
        match name.to_string().as_str() {
            "WrittenOrder" => Ok(Self::WrittenOrder),
            "DeltaFirst" => Ok(Self::DeltaFirst),
            _ => Err(syn::Error::new_spanned(name, "expected WrittenOrder or DeltaFirst")),
        }
    }
}
impl ToTokens for DriverPolicy {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        tokens.extend(match self {
            Self::WrittenOrder => quote!(WrittenOrder),
            Self::DeltaFirst => quote!(DeltaFirst),
        });
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SharedDemand {
    pub relation: Ident,
    pub columns: Vec<usize>,
}
impl Parse for SharedDemand {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let relation = input.parse()?;
        let columns;
        syn::parenthesized!(columns in input);
        Ok(Self {
            relation,
            columns: columns.parse_terminated(number, Token![,])?.into_iter().collect(),
        })
    }
}
impl ToTokens for SharedDemand {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let relation = &self.relation;
        let columns = self.columns.iter().copied().map(syn::Index::from);
        tokens.extend(quote!(#relation(#(#columns),*)));
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderingFacts {
    /// Original source-body occurrence, before the recorded permutation.
    pub delta: Option<usize>,
    pub seed: bool,
    pub driver: DriverPolicy,
    /// Complete source-body positions in Incremental execution order.
    /// Normalization may add clauses; these remain upstream occurrence IDs.
    pub permutation: Vec<usize>,
    /// Optional stratum index demands; recorded only, with no ranking effect.
    pub shared_demand: Option<Vec<SharedDemand>>,
}
pub fn parse_optional(input: ParseStream<'_>) -> syn::Result<Option<OrderingFacts>> {
    if input.peek(kw::ordering) {
        input.parse().map(Some)
    } else {
        Ok(None)
    }
}

impl OrderingFacts {
    /// Formal rule VO-OrderingInputs: The permutation is a bijection on its positions, seed iff no delta, and a delta indexes the permutation.
    pub fn check(&self) -> syn::Result<()> {
        let mut seen = vec![false; self.permutation.len()];
        for &position in &self.permutation {
            let Some(slot) = seen.get_mut(position) else {
                return Err(syn::Error::new_spanned(self, "VO-OrderingInputs: permutation is not a bijection"));
            };
            if std::mem::replace(slot, true) {
                return Err(syn::Error::new_spanned(self, "VO-OrderingInputs: permutation is not a bijection"));
            }
        }
        if self.seed != self.delta.is_none()
            || self.delta.is_some_and(|delta| delta >= self.permutation.len())
        {
            return Err(syn::Error::new_spanned(self, "VO-OrderingInputs: seed and delta facts disagree"));
        }
        Ok(())
    }
}
fn number(input: ParseStream<'_>) -> syn::Result<usize> {
    input.parse::<syn::LitInt>()?.base10_parse()
}
impl Parse for OrderingFacts {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<kw::ordering>()?;
        let body;
        syn::parenthesized!(body in input);
        body.parse::<kw::seed>()?;
        body.parse::<Token![=]>()?;
        let seed = body.parse::<syn::LitBool>()?.value;
        body.parse::<Token![,]>()?;
        body.parse::<kw::delta>()?;
        body.parse::<Token![=]>()?;
        let delta = if body.peek(kw::none) {
            body.parse::<kw::none>()?;
            None
        } else {
            Some(number(&body)?)
        };
        body.parse::<Token![,]>()?;
        body.parse::<kw::driver>()?;
        body.parse::<Token![=]>()?;
        let driver = body.parse()?;
        body.parse::<Token![,]>()?;
        body.parse::<kw::permutation>()?;
        body.parse::<Token![=]>()?;
        let permutation;
        syn::bracketed!(permutation in body);
        let permutation = permutation.parse_terminated(number, Token![,])?.into_iter().collect();
        body.parse::<Token![,]>()?;
        body.parse::<kw::shared_demand>()?;
        body.parse::<Token![=]>()?;
        let shared_demand = if body.peek(kw::none) {
            body.parse::<kw::none>()?;
            None
        } else {
            let demands;
            syn::bracketed!(demands in body);
            Some(demands.parse_terminated(SharedDemand::parse, Token![,])?.into_iter().collect())
        };
        if !body.is_empty() {
            return Err(body.error("unexpected ordering facts"));
        }
        Ok(Self { delta, seed, driver, permutation, shared_demand })
    }
}
impl ToTokens for OrderingFacts {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let seed = self.seed;
        let delta = self.delta.map_or_else(|| quote!(none), |value| {
            let value = syn::Index::from(value);
            quote!(#value)
        });
        let driver = self.driver;
        let permutation = self.permutation.iter().copied().map(syn::Index::from);
        let shared_demand = self.shared_demand.as_ref().map_or_else(
            || quote!(none),
            |demands| quote!([#(#demands),*]),
        );
        tokens.extend(quote!(ordering(seed = #seed, delta = #delta, driver = #driver,
            permutation = [#(#permutation),*], shared_demand = #shared_demand)));
    }
}
