use cq_stage::{Language, Sequence, Structures};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::{Token, parse::{Parse, ParseStream}};

#[derive(Clone)]
struct Physical(syn::LitInt);
#[derive(Clone)]
struct Providers(Physical, syn::Ident);
#[derive(Clone)]
struct Schedule(Providers, syn::Ident);

impl Parse for Physical {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> { Ok(Self(input.parse()?)) }
}
impl Parse for Providers {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let physical = input.parse()?;
        input.parse::<Token![;]>()?;
        Ok(Self(physical, input.parse()?))
    }
}
impl Parse for Schedule {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let providers = input.parse()?;
        input.parse::<Token![;]>()?;
        Ok(Self(providers, input.parse()?))
    }
}
impl ToTokens for Physical {
    fn to_tokens(&self, out: &mut TokenStream) { self.0.to_tokens(out); }
}
impl ToTokens for Providers {
    fn to_tokens(&self, out: &mut TokenStream) {
        let Self(p, s) = self;
        out.extend(quote!(#p; #s));
    }
}
impl ToTokens for Schedule {
    fn to_tokens(&self, out: &mut TokenStream) {
        let Self(s, q) = self;
        out.extend(quote!(#s; #q));
    }
}
macro_rules! language {
    ($($ty:ty),*) => { $(impl Language for $ty {
        fn check(&self) -> syn::Result<()> { Ok(()) }
        fn check_rewrite(source: &Self, target: &Self) -> syn::Result<()> {
            if source.to_token_stream().to_string() == target.to_token_stream().to_string() {
                Ok(())
            } else {
                Err(syn::Error::new_spanned(target, "rewrite changed the program"))
            }
        }
    })* };
}
language!(Physical, Providers, Schedule);
type Pair = Structures<Physical, Providers>;
type Triple = Sequence<Physical, Providers, Schedule>;

fn packet(output: TokenStream) -> TokenStream {
    syn::parse2::<syn::Macro>(output).unwrap().tokens
}

#[test]
fn structures_continuation_prints_separate_programs_and_forwards_arguments() {
    let output = Pair::rewrite(quote!(pipeline { next(opaque) }; 1; hash), |p, s, args| {
        assert!(args.is_empty());
        assert_eq!(p.to_token_stream().to_string(), "1");
        assert_eq!(s.to_token_stream().to_string(), "1 ; hash");
        Ok((p.clone(), s.clone()))
    });
    assert_eq!(output.to_string(), quote!(next! {
        arguments { opaque }; pipeline {};
        realization { physical { 1 } structures { 1; hash } }
    }).to_string());
    let terminal = Pair::pass::<syn::Expr>(packet(output), |p, s, args| {
        assert_eq!(args.to_string(), "opaque");
        assert_eq!(p.to_token_stream().to_string(), s.0.to_token_stream().to_string());
        Ok(syn::parse_quote!(42))
    });
    assert_eq!(terminal.to_string(), "42");
}

#[test]
fn sequence_consumers_receive_three_separate_programs() {
    let output = Triple::rewrite(quote!(pipeline { next }; 1; hash; rows), |p, s, q, _| {
        Ok((p.clone(), s.clone(), q.clone()))
    });
    assert_eq!(output.to_string(), quote!(next! { pipeline {};
        realization { physical { 1 } structures { 1; hash } sequence { 1; hash; rows } }
    }).to_string());
    let terminal = Triple::pass::<syn::Expr>(packet(output), |p, s, q, _| {
        assert_eq!(p.to_token_stream().to_string(), "1");
        assert_eq!(s.1, "hash");
        assert_eq!(q.1, "rows");
        Ok(syn::parse_quote!(42))
    });
    assert_eq!(terminal.to_string(), "42");
}

#[test]
fn mismatched_source_layers_never_reach_a_consumer() {
    for stack in [
        quote!(realization { physical { 2 } structures { 1; hash } sequence { 1; hash; rows } }),
        quote!(realization { physical { 1 } structures { 1; tree } sequence { 1; hash; rows } }),
    ] {
        let output = Triple::pass::<syn::Expr>(quote!(pipeline {}; #stack), |_, _, _, _| {
            panic!("mismatched source reached consumer")
        });
        assert!(output.to_string().contains("ST-Realization"));
    }
}

#[test]
fn mismatched_target_layers_never_reach_a_continuation() {
    let output = Pair::rewrite(quote!(pipeline { next }; 1; hash), |_, s, _| {
        Ok((syn::parse_quote!(2), s.clone()))
    });
    assert!(output.to_string().contains("ST-Realization"));
    let output = Triple::rewrite(quote!(pipeline { next }; 1; hash; rows), |p, _, q, _| {
        Ok((p.clone(), syn::parse_quote!(1; tree), q.clone()))
    });
    assert!(output.to_string().contains("ST-Realization"));
}

#[test]
fn observation_and_calibration_identify_only_the_selected_program() {
    let output = Triple::rewrite(quote!(pipeline {} observe { inspect }; 1; hash; rows), |p, s, q, _| {
        Ok((p.clone(), s.clone(), q.clone()))
    });
    assert_eq!(output.to_string(), quote!(inspect! { 1; hash; rows }).to_string());
    let stack: Triple = syn::parse_quote!(1; hash; rows);
    assert_eq!(stack.calibration_stage(), cq_stage::calibration::Stage::of(&stack.sequence));
    assert_eq!(stack.program_tokens().to_string(), "1 ; hash ; rows");
}
