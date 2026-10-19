//! Datalog stage entrypoints over the ordinary checked CQ macro protocol.
//!
//! Each entry accepts an optional named MiniLinq `pipeline { ... }` followed
//! by its authored Datalog program. An explicit pass queue is also accepted.
//! `datalog!` schedules source, `stratified!` resumes checked strata, and
//! `seminaive!` resumes a complete schedule. Every CQ hole receives the same
//! caller continuation, including unchanged opaque pass arguments. The bridge
//! owns sets, grouped host layouts and rounds; it adds no planner options.
//! Full layouts are maintained by inserting each newly admitted row at
//! publication. Delta layouts are separate grouped `HashMap` hosts: seed
//! publication and every later publication replace each demanded run once,
//! including empty runs, before any hole in the next round can borrow it.
//! All holes sharing a delta layout borrow the same immutable round snapshot.
//! Keyed delta reads and later key-zero probes use those hosts; only an initial
//! unkeyed delta atom scans its `Vec`. Written body and guard order is unchanged.
//! Every EDB is a set: before any layouts or rounds, its generic borrowed `Rows`
//! host is scanned once into a hash set of borrowed column tuples. All layouts,
//! negative-key projections, and EDB outputs reuse that canonical source; extra
//! layouts never rescan or re-deduplicate the input. Canonicalization clones no
//! column values. Owned grouped layouts and outputs clone only distinct tuples.
//! All EDB columns therefore require `Eq + Hash`; negative-only columns need
//! not implement `Clone`. This is mandatory input semantics, not a policy.
//! Multiple outputs return a tuple of owned iterators in written output order;
//! single outputs return one owned iterator, including EDB-only programs.
//! `parallel;` before `struct` opts into Rayon rule-hole evaluation. Admission
//! and publication remain sequential. Consumers supply their own `rayon`
//! dependency; captured values must be `Sync` and candidate rows `Send`.

use proc_macro::TokenStream;
use quote::ToTokens;
use syn::parse::{Parse, ParseStream};
use syn::{Token, braced};

mod render;

mod kw {
    syn::custom_keyword!(arguments);
    syn::custom_keyword!(pipeline);
    syn::custom_keyword!(parallel);
    syn::custom_keyword!(observe);
}

struct Input {
    pipeline: cq_stage::Pipeline,
    parallel: bool,
    program: cq_lang_datalog::Program,
}

fn defaults() -> cq_stage::Configuration {
    syn::parse_quote! { pipeline {
        plan = ::mini_linq::passes::plan;
        ix = ::mini_linq::passes::index_demands;
        lower = ::mini_linq::passes::lower;
        select = ::mini_linq::passes::select;
        represent = ::mini_linq::passes::represent;
        stage = ::mini_linq::passes::stage;
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
                input
                    .parse::<cq_stage::Configuration>()?
                    .resolve(defaults())?
            } else {
                input.parse::<cq_stage::Pipeline>()?
            }
        } else {
            defaults().resolve(cq_stage::Configuration::default())?
        };
        if input.peek(Token![;]) {
            input.parse::<Token![;]>()?;
        }
        let mut parallel = false;
        let mut observer = None;
        loop {
            if input.peek(kw::parallel) {
                if parallel {
                    return Err(input.error("duplicate parallel option"));
                }
                input.parse::<kw::parallel>()?;
                input.parse::<Token![;]>()?;
                parallel = true;
            } else if input.peek(kw::observe) {
                if observer.is_some() {
                    return Err(input.error("duplicate observe option"));
                }
                input.parse::<kw::observe>()?;
                input.parse::<Token![=]>()?;
                observer = Some(input.parse::<syn::Expr>()?);
                input.parse::<Token![;]>()?;
            } else {
                break;
            }
        }
        let mut program: cq_lang_datalog::Program = input.parse()?;
        program.observer = observer;
        Ok(Self {
            pipeline,
            parallel,
            program,
        })
    }
}

#[derive(Clone, Copy)]
enum Stage {
    Source,
    Stratified,
    Seminaive,
}

fn expand(tokens: proc_macro2::TokenStream, stage: Stage) -> syn::Result<proc_macro2::TokenStream> {
    let Input {
        pipeline,
        parallel,
        mut program,
    } = syn::parse2(tokens)?;
    cq_lang_datalog::contract::check(&program)?;
    let valid = match stage {
        Stage::Source => program.strata.is_none() && program.variants.is_none(),
        Stage::Stratified => program.strata.is_some() && program.variants.is_none(),
        Stage::Seminaive => program.variants.is_some(),
    };
    if !valid {
        return Err(syn::Error::new_spanned(
            &program,
            "Datalog entry does not match the authored stage",
        ));
    }
    if matches!(stage, Stage::Source) {
        program = cq_datalog_schedule::stratify::compile(&program)?;
    }
    if !matches!(stage, Stage::Seminaive) {
        program = cq_datalog_schedule::seminaive::compile(&program)?;
    }
    let fork = cq_datalog_fork::fork(&program, |hole| {
        pipeline.clone().start(hole.into_token_stream())
    })?;
    Ok(render::render(fork, parallel))
}

fn entry(input: TokenStream, stage: Stage) -> TokenStream {
    expand(input.into(), stage)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Schedule source Datalog and expand each CQ hole through the caller pipeline.
#[proc_macro]
pub fn datalog(input: TokenStream) -> TokenStream {
    entry(input, Stage::Source)
}

/// Resume an authored stratification, select semi-naive layouts, then execute.
#[proc_macro]
pub fn stratified(input: TokenStream) -> TokenStream {
    entry(input, Stage::Stratified)
}

/// Resume an authored complete semi-naive schedule and its selected layouts.
#[proc_macro]
pub fn seminaive(input: TokenStream) -> TokenStream {
    entry(input, Stage::Seminaive)
}

#[cfg(test)]
mod tests;
