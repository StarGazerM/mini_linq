//! Datalog stage entrypoints over the ordinary checked CQ macro protocol.
//!
//! Each entry accepts an optional named MiniLinq `pipeline { ... }` followed
//! by its authored Datalog program. An explicit pass queue is also accepted.
//! `datalog!` schedules source, `stratified!` resumes checked strata, and
//! `seminaive!` resumes a complete schedule. Every CQ hole receives the same
//! caller continuation, including unchanged opaque pass arguments. The bridge
//! owns sets, grouped host layouts and rounds; it adds no planner options.

use proc_macro::TokenStream;
use quote::ToTokens;
use syn::parse::{Parse, ParseStream};
use syn::{Token, braced};

mod render;

mod kw {
    syn::custom_keyword!(arguments);
    syn::custom_keyword!(pipeline);
}

struct Input {
    pipeline: cq_stage::Pipeline,
    program: cq_lang_datalog::Program,
}

fn defaults() -> cq_stage::Configuration {
    syn::parse_quote! { pipeline {
        plan = ::mini_linq::passes::plan;
        ix = ::mini_linq::passes::index_demands;
        lower = ::mini_linq::passes::lower;
        select = ::mini_linq::passes::select;
        print = ::mini_linq::passes::print_pull;
    } }
}

impl Parse for Input {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if input.peek(kw::arguments) {
            input.parse::<kw::arguments>()?;
            let args;
            braced!(args in input);
            if !args.is_empty() {
                return Err(args.error("Datalog entries do not accept arguments"));
            }
            input.parse::<Token![;]>()?;
        }
        let pipeline = if input.peek(kw::pipeline) {
            let look = input.fork();
            look.parse::<kw::pipeline>()?;
            let body;
            braced!(body in look);
            let named = if body.is_empty() {
                true
            } else {
                let first = body.fork();
                first.parse::<syn::Ident>().is_ok() && first.peek(Token![=])
            };
            if named {
                input.parse::<cq_stage::Configuration>()?.resolve(defaults())?
            } else {
                input.parse::<cq_stage::Pipeline>()?
            }
        } else {
            defaults().resolve(cq_stage::Configuration::default())?
        };
        if input.peek(Token![;]) { input.parse::<Token![;]>()?; }
        Ok(Self { pipeline, program: input.parse()? })
    }
}

#[derive(Clone, Copy)]
enum Stage { Source, Stratified, Seminaive }

fn expand(tokens: proc_macro2::TokenStream, stage: Stage) -> syn::Result<proc_macro2::TokenStream> {
    let Input { pipeline, mut program } = syn::parse2(tokens)?;
    cq_lang_datalog::contract::check(&program)?;
    let valid = match stage {
        Stage::Source => program.strata.is_none() && program.variants.is_none(),
        Stage::Stratified => program.strata.is_some() && program.variants.is_none(),
        Stage::Seminaive => program.variants.is_some(),
    };
    if !valid {
        return Err(syn::Error::new_spanned(&program, "Datalog entry does not match the authored stage"));
    }
    if matches!(stage, Stage::Source) {
        program = cq_datalog_schedule::stratify::compile(&program)?;
    }
    if !matches!(stage, Stage::Seminaive) {
        program = cq_datalog_schedule::seminaive::compile(&program)?;
    }
    let fork = cq_datalog_fork::fork(&program, |hole| pipeline.clone().start(hole.into_token_stream()))?;
    Ok(render::render(fork))
}

fn entry(input: TokenStream, stage: Stage) -> TokenStream {
    expand(input.into(), stage).unwrap_or_else(syn::Error::into_compile_error).into()
}

/// Schedule source Datalog and expand each CQ hole through the caller pipeline.
#[proc_macro]
pub fn datalog(input: TokenStream) -> TokenStream { entry(input, Stage::Source) }

/// Resume an authored stratification, select semi-naive layouts, then execute.
#[proc_macro]
pub fn stratified(input: TokenStream) -> TokenStream { entry(input, Stage::Stratified) }

/// Resume an authored complete semi-naive schedule and its selected layouts.
#[proc_macro]
pub fn seminaive(input: TokenStream) -> TokenStream { entry(input, Stage::Seminaive) }

#[cfg(test)]
mod tests;
