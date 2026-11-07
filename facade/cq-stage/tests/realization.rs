use cq_stage::{Annotation, Language, Sequence, Structures};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::{Token, parse::{Parse, ParseStream}};

#[derive(Clone)]
struct Physical(syn::Ident);
#[derive(Clone)]
struct Providers {
    name: syn::Ident,
    source: syn::Ident,
}
#[derive(Clone)]
struct Schedule {
    name: syn::Ident,
    source: syn::Ident,
    provider: syn::Ident,
}

impl Parse for Physical {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> { Ok(Self(input.parse()?)) }
}
impl Annotation<&Physical> for Providers {
    fn parse(input: ParseStream<'_>, _: &Physical) -> syn::Result<Self> {
        input.parse::<Token![;]>()?;
        let name = input.parse()?;
        input.parse::<Token![=]>()?;
        Ok(Self { name, source: input.parse()? })
    }
    fn check(&self, physical: &Physical) -> syn::Result<()> {
        if self.source == physical.0 {
            Ok(())
        } else {
            Err(syn::Error::new_spanned(&self.source, "unknown Physical reference"))
        }
    }
    fn check_rewrite(source_p: &Physical, source: &Self, target_p: &Physical, target: &Self) -> syn::Result<()> {
        Physical::check_rewrite(source_p, target_p)?;
        unchanged(source, target)
    }
}
impl Annotation<(&Physical, &Providers)> for Schedule {
    fn parse(input: ParseStream<'_>, _: (&Physical, &Providers)) -> syn::Result<Self> {
        input.parse::<Token![;]>()?;
        let name = input.parse()?;
        input.parse::<Token![=]>()?;
        let source = input.parse()?;
        input.parse::<Token![,]>()?;
        Ok(Self { name, source, provider: input.parse()? })
    }
    fn check(&self, (physical, providers): (&Physical, &Providers)) -> syn::Result<()> {
        if self.source != physical.0 {
            return Err(syn::Error::new_spanned(&self.source, "unknown Sequence Physical reference"));
        }
        if self.provider != providers.name {
            return Err(syn::Error::new_spanned(&self.provider, "unknown Structures reference"));
        }
        Ok(())
    }
    fn check_rewrite(source_base: (&Physical, &Providers), source: &Self,
        target_base: (&Physical, &Providers), target: &Self) -> syn::Result<()> {
        Providers::check_rewrite(source_base.0, source_base.1, target_base.0, target_base.1)?;
        unchanged(source, target)
    }
}
impl ToTokens for Physical {
    fn to_tokens(&self, out: &mut TokenStream) { self.0.to_tokens(out); }
}
impl ToTokens for Providers {
    fn to_tokens(&self, out: &mut TokenStream) {
        let Self { name, source } = self;
        out.extend(quote!(; #name = #source));
    }
}
impl ToTokens for Schedule {
    fn to_tokens(&self, out: &mut TokenStream) {
        let Self { name, source, provider } = self;
        out.extend(quote!(; #name = #source, #provider));
    }
}
fn unchanged(source: &impl ToTokens, target: &impl ToTokens) -> syn::Result<()> {
    if source.to_token_stream().to_string() == target.to_token_stream().to_string() {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(target, "rewrite changed the program"))
    }
}
impl Language for Physical {
    fn check(&self) -> syn::Result<()> { Ok(()) }
    fn check_rewrite(source: &Self, target: &Self) -> syn::Result<()> { unchanged(source, target) }
}
type Pair = Structures<Physical, Providers>;
type Triple = Sequence<Physical, Providers, Schedule>;

fn packet(output: TokenStream) -> TokenStream {
    syn::parse2::<syn::Macro>(output).unwrap().tokens
}

#[test]
fn structures_continuation_prints_separate_programs_and_forwards_arguments() {
    let output = Pair::rewrite(quote!(pipeline { next(opaque) }; input; hash = input), |p, s, args| {
        assert!(args.is_empty());
        assert_eq!(p.to_token_stream().to_string(), "input");
        assert_eq!(s.to_token_stream().to_string(), "; hash = input");
        Ok((p.clone(), s.clone()))
    });
    assert_eq!(output.to_string(), quote!(next! {
        arguments { opaque }; pipeline {};
        realization { physical { input } structures { ; hash = input } }
    }).to_string());
    let terminal = Pair::pass::<syn::Expr>(packet(output), |p, s, args| {
        assert_eq!(args.to_string(), "opaque");
        assert_eq!(p.0, s.source);
        Ok(syn::parse_quote!(42))
    });
    assert_eq!(terminal.to_string(), "42");
}

#[test]
fn sequence_consumers_receive_three_separate_programs() {
    let output = Triple::rewrite(quote!(pipeline { next }; input; hash = input; rows = input, hash), |p, s, q, _| {
        Ok((p.clone(), s.clone(), q.clone()))
    });
    assert_eq!(output.to_string(), quote!(next! { pipeline {};
        realization { physical { input } structures { ; hash = input } sequence { ; rows = input, hash } }
    }).to_string());
    let terminal = Triple::pass::<syn::Expr>(packet(output), |p, s, q, _| {
        assert_eq!(p.0, "input");
        assert_eq!(s.name, "hash");
        assert_eq!(q.name, "rows");
        Ok(syn::parse_quote!(42))
    });
    assert_eq!(terminal.to_string(), "42");
}

#[test]
fn mismatched_source_layers_never_reach_a_consumer() {
    for (stack, error) in [
        (quote!(realization { physical { other } structures { ; hash = input } sequence { ; rows = input, hash } }), "unknown Physical reference"),
        (quote!(realization { physical { input } structures { ; tree = input } sequence { ; rows = input, hash } }), "unknown Structures reference"),
        (quote!(realization { physical { input } structures { ; hash = input } sequence { ; rows = other, hash } }), "unknown Sequence Physical reference"),
    ] {
        let output = Triple::pass::<syn::Expr>(quote!(pipeline {}; #stack), |_, _, _, _| {
            panic!("mismatched source reached consumer")
        });
        assert!(output.to_string().contains(error));
    }
    let output = Pair::pass::<syn::Expr>(quote!(pipeline {}; input; hash = other), |_, _, _| {
        panic!("mismatched Structures source reached consumer")
    });
    assert!(output.to_string().contains("unknown Physical reference"));
}

#[test]
fn mismatched_target_layers_never_reach_a_continuation() {
    let output = Pair::rewrite(quote!(pipeline { next }; input; hash = input), |_, s, _| {
        Ok((syn::parse_quote!(other), s.clone()))
    });
    assert!(output.to_string().contains("unknown Physical reference"));
    let output = Triple::rewrite(quote!(pipeline { next }; input; hash = input; rows = input, hash), |p, s, q, _| {
        let mut s = s.clone();
        s.name = syn::parse_quote!(tree);
        Ok((p.clone(), s, q.clone()))
    });
    assert!(output.to_string().contains("unknown Structures reference"));
}

#[test]
fn rewrite_contract_receives_both_upper_stacks() {
    let output = Triple::rewrite(quote!(pipeline { next }; input; hash = input; rows = input, hash), |_, s, q, _| {
        let p: Physical = syn::parse_quote!(other);
        let mut s = s.clone();
        let mut q = q.clone();
        s.source = p.0.clone();
        q.source = p.0.clone();
        Ok((p, s, q))
    });
    assert!(output.to_string().contains("rewrite changed the program"));
}

#[test]
fn annotation_envelopes_reject_unconsumed_tokens() {
    for tokens in [
        quote!(realization { physical { input } structures { ; hash = input extra } sequence { ; rows = input, hash } }),
        quote!(realization { physical { input } structures { ; hash = input } sequence { ; rows = input, hash extra } }),
    ] {
        assert!(syn::parse2::<Triple>(tokens).is_err());
    }
    assert!(syn::parse2::<Pair>(quote!(realization {
        physical { input } structures { ; hash = input extra }
    })).is_err());
}

#[test]
fn observation_identifies_only_the_selected_program() {
    let output = Triple::rewrite(quote!(pipeline {} observe { inspect }; input; hash = input; rows = input, hash), |p, s, q, _| {
        Ok((p.clone(), s.clone(), q.clone()))
    });
    assert_eq!(output.to_string(), quote!(inspect! { ; rows = input, hash }).to_string());
    let stack: Triple = syn::parse_quote!(input; hash = input; rows = input, hash);
    assert_eq!(stack.program_tokens().to_string(), "; rows = input , hash");
    let pair = Pair::new(stack.physical, stack.structures);
    assert_eq!(pair.program_tokens().to_string(), "; hash = input");
}

#[test]
fn calibration_retains_selected_identity_and_separate_bases() {
    use cq_stage::calibration::Stage;
    let stack: Triple = syn::parse_quote!(input; hash = input; rows = input, hash);
    let stage = stack.calibration_stage();
    assert!(stage.is::<Schedule>());
    assert_eq!(stage.program, stack.program_tokens().to_string());
    assert_eq!(stage, Stage::annotation(&stack.sequence, vec![
        "input".into(), "; hash = input".into(),
    ]));
    let pair = Pair::new(stack.physical, stack.structures);
    let stage = pair.calibration_stage();
    assert!(stage.is::<Providers>());
    assert_eq!(stage.program, pair.program_tokens().to_string());
    assert_eq!(stage, Stage::annotation(&pair.structures, vec!["input".into()]));
}
