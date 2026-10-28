//! Branch step sequences retain the CQ tree. A block's subatom records the
//! visible columns selected by this read; its occurrence extends through every
//! branch. Fallback ordering is semantic, union siblings are independent.

use crate::{Step, Subatom, parse_steps};
use cq_lang_cq::blocks::Kind;
use quote::{ToTokens, quote};
use syn::{Token, token};

mod kw {
    syn::custom_keyword!(hoisted);
    syn::custom_keyword!(correlated);
}

/// Whether arms form an independent relation or consume the current prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    Hoisted,
    Correlated,
}

impl syn::parse::Parse for Placement {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        if input.peek(kw::hoisted) {
            input.parse::<kw::hoisted>()?;
            Ok(Self::Hoisted)
        } else {
            input.parse::<kw::correlated>()?;
            Ok(Self::Correlated)
        }
    }
}

impl ToTokens for Placement {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        tokens.extend(match self {
            Self::Hoisted => quote!(hoisted),
            Self::Correlated => quote!(correlated),
        });
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    pub brace: token::Brace,
    pub steps: Vec<Step>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub source: Subatom,
    pub placement: Placement,
    pub kind: Kind,
    pub branches: Vec<Branch>,
}
impl syn::parse::Parse for Block {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let source = input.parse()?;
        let placement = input.parse()?;
        let mut branches = Vec::new();
        let mut kind = None;
        loop {
            let contents;
            let brace = syn::braced!(contents in input);
            branches.push(Branch {
                brace,
                steps: parse_steps(&contents)?,
            });
            let next = if input.peek(Token![|]) {
                input.parse::<Token![|]>()?;
                Kind::Union
            } else if input.peek(Token![else]) {
                input.parse::<Token![else]>()?;
                Kind::Fallback
            } else {
                break;
            };
            if kind.is_some_and(|old| old != next) {
                return Err(input.error("mixed block operators require nesting"));
            }
            kind = Some(next);
        }
        Ok(Self {
            source,
            placement,
            kind: kind.ok_or_else(|| input.error("a valuation block requires two branches"))?,
            branches,
        })
    }
}
impl ToTokens for Block {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.source.to_tokens(tokens);
        self.placement.to_tokens(tokens);
        for (j, branch) in self.branches.iter().enumerate() {
            if j != 0 {
                tokens.extend(match self.kind {
                    Kind::Union => quote!(|),
                    Kind::Fallback => quote!(else),
                });
            }
            branch.brace.surround(tokens, |tokens| {
                for step in &branch.steps {
                    step.to_tokens(tokens);
                }
            });
        }
    }
}
