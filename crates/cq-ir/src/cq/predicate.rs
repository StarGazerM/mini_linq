//! A deliberately total selection fragment: comparisons of i32 variables/literals.
//! This is an optimizer recognizer, not the CQ expression grammar. CQ and RA
//! retain syn::Expr; only this proven total subset is eligible for movement.
use crate::symbol_name;
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use std::collections::BTreeSet;
use syn::{
    Ident, Token,
    parse::{Parse, ParseStream},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Term {
    Variable(Ident),
    Integer(i32),
}
impl Parse for Term {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if input.peek(Ident) {
            return Ok(Self::Variable(input.parse()?));
        }
        let minus: Option<Token![-]> = input.parse()?;
        let integer: syn::LitInt = input.parse()?;
        if !integer.suffix().is_empty() && integer.suffix() != "i32" {
            return Err(syn::Error::new_spanned(
                integer,
                "selection literals must be i32",
            ));
        }
        let magnitude = integer.base10_parse::<i64>()?;
        let value = if minus.is_some() {
            -magnitude
        } else {
            magnitude
        };
        let value = i32::try_from(value)
            .map_err(|_| syn::Error::new_spanned(&integer, "selection literal exceeds i32"))?;
        Ok(Self::Integer(value))
    }
}
impl ToTokens for Term {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        match self {
            Self::Variable(v) => v.to_tokens(tokens),
            Self::Integer(value) => value.to_tokens(tokens),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Comparison {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}
impl Parse for Comparison {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if input.peek(Token![==]) {
            input.parse::<Token![==]>()?;
            Ok(Self::Eq)
        } else if input.peek(Token![!=]) {
            input.parse::<Token![!=]>()?;
            Ok(Self::Ne)
        } else if input.peek(Token![<=]) {
            input.parse::<Token![<=]>()?;
            Ok(Self::Le)
        } else if input.peek(Token![>=]) {
            input.parse::<Token![>=]>()?;
            Ok(Self::Ge)
        } else if input.peek(Token![<]) {
            input.parse::<Token![<]>()?;
            Ok(Self::Lt)
        } else if input.peek(Token![>]) {
            input.parse::<Token![>]>()?;
            Ok(Self::Gt)
        } else {
            Err(input.error("expected a comparison operator"))
        }
    }
}
impl ToTokens for Comparison {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        tokens.extend(match self {
            Self::Eq => quote!(==),
            Self::Ne => quote!(!=),
            Self::Lt => quote!(<),
            Self::Le => quote!(<=),
            Self::Gt => quote!(>),
            Self::Ge => quote!(>=),
        });
    }
}
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Predicate {
    pub left: Term,
    pub comparison: Comparison,
    pub right: Term,
}
impl Predicate {
    pub fn variables(&self) -> BTreeSet<String> {
        [&self.left, &self.right]
            .into_iter()
            .filter_map(|term| match term {
                Term::Variable(v) => Some(symbol_name(v)),
                Term::Integer(_) => None,
            })
            .collect()
    }
    /// Name availability is local. As with join keys, Rust resolves column
    /// types and aliases. Lowering explicitly requires primitive i32 operands,
    /// so overloaded comparisons cannot make a movable predicate effectful.
    pub fn check_scope(&self, bound: &BTreeSet<String>) -> syn::Result<()> {
        if let Some(variable) = self.variables().difference(bound).next() {
            return Err(syn::Error::new_spanned(
                self,
                format!("predicate variable `{variable}` is unavailable"),
            ));
        }
        Ok(())
    }
}
