//! Test-only macros loading the packaged cq-testkit corpus at compile time.

use cq_stage::Pipeline;
use cq_testkit::{SCENARIOS, Scenario};
use proc_macro::TokenStream;
use quote::{ToTokens, format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::{Ident, Token};

struct Invocation {
    name: Ident,
    pipeline: Pipeline,
}

impl Parse for Invocation {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let name = input.parse()?;
        input.parse::<Token![;]>()?;
        let pipeline = input.parse()?;
        Ok(Self { name, pipeline })
    }
}

fn scenario(name: &Ident) -> syn::Result<&'static Scenario> {
    SCENARIOS
        .iter()
        .find(|s| name == s.name)
        .ok_or_else(|| syn::Error::new_spanned(name, "unknown corpus scenario"))
}

fn finish(result: syn::Result<proc_macro2::TokenStream>) -> TokenStream {
    result.unwrap_or_else(syn::Error::into_compile_error).into()
}

/// Load the published IX program and invoke the caller's explicit pass queue.
#[proc_macro]
pub fn corpus_ix(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as Invocation);
    finish((|| {
        let program = scenario(&input.name)?.parse_ix()?;
        input.pipeline.start(program.to_token_stream())
    })())
}

/// Load the published Physical program and invoke the explicit pass queue.
#[proc_macro]
pub fn corpus_physical(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as Invocation);
    finish((|| {
        let program = scenario(&input.name)?.parse_physical()?;
        input.pipeline.start(program.to_token_stream())
    })())
}

/// Load an authored, adjacent triangle plan for the active smallest rewrite.
#[proc_macro]
pub fn adjacent_triangle(input: TokenStream) -> TokenStream {
    let pipeline = syn::parse_macro_input!(input as Pipeline);
    finish((|| {
        let source: cq_testkit::physical::Program =
            syn::parse_str(include_str!("../../tests/fixtures/adjacent.physical"))?;
        pipeline.start(source.to_token_stream())
    })())
}

/// Load a mixed host/retained three-provider intersection through public passes.
#[proc_macro]
pub fn three_way(input: TokenStream) -> TokenStream {
    let pipeline = syn::parse_macro_input!(input as Pipeline);
    finish((|| {
        let source: cq_testkit::physical::Program =
            syn::parse_str(include_str!("../../tests/fixtures/three_way.physical"))?;
        pipeline.start(source.to_token_stream())
    })())
}

/// A test printer which captures the exact checked IR before Rust realization.
#[proc_macro]
pub fn capture_physical(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_testkit::physical::Program, syn::Expr>(input.into(), |p, _| {
        let text = p.to_token_stream().to_string();
        Ok(syn::parse_quote!(#text))
    })
    .into()
}

/// Capture checked Structures IR; compile pipelines must set `print_input = structures`.
#[proc_macro]
pub fn capture_structures(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_testkit::structures::Program, syn::Expr>(input.into(), |p, _| {
        let text = p.to_token_stream().to_string();
        Ok(syn::parse_quote!(#text))
    })
    .into()
}

/// Generate compile-and-run tests from cq-testkit's raw inputs (duplicates
/// retained), with every stage entered through the supplied macro queue.
#[proc_macro]
pub fn corpus_tests(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as Invocation);
    finish((|| {
        let mut tests = proc_macro2::TokenStream::new();
        for scenario in SCENARIOS {
            let physical = scenario.parse_physical()?;
            let source = match input.name.to_string().as_str() {
                "ix" => scenario.parse_ix()?.to_token_stream(),
                "physical" => physical.to_token_stream(),
                _ => {
                    return Err(syn::Error::new_spanned(
                        input.name,
                        "expected ix or physical",
                    ));
                }
            };
            let expression = input.pipeline.clone().start(source)?;
            let name = scenario.name;
            let test_name = format_ident!("{name}");
            let case = scenario.case().expect("corpus case");
            let mut bindings = proc_macro2::TokenStream::new();
            for declaration in &physical.program.declarations {
                let name = &declaration.name;
                let (_, rows) = case.inputs.iter().find(|(n, _)| name == *n).expect("input");
                let types = declaration.columns.iter().map(|c| &c.ty);
                let values = rows.iter().map(|row| quote!((#(#row,)*)));
                bindings.extend(quote! {
                    let #name: Vec<(#(#types,)*)> = vec![#(#values),*];
                });
            }
            let columns: Vec<_> = (0..physical.program.head.args.len())
                .map(|i| format_ident!("col_{i}"))
                .collect();
            tests.extend(quote! {
                #[test]
                #[allow(non_snake_case)]
                fn #test_name() {
                    #bindings
                    let output: Vec<_> = (#expression).collect();
                    let count = output.len();
                    let actual: cq_testkit::Rows = output.into_iter()
                        .map(|(#(#columns,)*)| vec![#(*#columns),*]).collect();
                    assert_eq!(count, actual.len(), "duplicate answer from {}", #name);
                    let scenario = cq_testkit::SCENARIOS.iter().find(|s| s.name == #name).unwrap();
                    let case = scenario.case().unwrap();
                    assert_eq!(actual, case.expected_rows(), "{}", #name);
                    assert_eq!(actual, cq_testkit::cq_semantics::evaluate_cq(
                        &scenario.parse_cq().unwrap(), &case.database()));
                }
            });
        }
        Ok(tests)
    })())
}
