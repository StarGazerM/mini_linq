//! Optional Datalog-owned partitioning adapter for the public pull printer.
//!
//! `print_hole(partition(N))` uses one of the facade's sixteen separately
//! monomorphized helpers; omitted arguments select partition zero. This is not
//! the default terminal. A Round rewrite can assign partitions independently
//! without changing CQ, the pull interface, or the terminal registry.
//!
//! The public `cq_physical_realization` entry points are the same ones used by
//! `mini_linq::passes::print_pull`. Both ordinary and counter-enabled output are
//! retained verbatim inside the wrapper. The wrapper adds no row allocation,
//! collection, cloning, or ownership conversion. Its non-move closure preserves
//! the original expression's capture and borrow behavior. Iterator consumption
//! may still be instantiated by the caller; partitioning is not a performance
//! guarantee.

use proc_macro2::TokenStream;
use quote::format_ident;
use syn::parse::{Parse, ParseStream};

struct Partition(usize);

impl Parse for Partition {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if input.is_empty() {
            return Ok(Self(0));
        }
        let name: syn::Ident = input.parse()?;
        if name != "partition" {
            return Err(syn::Error::new_spanned(name, "expected partition(N)"));
        }
        let contents;
        syn::parenthesized!(contents in input);
        let number: syn::LitInt = contents.parse()?;
        let partition = number.base10_parse::<usize>()?;
        if partition >= 16 {
            return Err(syn::Error::new_spanned(
                number,
                "partition must be in 0..16",
            ));
        }
        if !contents.is_empty() {
            return Err(contents.error("expected one partition index"));
        }
        Ok(Self(partition))
    }
}

fn wrap(expression: syn::Expr, partition: Partition) -> syn::Expr {
    let module = format_ident!("partition_{}", partition.0);
    syn::parse_quote!(
        ::cq_datalog::__codegen::#module::run((), |_| { #expression })
    )
}

/// Print checked Structures through the public pull API, then partition it.
/// Arguments are empty or `partition(N)`, where `N` is between zero and fifteen.
pub fn compile_with(
    source: &cq_lang_structures::Program,
    arguments: TokenStream,
) -> syn::Result<syn::Expr> {
    let partition = syn::parse2(arguments)?;
    let plain = cq_physical_realization::compile(source)?;
    let profile = cq_physical_realization::compile_profile(source)?;
    Ok(wrap(
        syn::parse_quote!(::cq_storage::profile_select! { plain { #plain } counters { #profile } }),
        partition,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::{ToTokens, quote};

    #[test]
    fn every_partition_and_default_are_available() {
        assert_eq!(syn::parse2::<Partition>(TokenStream::new()).unwrap().0, 0);
        for index in 0..16 {
            let parsed: Partition = syn::parse2(quote!(partition(#index))).unwrap();
            assert_eq!(parsed.0, index);
            let printed = wrap(syn::parse_quote!(rows.iter()), parsed);
            let module = format_ident!("partition_{index}");
            let expected: syn::Expr = syn::parse_quote!(
                ::cq_datalog::__codegen::#module::run((), |_| { rows.iter() })
            );
            assert_eq!(printed, expected);
        }
    }

    #[test]
    fn wrapping_preserves_borrows_and_opaque_tokens_exactly() {
        let expression: syn::Expr = syn::parse_quote!({
            let borrowed = &rows;
            borrowed.iter().filter(|row| predicate(row, &captured))
        });
        let result = wrap(expression.clone(), Partition(7));
        let syn::Expr::Call(call) = result else {
            panic!("expected helper call")
        };
        assert_eq!(call.args.len(), 2);
        let syn::Expr::Closure(closure) = &call.args[1] else {
            panic!("expected closure")
        };
        assert!(closure.capture.is_none());
        let expected: syn::Expr = syn::parse_quote!({ #expression });
        assert_eq!(*closure.body, expected);
        assert!(!call.to_token_stream().to_string().contains("collect"));
    }

    #[test]
    fn malformed_or_out_of_range_partitions_are_rejected() {
        for args in [
            quote!(partition(16)),
            quote!(partition(-1)),
            quote!(partition(1, 2)),
            quote!(partition(1) extra),
            quote!(unknown(0)),
        ] {
            assert!(syn::parse2::<Partition>(args).is_err());
        }
    }
}
