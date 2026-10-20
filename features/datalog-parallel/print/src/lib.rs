//! Checked sharded admission and exclusive-layout publication around CQ evaluation.
//! Bindings, round barriers, observers and outputs reuse the base printer.
#[cfg(feature = "hole-timing")]
use cq_lang_datalog_round::Program;
use cq_lang_datalog_round::{self as round, Execution, InputSource, RoundContext, RuleHole};
use proc_macro2::{Span, TokenStream};
#[cfg(feature = "hole-timing")]
use quote::ToTokens;
use quote::{format_ident, quote};
use syn::Ident;
mod publication;

pub fn compile(program: &cq_datalog_partition::ShardedProgram) -> syn::Result<syn::Expr> {
    let source = round::erase(&program.round)?;
    cq_datalog_partition::check_sharded(&source, program)?;
    let holes = cq_datalog_print::splice(&program.round)?;
    let threshold = program.certificate.threshold;
    cq_datalog_print::render_with_publication(
        &program.round,
        &holes,
        |rules, context, holes, execution| {
            evaluate(
                rules,
                context,
                holes,
                execution,
                threshold,
                program.certificate.grain,
                #[cfg(feature = "hole-timing")]
                &source,
            )
        },
        |context, stratum| publication::publication(context, stratum, threshold),
    )
}

fn evaluate(
    rules: &[RuleHole],
    context: &RoundContext,
    holes: &[TokenStream],
    execution: &[Execution],
    threshold: usize,
    partition_grain: usize,
    #[cfg(feature = "hole-timing")] original: &Program,
) -> syn::Result<(TokenStream, TokenStream)> {
    let mut declarations = Vec::new();
    let mut evaluations = Vec::new();
    let mut candidates_by_head = ::std::collections::BTreeMap::<usize, Vec<Ident>>::new();
    let scope = Ident::new("__cq_scope", Span::mixed_site());
    let source_name = Ident::new("__cq_source", Span::mixed_site());
    let evaluator = Ident::new("__cq_evaluate", Span::mixed_site());
    let grain = Ident::new("__cq_grain", Span::mixed_site());
    let bucket = Ident::new("__cq_bucket", Span::mixed_site());
    let inline = Ident::new("__cq_inline", Span::mixed_site());
    for rule in rules {
        let candidates = format_ident!("__cq_candidates_{}", rule.hole, span = Span::mixed_site());
        let invocation = &holes[rule.hole];
        let head = &context.relations[rule.head];
        let types = head.declaration.columns.iter().map(|column| &column.ty);
        let consume = quote!((#invocation).collect::<::std::vec::Vec<(#(#types,)*)>>());
        #[cfg(feature = "hole-timing")]
        let consume = timed_chunk(consume, rule, execution);
        let evaluation = if let Some(scan) = &rule.scan {
            let annotation = execution
                .iter()
                .find(|annotation| annotation.hole == rule.hole)
                .expect("checked execution coverage");
            let chunk = &annotation.binding;
            let (source, relation, columns) = match scan.source {
                InputSource::Delta(index) => {
                    let relation = &context.relations[index];
                    let host = relation.delta_host.as_ref().expect("checked delta host");
                    (
                        quote!(#host.as_slice()),
                        relation,
                        (0..relation.declaration.arity()).collect::<Vec<_>>(),
                    )
                }
                InputSource::Layout(index) => {
                    let layout = &context.layouts[index];
                    let host = &layout.host;
                    (
                        quote!(#host.get(&()).map_or(&[][..], |#bucket| #bucket.as_slice())),
                        &context.relations[layout.relation],
                        layout.columns.clone(),
                    )
                }
            };
            let input_types = columns
                .iter()
                .map(|&column| &relation.declaration.columns[column].ty);
            quote!({
                let #source_name = #source;
                let #evaluator = |#chunk: &[(#(#input_types,)*)]| { #consume };
                let #grain = ::core::cmp::max(
                    #partition_grain,
                    #source_name.len() / (4 * ::rayon::current_num_threads()),
                );
                if #inline || #source_name.len() <= #grain {
                    ::std::vec![#evaluator(#source_name)]
                } else {
                    use ::rayon::prelude::*;
                    #source_name.par_chunks(#grain).map(#evaluator).collect::<::std::vec::Vec<_>>()
                }
            })
        } else {
            quote!(::std::vec![#consume])
        };
        declarations.push(quote!(let mut #candidates = ::std::vec::Vec::new();));
        #[cfg(not(feature = "hole-timing"))]
        evaluations.push(quote!(#candidates = #evaluation;));
        #[cfg(feature = "hole-timing")]
        {
            let hole = rule.hole;
            let variant = rule.variant;
            let partitioned = rule.scan.is_some();
            // Erasure restores the original CQ bindings before partition renaming.
            let cq = original.holes[hole].cq.query.to_token_stream().to_string();
            let invocation_id = Ident::new("__cq_timing_invocation", Span::mixed_site());
            let chunk_counter = Ident::new("__cq_timing_chunks", Span::mixed_site());
            let sequence = Ident::new("__cq_timing_sequence", Span::mixed_site());
            let started = Ident::new("__cq_timing_started", Span::mixed_site());
            let elapsed = Ident::new("__cq_timing_elapsed", Span::mixed_site());
            evaluations.push(quote!({
                static #sequence: ::std::sync::atomic::AtomicUsize =
                    ::std::sync::atomic::AtomicUsize::new(0);
                let #invocation_id = #sequence.fetch_add(1, ::std::sync::atomic::Ordering::Relaxed);
                let #chunk_counter = ::std::sync::atomic::AtomicUsize::new(0);
                let #started = ::std::time::Instant::now();
                #candidates = #evaluation;
                let #elapsed = #started.elapsed().as_nanos();
                ::std::eprintln!(
                    "cq_hole_timing\tkind=hole\thole={}\tvariant={}\tinvocation={}\tpartitioned={}\telapsed_ns={}\tcandidates={}\tchunks={}\tcq={:?}",
                    #hole, #variant, #invocation_id, #partitioned, #elapsed,
                    #candidates.iter().map(|chunk| chunk.len()).sum::<usize>(),
                    #chunk_counter.load(::std::sync::atomic::Ordering::Relaxed),
                    #cq,
                );
            }));
        }
        candidates_by_head
            .entry(rule.head)
            .or_default()
            .push(candidates);
    }
    if rules.is_empty() {
        return Ok((TokenStream::new(), TokenStream::new()));
    }
    // Count each driving occurrence: a small delta must not serialize holes
    // whose selected full-input scans still do substantial work.
    let delta_work = if let Some(stratum) = context.strata.iter().find(|s| s.steps == rules) {
        let lengths = stratum
            .members
            .iter()
            .filter_map(|&index| context.relations[index].delta_host.as_ref())
            .map(|delta| quote!(#delta.len()));
        quote!(0usize #(+ #lengths)*)
    } else {
        quote!(0usize)
    };
    let lengths = rules
        .iter()
        .filter_map(|rule| rule.scan.as_ref())
        .map(|scan| match scan.source {
            InputSource::Delta(index) => {
                let host = context.relations[index]
                    .delta_host
                    .as_ref()
                    .expect("checked delta host");
                quote!(#host.len())
            }
            InputSource::Layout(index) => {
                let host = &context.layouts[index].host;
                quote!(#host.get(&()).map_or(0, |#bucket| #bucket.len()))
            }
        });
    let work = quote!(::core::cmp::max(#delta_work, 0usize #(+ #lengths)*));
    let evaluators: Vec<_> = rules
        .iter()
        .map(|rule| format_ident!("__cq_run_hole_{}", rule.hole, span = Span::mixed_site()))
        .collect();
    let closures = evaluators
        .iter()
        .zip(&evaluations)
        .map(|(run, body)| quote!(let mut #run = || { #body };));
    let spawned = evaluators
        .iter()
        .map(|run| quote!(#scope.spawn(move |_| { #run(); });));
    let evaluation = quote!(
        #(#declarations)*
        let #inline = #work < #threshold || ::rayon::current_num_threads() == 1;
        #(#closures)*
        if #inline {
            #(#evaluators();)*
        } else {
            ::rayon::scope(|#scope| { #(#spawned)* });
        }
    );
    let admissions = candidates_by_head.into_iter().map(|(index, candidates)| {
        let relation = &context.relations[index];
        let seen = &relation.seen;
        let next = &relation.next;
        let first = &candidates[0];
        let rest = &candidates[1..];
        quote!(
            #(#first.extend(#rest);)*
            #next = #seen.admit(#first, #threshold);
        )
    });
    Ok((evaluation, quote!(#(#admissions)*)))
}

/// Diagnostic records are tab-separated key/value fields. `cq` is a Rust-debug
/// escaped string (so tabs/newlines never split records); other fields are
/// integers, booleans, or `unknown`. Hole wall time includes chunk-record output,
/// while chunk elapsed time measures only CQ collection, excluding record output.
/// Invocation numbers are local to each generated hole evaluation site.
#[cfg(feature = "hole-timing")]
fn timed_chunk(consume: TokenStream, rule: &RuleHole, execution: &[Execution]) -> TokenStream {
    let hole = rule.hole;
    let variant = rule.variant;
    let partitioned = rule.scan.is_some();
    let invocation_id = Ident::new("__cq_timing_invocation", Span::mixed_site());
    let chunk_counter = Ident::new("__cq_timing_chunks", Span::mixed_site());
    let chunk_id = Ident::new("__cq_timing_chunk_id", Span::mixed_site());
    let started = Ident::new("__cq_timing_chunk_started", Span::mixed_site());
    let elapsed = Ident::new("__cq_timing_chunk_elapsed", Span::mixed_site());
    let result = Ident::new("__cq_timing_chunk_result", Span::mixed_site());
    let input_rows = if partitioned {
        let chunk = &execution
            .iter()
            .find(|entry| entry.hole == hole)
            .expect("checked execution coverage")
            .binding;
        quote!(#chunk.len())
    } else {
        quote!("unknown")
    };
    quote!({
        let #chunk_id = #chunk_counter.fetch_add(1, ::std::sync::atomic::Ordering::Relaxed);
        let #started = ::std::time::Instant::now();
        let #result = #consume;
        let #elapsed = #started.elapsed().as_nanos();
        ::std::eprintln!(
            "cq_hole_timing\tkind=chunk\thole={}\tvariant={}\tinvocation={}\tchunk={}\tpartitioned={}\telapsed_ns={}\tcandidates={}\tinput_rows={}",
            #hole, #variant, #invocation_id, #chunk_id, #partitioned,
            #elapsed, #result.len(), #input_rows,
        );
        #result
    })
}
