//! Positional occurrence paths, including Rust's float-literal tokenization.
//! A path is a clause followed by branch/clause pairs; its printed spelling
//! preserves source provenance through arbitrary nested blocks.

use quote::ToTokens;
use std::{fmt, str::FromStr};
use syn::{Token, parse::Parse};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Occurrence(pub Vec<String>);

impl Occurrence {
    pub fn new(clause: usize) -> Self {
        Self(vec![clause.to_string()])
    }
    pub fn child(&self, branch: usize, clause: usize) -> Self {
        let mut path = self.0.clone();
        path.extend([branch.to_string(), clause.to_string()]);
        Self(path)
    }
    pub fn clause(&self) -> usize {
        self.0
            .last()
            .expect("checked nonempty occurrence")
            .parse()
            .expect("checked occurrence index")
    }
    pub fn check(&self) -> syn::Result<()> {
        if self.0.is_empty() || self.0.len() % 2 != 1 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "occurrence path requires clause/branch/clause indices",
            ));
        }
        for part in &self.0 {
            part.parse::<usize>().map_err(|e| {
                syn::Error::new(
                    proc_macro2::Span::call_site(),
                    format!("occurrence index: {e}"),
                )
            })?;
        }
        Ok(())
    }
    pub fn base10_parse<T: FromStr>(&self) -> syn::Result<T>
    where
        T::Err: fmt::Display,
    {
        self.to_string()
            .parse()
            .map_err(|e| syn::Error::new(proc_macro2::Span::call_site(), e))
    }
}
impl From<syn::LitInt> for Occurrence {
    fn from(value: syn::LitInt) -> Self {
        Self(vec![value.base10_digits().to_owned()])
    }
}
impl From<usize> for Occurrence {
    fn from(value: usize) -> Self {
        Self::new(value)
    }
}
impl fmt::Display for Occurrence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, part) in self.0.iter().enumerate() {
            if i != 0 {
                f.write_str(".")?;
            }
            write!(f, "{part}")?;
        }
        Ok(())
    }
}
impl ToTokens for Occurrence {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        tokens.extend(
            self.to_string()
                .parse::<proc_macro2::TokenStream>()
                .expect("numeric path tokens"),
        );
    }
}
impl Parse for Occurrence {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let mut path = Vec::new();
        loop {
            let literal: proc_macro2::Literal = input.parse()?;
            let text = literal.to_string();
            if let Ok(integer) = syn::parse_str::<syn::LitInt>(&text) {
                let digits = integer.base10_digits().trim_start_matches('0');
                path.push(if digits.is_empty() {
                    "0".into()
                } else {
                    digits.into()
                });
            } else {
                for part in text.split('.') {
                    if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                        return Err(syn::Error::new(
                            literal.span(),
                            "occurrence path requires unsigned indices",
                        ));
                    }
                    let digits = part.trim_start_matches('0');
                    path.push(if digits.is_empty() {
                        "0".into()
                    } else {
                        digits.into()
                    });
                }
            }
            if !input.peek(Token![.]) {
                break;
            }
            input.parse::<Token![.]>()?;
        }
        if path.len() % 2 != 1 {
            return Err(input.error("occurrence path is a clause followed by branch/clause pairs"));
        }
        Ok(Self(path))
    }
}
