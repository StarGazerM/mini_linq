//! The author-facing serialization of the planner's checked trace.
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::parse::{Parse, ParseStream};
use syn::{Ident, LitInt, Token, parenthesized};

use crate::Advice;

impl Parse for Advice {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let fork = input.fork();
        let kind: Ident = fork.parse()?;
        if kind == "evidence" {
            return input.parse().map(Self::Evidence);
        }
        let kind: Ident = input.parse()?;
        let arguments;
        parenthesized!(arguments in input);
        let result = match kind.to_string().as_str() {
            "store" => Self::Store(arguments.parse()?),
            "group_without_canon" => Self::GroupWithoutCanon(arguments.parse()?),
            "key_partition" => Self::KeyPartition(arguments.parse()?),
            "store_level" => Self::StoreLevel(arguments.parse()?),
            "publish" => {
                let name = arguments.parse()?;
                arguments.parse::<Token![,]>()?;
                Self::Publish {
                    name,
                    producer: arguments.parse()?,
                }
            }
            "tile" | "partition" | "segment" => {
                let name = arguments.parse()?;
                arguments.parse::<Token![,]>()?;
                let n = arguments.parse::<LitInt>()?.base10_parse()?;
                let items = if arguments.peek(Token![,]) {
                    arguments.parse::<Token![,]>()?;
                    let keyword: Ident = arguments.parse()?;
                    if keyword != "items" {
                        return Err(syn::Error::new(keyword.span(), "expected items"));
                    }
                    Some(arguments.parse::<LitInt>()?.base10_parse()?)
                } else {
                    None
                };
                match kind.to_string().as_str() {
                    "tile" => Self::Tile { name, n, items },
                    "partition" => Self::Partition { name, n, items },
                    _ => Self::Segment { name, n, items },
                }
            }
            "output" => Self::Output(arguments.parse()?),
            "grade" => {
                let machine = arguments.parse()?;
                arguments.parse::<Token![,]>()?;
                let name = arguments.parse()?;
                let keyword: Ident = arguments.parse()?;
                if keyword != "at" {
                    return Err(syn::Error::new(keyword.span(), "expected at"));
                }
                let level = arguments
                    .parse_terminated(Ident::parse, Token![.])?
                    .into_iter()
                    .collect();
                Self::Grade {
                    name,
                    level,
                    machine,
                }
            }
            _ => return Err(syn::Error::new(kind.span(), "unknown Sequence advice")),
        };
        if !arguments.is_empty() {
            return Err(arguments.error("unexpected advice argument"));
        }
        Ok(result)
    }
}

impl ToTokens for Advice {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let invocation = match self {
            Self::Evidence(evidence) => quote!(#evidence),
            Self::Store(name) => quote!(store(#name)),
            Self::GroupWithoutCanon(name) => quote!(group_without_canon(#name)),
            Self::StoreLevel(name) => quote!(store_level(#name)),
            Self::KeyPartition(arguments) => quote!(key_partition(#arguments)),
            Self::Publish { name, producer } => quote!(publish(#name, #producer)),
            Self::Tile { name, n, items }
            | Self::Partition { name, n, items }
            | Self::Segment { name, n, items } => {
                let kind = match self {
                    Self::Tile { .. } => "tile",
                    Self::Partition { .. } => "partition",
                    _ => "segment",
                };
                let kind = Ident::new(kind, name.span());
                let n = LitInt::new(&n.to_string(), name.span());
                let items = items.map(|items| {
                    let items = LitInt::new(&items.to_string(), name.span());
                    quote!(, items #items)
                });
                quote!(#kind(#name, #n #items))
            }
            Self::Output(protocol) => quote!(output(#protocol)),
            Self::Grade {
                name,
                level,
                machine,
            } => quote!(grade(#machine, #name at #(#level).*)),
        };
        tokens.extend(invocation);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authored_advice_is_the_serialized_planner_trace() {
        for invocation in [
            "store(result)",
            "group_without_canon(grouped)",
            "store_level(joined)",
            "publish(result, producer)",
            "tile(result, 256)",
            "partition(result, 256, items 4)",
            "segment(result, 1024, items 4)",
            "output(reserve)",
            "grade(gpu, result at host.device)",
            "evidence { count R = 42; }",
            "key_partition(grouped, published, [region, year], 8)",
            "key_partition(provider, published, [key], 8, threshold 65536)",
        ] {
            let advice: Advice = syn::parse_str(invocation).unwrap();
            assert_eq!(advice, syn::parse2(advice.to_token_stream()).unwrap());
        }
    }

    #[test]
    fn retired_relational_replanning_advice_is_rejected() {
        for invocation in [
            "lookup_first(using Index, order [r,s])",
            "adaptive(filtered, published, 100000)",
        ] {
            assert!(syn::parse_str::<Advice>(invocation).is_err());
        }
    }
}
