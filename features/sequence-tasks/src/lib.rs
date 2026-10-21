//! Sequence interpretation for an ordered, dynamically scheduled Rayon task pool.
//!
//! Generated expressions require `rayon` and `cq-storage` in the consumer. The
//! caller supplies facts checked against [`machine`]; translation never repeats
//! analysis. Host and task-local constructors reuse the sequential interpreter.
//! Pool chunks own their buffers and publish in chunk order at the join. Rayon
//! propagates a task panic to that join rather than publishing partial output.

use cq_lang_sequence::{Analysis, LevelNode, Machine, Monoid, Program, Term};
use cq_sequence_pull::{Emitter, Value};
use proc_macro2::TokenStream;
use quote::quote;

/// The host owns a separated dynamic pool of sequential tasks.
pub fn machine() -> Machine {
    Machine {
        name: syn::parse_quote!(tasks),
        root: LevelNode {
            name: syn::parse_quote!(host),
            collaborative: false,
            dynamic: false,
            children: vec![LevelNode {
                name: syn::parse_quote!(pool),
                collaborative: false,
                dynamic: true,
                children: vec![LevelNode {
                    name: syn::parse_quote!(task),
                    collaborative: false,
                    dynamic: false,
                    children: vec![],
                }],
            }],
        },
    }
}

/// Translate checked Sequence into an owned, deterministically concatenated Vec.
// Formal rule T-Tasks: fold the checked term using ordered pool partitions and
// local buffers; canonicalize parts before the global canonical boundary.
pub fn translate(program: &Program, analysis: &Analysis) -> syn::Result<syn::Expr> {
    cq_sequence_pull::translate_with(
        program,
        analysis,
        |term, emitter, child, output, definition, nesting| {
            let level = analysis
                .levels
                .get(definition)
                .map_or(0, |path| path.len().saturating_sub(1))
                + nesting;
            if level != 1 {
                return Ok(());
            }
            match (term, child) {
                (Term::Tile { n, .. }, Some(input)) => tile(*n, input, output),
                (Term::Store { capacity, .. }, Some(input)) => store(*capacity, input, output),
                (Term::Fold(monoid, _), Some(input)) => fold(monoid, emitter, input, output)?,
                (Term::Canon(_), Some(input)) => canon(emitter, input, output)?,
                _ => {}
            }
            Ok(())
        },
    )
}

fn tile(n: usize, input: &Value, output: &mut Value) {
    let rows = &input.rows;
    output.rows = quote! {{
        use ::rayon::prelude::*;
        let __cq_task_input = (#rows).collect::<::std::vec::Vec<_>>();
        __cq_task_input.par_chunks(#n)
            .map(|__cq_chunk| __cq_chunk.to_vec())
            .collect::<::std::vec::Vec<_>>()
            .into_iter()
    }};
}

fn store(capacity: Option<usize>, input: &Value, output: &mut Value) {
    let rows = &input.rows;
    let capacity_check = capacity.map(|capacity| {
        quote! {
            ::core::assert!(__cq_task_input.len() <= #capacity, "Sequence store capacity exceeded");
        }
    });
    output.rows = quote! {{
        use ::rayon::prelude::*;
        let __cq_task_input = (#rows).collect::<::std::vec::Vec<_>>();
        #capacity_check
        let __cq_parts = __cq_task_input.into_par_iter()
            .chunks(1024)
            .collect::<::std::vec::Vec<_>>();
        __cq_parts.into_iter().flatten()
    }};
}

fn fold(
    monoid: &Monoid,
    emitter: &Emitter<'_>,
    input: &Value,
    output: &mut Value,
) -> syn::Result<()> {
    let rows = &input.rows;
    let reduction: TokenStream = match monoid {
        Monoid::Count => quote! {
            __cq_task_input.into_par_iter().map(|_| 1usize).reduce(|| 0usize, |a, b| a + b)
        },
        Monoid::Or => quote! {
            __cq_task_input.into_par_iter().map(|_| true).reduce(|| false, |a, b| a || b)
        },
        Monoid::Sum(column) => {
            let result = input.result.as_deref().ok_or_else(|| {
                syn::Error::new(column.span(), "sum requires a checked row heading")
            })?;
            let pattern = emitter.context().pattern(result);
            let column = syn::ext::IdentExt::unraw(column).to_string();
            let value = emitter.context().access(result, &column);
            quote! {
                __cq_task_input.into_par_iter().map(|#pattern| #value).reduce(|| 0, |a, b| a + b)
            }
        }
    };
    output.rows = quote! {{
        use ::rayon::prelude::*;
        let __cq_task_input = (#rows).collect::<::std::vec::Vec<_>>();
        ::core::iter::once(#reduction)
    }};
    Ok(())
}

fn canon(emitter: &mut Emitter<'_>, input: &Value, output: &mut Value) -> syn::Result<()> {
    let rows = &input.rows;
    let mut part = input.clone();
    part.rows = quote! { __cq_chunk.iter().cloned() };
    part.stored = false;
    let local = emitter.canon(part)?.rows;
    let mut prefiltered = input.clone();
    prefiltered.rows = quote! {{
        use ::rayon::prelude::*;
        let __cq_task_input = (#rows).collect::<::std::vec::Vec<_>>();
        let __cq_parts = __cq_task_input.par_chunks(1024)
            .map(|__cq_chunk| (#local).collect::<::std::vec::Vec<_>>())
            .collect::<::std::vec::Vec<_>>();
        __cq_parts.into_iter().flatten()
    }};
    prefiltered.stored = false;
    *output = emitter.canon(prefiltered)?;
    Ok(())
}
