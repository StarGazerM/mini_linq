//! Checked sharded admission and exclusive-layout publication around CQ evaluation.
//! Bindings, round barriers, observers and outputs reuse the base printer.
//! Certified chunk inputs retain their renamed host-set facts. The checked
//! per-task realization removes local duplicates without cloning owned rows.
//! Cross-task/round duplicates still reach the recorded canonical admission owner.
//! Standard rule tasks use the registered push adapter to feed the collector
//! directly; custom pull terminals retain iteration through their authored API.
use cq_lang_datalog_round::{self as round, InputSource, RoundContext, RuleHole};
#[cfg(feature = "hole-timing")]
use cq_lang_datalog_round::{Execution, Program};
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
    let mut stratum_index = 0usize;
    cq_datalog_print::render_with_publication(
        &program.round,
        &holes,
        |rules, context, holes, _| {
            evaluate(
                rules,
                context,
                holes,
                &program.round,
                &program.certificate,
                #[cfg(feature = "hole-timing")]
                &source,
            )
        },
        |context, _| {
            let barrier = program
                .certificate
                .publication
                .iter()
                .find(|barrier| barrier.stratum == stratum_index)
                .expect("checked complete publication barriers");
            stratum_index += 1;
            publication::publication(context, barrier, &program.certificate)
        },
    )
}

fn evaluate(
    rules: &[RuleHole],
    context: &RoundContext,
    holes: &[TokenStream],
    program: &round::Program,
    certificate: &cq_datalog_partition::ShardCertificate,
    #[cfg(feature = "hole-timing")] original: &Program,
) -> syn::Result<(TokenStream, TokenStream)> {
    let threshold = certificate.threshold;
    let partition_grain = certificate.grain;
    let self_dedup = certificate.self_dedup.is_some();
    let specifications = &program.holes;
    let execution = &program.execution;
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
        let consume = if self_dedup {
            let hasher = format_ident!("__cq_task_hasher_{}", rule.hole, span = Span::mixed_site());
            let seen = &head.seen;
            // Borrow only S before task closures: capturing seen inside a split
            // evaluation would request mutable storage and impose row Sync.
            declarations.push(quote!(let #hasher = #seen.hasher();));
            collect_task(
                specifications[rule.hole].interface,
                invocation,
                quote!((#(#types,)*)),
                &hasher,
            )
        } else {
            cq_datalog_print::collect_hole(
                specifications[rule.hole].interface,
                invocation,
                quote!(::std::vec::Vec<(#(#types,)*)>),
            )
        };
        #[cfg(feature = "hole-timing")]
        let consume = timed_chunk(consume, rule, execution, self_dedup);
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
        let evaluation = cq_datalog_print::split_evaluation(rule, context, evaluation);
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
            let raw_counter = Ident::new("__cq_timing_candidates_before", Span::mixed_site());
            let emitted = Ident::new("__cq_timing_candidates_after", Span::mixed_site());
            let sequence = Ident::new("__cq_timing_sequence", Span::mixed_site());
            let started = Ident::new("__cq_timing_started", Span::mixed_site());
            let elapsed = Ident::new("__cq_timing_elapsed", Span::mixed_site());
            evaluations.push(quote!({
                static #sequence: ::std::sync::atomic::AtomicUsize =
                    ::std::sync::atomic::AtomicUsize::new(0);
                let #invocation_id = #sequence.fetch_add(1, ::std::sync::atomic::Ordering::Relaxed);
                let #chunk_counter = ::std::sync::atomic::AtomicUsize::new(0);
                let #raw_counter = ::std::sync::atomic::AtomicUsize::new(0);
                let #started = ::std::time::Instant::now();
                #candidates = #evaluation;
                let #elapsed = #started.elapsed().as_nanos();
                let #emitted = #candidates.iter().map(|chunk| chunk.len()).sum::<usize>();
                ::std::eprintln!(
                    "cq_hole_timing\tkind=hole\thole={}\tvariant={}\tinvocation={}\tpartitioned={}\telapsed_ns={}\tcandidates={}\tchunks={}\tcq={:?}\tcandidates_before={}\tcandidates_after={}",
                    #hole, #variant, #invocation_id, #partitioned, #elapsed,
                    #emitted,
                    #chunk_counter.load(::std::sync::atomic::Ordering::Relaxed),
                    #cq, #raw_counter.load(::std::sync::atomic::Ordering::Relaxed), #emitted,
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

fn collect_task(
    interface: round::HoleInterface,
    invocation: &TokenStream,
    row_type: TokenStream,
    hasher: &Ident,
) -> TokenStream {
    let candidates = Ident::new("__cq_task_candidates", Span::mixed_site());
    #[cfg(feature = "hole-timing")]
    let count = {
        let raw = Ident::new("__cq_timing_chunk_before", Span::mixed_site());
        quote!(#raw += 1;)
    };
    #[cfg(not(feature = "hole-timing"))]
    let count = TokenStream::new();
    let consume = cq_datalog_print::consume_hole(
        interface,
        invocation,
        quote!(|row| {
            #count
            #candidates.insert(row);
        }),
    );
    quote!({
        let mut #candidates =
            ::cq_datalog_parallel_runtime::TaskCandidates::<#row_type, _>::with_hasher(#hasher);
        #consume;
        #candidates.into_rows()
    })
}

/// Diagnostic records are tab-separated key/value fields. `cq` is a Rust-debug
/// escaped string (so tabs/newlines never split records); other fields are
/// integers, booleans, or `unknown`. Hole wall time includes chunk-record output,
/// while chunk elapsed time measures only CQ collection, excluding record output.
/// Invocation numbers are local to each generated hole evaluation site.
/// `candidates_before` counts raw CQ answers, `candidates_after` counts emitted
/// task candidates, and the existing `candidates` field aliases the latter.
#[cfg(feature = "hole-timing")]
fn timed_chunk(
    consume: TokenStream,
    rule: &RuleHole,
    execution: &[Execution],
    self_dedup: bool,
) -> TokenStream {
    let hole = rule.hole;
    let variant = rule.variant;
    let partitioned = rule.scan.is_some();
    let invocation_id = Ident::new("__cq_timing_invocation", Span::mixed_site());
    let chunk_counter = Ident::new("__cq_timing_chunks", Span::mixed_site());
    let chunk_id = Ident::new("__cq_timing_chunk_id", Span::mixed_site());
    let started = Ident::new("__cq_timing_chunk_started", Span::mixed_site());
    let elapsed = Ident::new("__cq_timing_chunk_elapsed", Span::mixed_site());
    let result = Ident::new("__cq_timing_chunk_result", Span::mixed_site());
    let raw = Ident::new("__cq_timing_chunk_before", Span::mixed_site());
    let raw_counter = Ident::new("__cq_timing_candidates_before", Span::mixed_site());
    let before = self_dedup.then(|| quote!(let mut #raw = 0usize;));
    let after = (!self_dedup).then(|| quote!(let #raw = #result.len();));
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
        #before
        let #result = #consume;
        #after
        #raw_counter.fetch_add(#raw, ::std::sync::atomic::Ordering::Relaxed);
        let #elapsed = #started.elapsed().as_nanos();
        ::std::eprintln!(
            "cq_hole_timing\tkind=chunk\thole={}\tvariant={}\tinvocation={}\tchunk={}\tpartitioned={}\telapsed_ns={}\tcandidates={}\tinput_rows={}\tcandidates_before={}\tcandidates_after={}",
            #hole, #variant, #invocation_id, #chunk_id, #partitioned,
            #elapsed, #result.len(), #input_rows, #raw, #result.len(),
        );
        #result
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::ToTokens;

    fn pull_program() -> round::Program {
        let source = syn::parse_quote! {
            struct Tasks;
            relation E(x:i32) = (rows);
            relation B(x:i32) = (blocked);
            relation R(x:i32);
            output R;
            R(*x % 2) :- E(x), !B(x);
            R(*x % 2) :- if enabled, E(x), !B(x);
            R((*x + 1) % 2) :- R(x);
        };
        let strata = cq_datalog_stratify::compile(&source).unwrap();
        let incremental = cq_datalog_incremental::compile(&strata).unwrap();
        let storage = cq_datalog_storage::compile(&incremental).unwrap();
        let storage = cq_datalog_partition::compile_storage(&storage).unwrap();
        cq_datalog_fork::compile(&storage).unwrap()
    }

    fn program() -> cq_datalog_partition::ShardedProgram {
        cq_datalog_partition::compile_sharded(&pull_program()).unwrap()
    }

    /// Canonical admission and certified host inputs belong to Round, not to
    /// an individual CQ. Sequence must carry those facts across both serialized
    /// pass boundaries; dropping an envelope must not silently restore dedup.
    #[test]
    fn discharged_holes_preserve_sequence_envelopes_without_local_dedup() {
        use cq_stage::Language;

        let pull = pull_program();
        let parallel = cq_datalog_partition::compile_sharded(&pull).unwrap();
        let machine = cq_lang_sequence::Machine {
            name: syn::parse_quote!(sequential),
            root: cq_lang_sequence::LevelNode {
                name: syn::parse_quote!(host),
                collaborative: false,
                dynamic: false,
                children: vec![],
            },
        };
        assert!(!pull.context.strata[0].keys.is_empty());
        assert!(
            pull.context.strata[0]
                .seeds
                .iter()
                .any(|rule| rule.scan.is_none())
        );
        for program in [&pull, &parallel.round] {
            let splices = cq_datalog_print::splice(program).unwrap();
            assert_eq!(splices.len(), program.holes.len());
            for (hole, splice) in program.holes.iter().zip(splices) {
                assert!(hole.facts.consumer_canonical);
                assert!(!hole.facts.discharged_host_sets.is_empty());
                let arguments = &hole.pipeline.passes[hole.lower].arguments;
                assert_eq!(
                    cq_physical_lowering::context(arguments.clone()).unwrap(),
                    hole.facts,
                );
                let ra = cq_planners_plan::compile(&hole.cq, cq_planners_plan::Planner::default())
                    .unwrap();
                let ix = cq_index_policy::compile(&ra).unwrap();
                let physical = cq_physical_lowering::compile_with(&ix, arguments.clone()).unwrap();
                assert_eq!(physical.context(), hole.facts);
                assert!(physical.program.return_canon.is_none());
                assert!(
                    physical
                        .program
                        .definitions
                        .iter()
                        .all(|definition| definition.canon.is_none())
                );
                let structures = cq_structure_selection::compile(&physical).unwrap();
                assert_eq!(structures.context(), hole.facts);
                let facts = &hole.facts;
                let continuation = cq_stage::pass::<
                    cq_lang_structures::Program,
                    cq_lang_sequence::Program,
                >(
                    quote!(context { #facts }; pipeline { sequential, translate_pull }; #structures),
                    |source, _| cq_sequence_lowering::compile(source),
                );
                let continuation: syn::Macro = syn::parse2(continuation).unwrap();
                let graded = cq_stage::rewrite::<cq_lang_sequence::Program>(
                    continuation.tokens,
                    |source, _| {
                        assert_eq!(source.context(), hole.facts);
                        cq_lang_sequence::check_grading(source, &machine)?;
                        Ok(source.clone())
                    },
                );
                let graded: syn::Macro = syn::parse2(graded).unwrap();
                let emitted = cq_stage::pass::<cq_lang_sequence::Program, syn::Expr>(
                    graded.tokens,
                    |source, _| {
                        assert_eq!(source.context(), hole.facts);
                        let analysis = cq_lang_sequence::check_grading(source, &machine)?;
                        let plain = cq_sequence_pull::translate(source, &analysis)?;
                        let profile = cq_sequence_pull::translate_profile(source, &analysis)?;
                        for expression in [&plain, &profile] {
                            let tokens = expression.to_token_stream().to_string();
                            assert!(!tokens.contains("__cq_seen"), "{tokens}");
                            assert!(!tokens.contains("HashSet"), "{tokens}");
                        }
                        Ok(plain)
                    },
                );
                syn::parse2::<syn::Expr>(emitted).unwrap();
                let sequence = cq_sequence_lowering::compile(&structures).unwrap();
                let rejected = cq_stage::pass::<cq_lang_sequence::Program, syn::Expr>(
                    quote!(pipeline {}; #sequence),
                    |_, _| panic!("discharged Sequence entered without its envelope"),
                );
                assert!(rejected.to_string().contains("CTX-Entry"));

                let tail = &hole.pipeline.passes[hole.lower + 1..];
                let names: Vec<_> = tail
                    .iter()
                    .map(|pass| pass.path.segments.last().unwrap().ident.to_string())
                    .collect();
                if hole.interface == round::HoleInterface::Push {
                    // The registered push adapter is a Structures terminal,
                    // never an invented Sequence consumer entry.
                    assert_eq!(names.last().unwrap(), "print_push");
                    assert!(!names.iter().any(|name| name == "sequence"));
                    assert!(
                        splice
                            .to_string()
                            .contains("consumer { __cq_datalog_consumer }")
                    );
                } else {
                    assert!(names.ends_with(&[
                        "sequence".into(),
                        "sequential".into(),
                        "translate_pull".into(),
                    ]));
                }
            }
        }
        assert!(
            parallel
                .round
                .holes
                .iter()
                .any(|hole| hole.interface == round::HoleInterface::Push)
        );
        assert!(
            pull.holes
                .iter()
                .all(|hole| hole.interface == round::HoleInterface::Pull)
        );
    }

    #[test]
    fn checked_task_policy_covers_scan_and_single_task_rules_but_not_key_holes() {
        for interface in [round::HoleInterface::Pull, round::HoleInterface::Push] {
            let mut program = program();
            if interface == round::HoleInterface::Pull {
                program
                    .round
                    .terminals
                    .0
                    .push(syn::parse_quote!(custom::pull => pull));
                for rule in program
                    .round
                    .context
                    .strata
                    .iter()
                    .flat_map(|s| s.seeds.iter().chain(&s.steps))
                {
                    let hole = &mut program.round.holes[rule.hole];
                    hole.interface = interface;
                    hole.pipeline.passes.last_mut().unwrap().path = syn::parse_quote!(custom::pull);
                }
            }
            let component = &program.round.context.strata[0];
            assert_eq!(component.keys.len(), 1);
            assert!(component.seeds.iter().any(|rule| rule.scan.is_none()));
            let count = component.seeds.len() + component.steps.len();
            for rule in component.seeds.iter().chain(&component.steps) {
                assert_eq!(program.round.holes[rule.hole].interface, interface);
            }
            for key in &component.keys {
                assert_eq!(
                    program.round.holes[key.hole].interface,
                    round::HoleInterface::Pull
                );
            }
            let printed = compile(&program).unwrap().to_token_stream().to_string();
            assert_eq!(printed.matches("TaskCandidates").count(), count);
            assert_eq!(printed.matches(". hasher ()").count(), count);
            assert_eq!(printed.matches(". into_rows ()").count(), count);
            assert!(!printed.contains("__cq_seen"));
            let holes = cq_datalog_print::splice(&program.round).unwrap();
            let (evaluation, _) = evaluate(
                &component.seeds,
                &program.round.context,
                &holes,
                &program.round,
                &program.certificate,
                #[cfg(feature = "hole-timing")]
                &round::erase(&program.round).unwrap(),
            )
            .unwrap();
            let evaluation = evaluation.to_string();
            assert_eq!(
                evaluation.contains("for row in"),
                interface == round::HoleInterface::Pull,
            );
            if interface == round::HoleInterface::Push {
                assert_eq!(printed.matches("print_push").count(), count);
                assert_eq!(
                    printed
                        .matches("consumer { __cq_datalog_consumer }")
                        .count(),
                    count
                );
            }
            for relation in &program.round.context.relations {
                let seen = &relation.seen;
                assert!(!evaluation.contains(&quote!(&mut #seen).to_string()));
            }
            #[cfg(feature = "hole-timing")]
            {
                assert!(printed.contains("candidates_before="));
                assert!(printed.contains("candidates_after="));
                assert!(printed.contains("__cq_timing_chunk_before += 1"));
            }
            #[cfg(not(feature = "hole-timing"))]
            assert!(!printed.contains("__cq_timing_chunk_before"));

            program.certificate.self_dedup = None;
            for execution in &mut program.round.execution {
                execution.self_dedup = None;
            }
            let ordinary = compile(&program).unwrap().to_token_stream().to_string();
            assert!(!ordinary.contains("TaskCandidates"));
            assert!(!ordinary.contains(". hasher ()"));
        }
    }
}
