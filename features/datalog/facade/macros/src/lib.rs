//! Thin checked Datalog pass wrappers and named pipeline entry.
use cq_datalog_configuration::DatalogConfiguration;
use proc_macro::TokenStream;
use syn::Token;
use syn::parse::{Parse, ParseStream};

mod kw {
    syn::custom_keyword!(pipeline);
}

struct Entry {
    configuration: DatalogConfiguration,
    source: proc_macro2::TokenStream,
}
impl Parse for Entry {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
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
            source: input.parse()?,
        })
    }
}

fn expand(input: proc_macro2::TokenStream) -> syn::Result<proc_macro2::TokenStream> {
    let entry: Entry = syn::parse2(input)?;
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

/// Surface tokens → Source tokens, expanding refutable bindings explicitly.
#[proc_macro]
pub fn surface_if_let(input: TokenStream) -> TokenStream {
    cq_stage::pass::<proc_macro2::TokenStream, proc_macro2::TokenStream>(
        input.into(),
        |source, args| {
            no_arguments(args)?;
            let target = cq_datalog_surface_if_let::compile(source)?;
            cq_datalog_surface_if_let::check(source, &target)?;
            Ok(target)
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
            let target = cq_datalog_stratify::compile(source)?;
            cq_datalog_stratify::check(source, &target)?;
            Ok(target)
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
            no_arguments(args)?;
            let target = cq_datalog_incremental::compile(source)?;
            cq_datalog_incremental::check(source, &target)?;
            Ok(target)
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
            let target = cq_datalog_storage::compile(source)?;
            cq_datalog_storage::check(source, &target)?;
            Ok(target)
        },
    )
    .into()
}

/// Storage → rounds with checked CQ holes and recorded continuations.
#[proc_macro]
pub fn fork(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_datalog_storage::Program, cq_lang_datalog_round::Program>(
        input.into(),
        |source, args| {
            let target = cq_datalog_fork::compile_with(source, args)?;
            cq_datalog_fork::check(source, &target)?;
            Ok(target)
        },
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

#[cfg(test)]
mod tests;
