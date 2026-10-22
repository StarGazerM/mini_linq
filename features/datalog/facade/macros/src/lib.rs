//! Thin Datalog wrappers: each pass compile owns its correspondence check.
use cq_datalog_configuration::DatalogConfiguration;
use proc_macro::TokenStream;
use syn::Token;
use syn::parse::{Parse, ParseStream};

mod kw {
    syn::custom_keyword!(pipeline);
    syn::custom_keyword!(arguments);
    syn::custom_keyword!(hoist_invariants);
}

struct Entry {
    configuration: DatalogConfiguration,
    hoist_invariants: bool,
    source: proc_macro2::TokenStream,
}
impl Parse for Entry {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut hoist_invariants = cfg!(feature = "hoist-invariants");
        if input.peek(kw::arguments) {
            input.parse::<kw::arguments>()?;
            let arguments;
            syn::braced!(arguments in input);
            hoist_invariants |= invariant_arguments(&arguments)?;
            input.parse::<Token![;]>()?;
        }
        let configuration = if input.peek(kw::pipeline) {
            let configuration = input.parse()?;
            if input.peek(Token![;]) {
                input.parse::<Token![;]>()?;
            }
            configuration
        } else {
            DatalogConfiguration::default()
        };
        Ok(Self {
            configuration,
            hoist_invariants,
            source: input.parse()?,
        })
    }
}

fn expand(input: proc_macro2::TokenStream) -> syn::Result<proc_macro2::TokenStream> {
    let mut entry: Entry = syn::parse2(input)?;
    if entry.hoist_invariants {
        let incremental = entry.configuration.incremental.get_or_insert_with(|| {
            syn::parse_quote!(::cq_datalog::passes::incremental)
        });
        incremental.arguments.extend(quote::quote!(hoist_invariants;));
    }
    entry
        .configuration
        .resolve(DatalogConfiguration::defaults())?
        .start(entry.source)
}

/// Expand surface syntax and run the named checked Datalog pipeline.
#[proc_macro]
pub fn datalog(input: TokenStream) -> TokenStream {
    expand(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn no_arguments(arguments: proc_macro2::TokenStream) -> syn::Result<()> {
    if arguments.is_empty() {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(
            arguments,
            "this default Datalog pass accepts no arguments",
        ))
    }
}

fn invariant_arguments(input: ParseStream<'_>) -> syn::Result<bool> {
    let enabled = input.peek(kw::hoist_invariants);
    if enabled {
        input.parse::<kw::hoist_invariants>()?;
        input.parse::<Token![;]>()?;
    }
    if !input.is_empty() {
        return Err(input.error("expected hoist_invariants;"));
    }
    Ok(enabled)
}

/// Surface tokens → Source tokens, expanding refutable bindings explicitly.
#[proc_macro]
pub fn surface_if_let(input: TokenStream) -> TokenStream {
    cq_stage::pass::<proc_macro2::TokenStream, proc_macro2::TokenStream>(
        input.into(),
        |source, args| {
            no_arguments(args)?;
            cq_datalog_surface_if_let::compile(source)
        },
    )
    .into()
}

/// Source → checked ordered components.
#[proc_macro]
pub fn stratify(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_datalog_source::Program, cq_lang_datalog_strata::Program>(
        input.into(),
        |source, args| {
            no_arguments(args)?;
            cq_datalog_stratify::compile(source)
        },
    )
    .into()
}

/// Strata → explicit covering variants and delta declarations.
#[proc_macro]
pub fn incremental(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_datalog_strata::Program, cq_lang_datalog_incremental::Program>(
        input.into(),
        |source, args| {
            use syn::parse::Parser;
            let hoist = invariant_arguments.parse2(args)? || cfg!(feature = "hoist-invariants");
            if hoist && source.source.invariants.is_empty() {
                let source = cq_datalog_stratify::invariants::compile(source)?;
                cq_datalog_incremental::compile(&source)
            } else {
                cq_datalog_incremental::compile(source)
            }
        },
    )
    .into()
}

/// Incremental → explicit storage and read bindings.
#[proc_macro]
pub fn storage(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_datalog_incremental::Program, cq_lang_datalog_storage::Program>(
        input.into(),
        |source, args| {
            no_arguments(args)?;
            cq_datalog_storage::compile(source)
        },
    )
    .into()
}

/// Storage → rounds with checked CQ holes and recorded continuations.
#[proc_macro]
pub fn fork(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_datalog_storage::Program, cq_lang_datalog_round::Program>(
        input.into(),
        cq_datalog_fork::compile_with,
    )
    .into()
}

/// Checked Round → sequential Rust expression, expanding holes afterwards.
#[proc_macro]
pub fn print_rounds(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_datalog_round::Program, syn::Expr>(input.into(), |source, args| {
        no_arguments(args)?;
        cq_datalog_print_rounds::compile(source)
    })
    .into()
}

/// Structures → pull expression with an independently selected hole partition.
#[proc_macro]
pub fn print_hole(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_structures::Program, syn::Expr>(
        input.into(),
        cq_datalog_print_hole::compile_with,
    )
    .into()
}

#[cfg(test)]
mod tests;
