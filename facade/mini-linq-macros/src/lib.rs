//! Proc-macro ABI adapters for the public checked pipeline.
use proc_macro::TokenStream;
use syn::parse::{Parse, ParseStream};

mod load;
mod workload;

struct Source {
    config: cq_stage::Configuration,
    tokens: proc_macro2::TokenStream,
}
impl Parse for Source {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let config = input.parse()?;
        if input.peek(syn::Token![;]) {
            input.parse::<syn::Token![;]>()?;
        }
        Ok(Self {
            config,
            tokens: input.parse()?,
        })
    }
}
fn defaults() -> cq_stage::Configuration {
    syn::parse_quote! { pipeline {
        plan = ::mini_linq::passes::plan;
        ix = ::mini_linq::passes::index_demands;
        lower = ::mini_linq::passes::lower;
        select = ::mini_linq::passes::select;
        represent = ::mini_linq::passes::represent;
        stage = ::mini_linq::passes::stage;
        sequence = ::mini_linq::passes::sequence;
        decompose = [::mini_linq::passes::plan_sequential];
        grade = ::mini_linq::passes::sequential;
        print = ::mini_linq::passes::translate_pull;
    } }
}

/// CQ entry is not a checked continuation. In particular, a printed lower IR
/// cannot import restricted facts through a custom surface/planning adapter.
/// Configured lower/select packets introduce facts later, on their own pass.
fn check_entry(tokens: &proc_macro2::TokenStream) -> syn::Result<()> {
    use cq_stage::Language;
    use syn::parse::Parser;

    let mut prefix = tokens.clone().into_iter();
    let Some(proc_macro2::TokenTree::Ident(name)) = prefix.next() else {
        return Ok(());
    };
    if (name == "context" || name == "arguments")
        && matches!(prefix.next(), Some(proc_macro2::TokenTree::Group(group))
            if group.delimiter() == proc_macro2::Delimiter::Brace)
        && matches!(prefix.next(), Some(proc_macro2::TokenTree::Punct(punct))
            if punct.as_char() == ';')
    {
        return Err(syn::Error::new_spanned(
            tokens,
            "CTX-Entry: CQ entry does not accept a checked continuation envelope",
        ));
    }
    // Structures and Sequence both begin with their complete Physical program.
    // Do not parse normal CQ or opaque surface input as three other languages.
    if name != "physical" {
        return Ok(());
    }
    let recorded = |input: ParseStream<'_>| -> syn::Result<bool> {
        let program: cq_lang_physical::Program = input.parse()?;
        let _: proc_macro2::TokenStream = input.parse()?;
        Ok(!program.context().is_empty())
    };
    if recorded.parse2(tokens.clone()).unwrap_or(false) {
        Err(syn::Error::new_spanned(
            tokens,
            "CTX-Entry: recorded context facts require an explicit exact context envelope; CQ entry is not a checked continuation",
        ))
    } else {
        Ok(())
    }
}

fn expand_compile(input: proc_macro2::TokenStream) -> syn::Result<proc_macro2::TokenStream> {
    let source: Source = syn::parse2(input)?;
    check_entry(&source.tokens)?;
    if syn::parse2::<cq_lang_sequence::Program>(source.tokens.clone()).is_ok() {
        return sequence_pipeline(source.config)?.start(source.tokens);
    }
    let mut defaults = defaults();
    if source.config.plan.is_some()
        || !source.config.physical.is_empty()
        || !source.config.structures.is_empty()
    {
        defaults.decompose =
            vec![syn::parse_quote!(::mini_linq::passes::plan_sequential(preserve_order))];
    }
    source.config.resolve(defaults)?.start(source.tokens)
}

/// Complete Sequence input starts at planning, not at the CQ ordering phase.
fn sequence_pipeline(config: cq_stage::Configuration) -> syn::Result<cq_stage::Pipeline> {
    if config.ix.is_some()
        || config.lower.is_some()
        || !config.physical.is_empty()
        || config.select.is_some()
        || !config.structures.is_empty()
        || config.represent.is_some()
        || config.stage.is_some()
        || config.sequence.is_some()
        || !config.surface.is_empty()
    {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "complete Sequence input accepts only plan, decompose, grade and print phases",
        ));
    }
    let mut passes = config.decompose;
    passes.push(
        config
            .plan
            .unwrap_or_else(|| syn::parse_quote!(::mini_linq::passes::plan)),
    );
    passes.push(
        config
            .grade
            .unwrap_or_else(|| syn::parse_quote!(::mini_linq::passes::sequential)),
    );
    passes.push(
        config
            .print
            .unwrap_or_else(|| syn::parse_quote!(::mini_linq::passes::translate_pull)),
    );
    Ok(cq_stage::Pipeline { passes })
}

fn expand_mini_linq(input: proc_macro2::TokenStream) -> syn::Result<proc_macro2::TokenStream> {
    check_entry(&input)?;
    defaults()
        .resolve(cq_stage::Configuration::default())?
        .start(input)
}

/// Compile CQ with a named pipeline; omitted phases use the core defaults.
#[proc_macro]
pub fn compile(input: TokenStream) -> TokenStream {
    expand_compile(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Compile CQ with the deterministic default pipeline.
/// Formal rule ST-Entry: Start CQ surface expansion and the selected checked pipeline.
#[proc_macro]
pub fn mini_linq(input: TokenStream) -> TokenStream {
    expand_mini_linq(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn no_arguments(arguments: proc_macro2::TokenStream) -> syn::Result<()> {
    if arguments.is_empty() {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(
            arguments,
            "this default pass accepts no arguments",
        ))
    }
}

#[derive(Default)]
struct PlanOptions {
    /// An explicit relation order, checked by the ordinary ordering pass.
    order: Option<Vec<syn::Ident>>,
    no_reduce: bool,
    /// `(target, support)` relation names to add, in advice order.
    reduce: Vec<(syn::Ident, syn::Ident)>,
    /// `(target, support)` relation names whose Masks reduction is dropped.
    skip: Vec<(syn::Ident, syn::Ident)>,
}
impl Parse for PlanOptions {
    /// Optional `order(Relation, ...)` plus `no_reduce`, or any number of
    /// `reduce(Target with Support)` and `skip(Target with Support)`, comma separated.
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut options = Self::default();
        while !input.is_empty() {
            let option: syn::Ident = input.parse()?;
            let advised = !options.reduce.is_empty() || !options.skip.is_empty();
            if option == "order" && options.order.is_none() {
                let content;
                syn::parenthesized!(content in input);
                options.order = Some(
                    content
                        .parse_terminated(syn::Ident::parse, syn::Token![,])?
                        .into_iter()
                        .collect(),
                );
            } else if option == "no_reduce" && !options.no_reduce && !advised {
                options.no_reduce = true;
            } else if (option == "reduce" || option == "skip") && !options.no_reduce {
                let content;
                syn::parenthesized!(content in input);
                let target: syn::Ident = content.parse()?;
                let with: syn::Ident = content.parse()?;
                if with != "with" {
                    return Err(syn::Error::new_spanned(with, "expected `with`"));
                }
                let support: syn::Ident = content.parse()?;
                if !content.is_empty() {
                    return Err(content.error("expected (Target with Support)"));
                }
                if option == "reduce" {
                    options.reduce.push((target, support));
                } else {
                    options.skip.push((target, support));
                }
            } else {
                return Err(syn::Error::new_spanned(
                    option,
                    "expected order(Relation, ...), and no_reduce or reduce/skip(Target with Support)",
                ));
            }
            if !input.is_empty() {
                input.parse::<syn::Token![,]>()?;
                if input.is_empty() {
                    return Err(input.error("trailing comma after plan options"));
                }
            }
        }
        Ok(options)
    }
}

/// The written atom clause of the stream scope reading `relation`; advice must
/// name a relation read exactly once there.
fn advised_clause(
    body: &cq_lang_cq::CommaList<cq_lang_cq::BodyItem>,
    relation: &syn::Ident,
) -> syn::Result<usize> {
    let mut clauses = body
        .iter()
        .enumerate()
        .filter(|(_, item)| item.atom().is_some_and(|atom| atom.relation == *relation))
        .map(|(clause, _)| clause);
    match (clauses.next(), clauses.next()) {
        (Some(clause), None) => Ok(clause),
        _ => Err(syn::Error::new_spanned(
            relation,
            "reduce advice names a relation read by exactly one top-level atom",
        )),
    }
}

/// In a Sequence decomposition list, `plan(target::plan)` delegates to the
/// target-owned planner macro after all preceding authored advice. The target
/// supplies its machine/catalog to the shared planner without a target dependency
/// in this facade. CQ planning options retain their existing meaning.
/// CQ → RA′: a left-deep plan in written order with checked reductions.
/// The first clause of each scope is the outer stream; every other clause is
/// reduced by its filtered subtrees of the outer-rooted reduction tree before
/// the stream starts. `plan(no_reduce)` disables reductions, not the order.
/// `plan(reduce(T with S))` additionally reduces the top-level atom reading `T`
/// by the one reading `S` after the Masks reductions, before the stream;
/// `plan(skip(T with S))` drops that Masks reduction. Both record the author's
/// cardinality knowledge as VO reduce steps; dropping a reduction is sound.
/// Formal rule Plan-Prefix: Normalize, order Written/Coarse, then optionally reduce by outer-rooted masks and advised reductions before IX.
#[proc_macro]
pub fn plan(input: TokenStream) -> TokenStream {
    let input: proc_macro2::TokenStream = input.into();
    if sequence_plan_target(&input) {
        return cq_stage::delegate(input).into();
    }
    if sequence_plan_source(&input) {
        return plan_sequential(input.into());
    }
    cq_stage::pass::<cq_lang_cq::Program, cq_lang_ra::Program>(input, |source, args| {
        let args: PlanOptions = syn::parse2(args)?;
        let normal = cq_lang_cq::normal::normalize(source)?;
        let ordering = cq_order::Options {
            heuristic: cq_order::Heuristic::Written,
            split: cq_order::Split::Coarse,
        };
        let plan = if let Some(relations) = &args.order {
            cq_order::compile_explicit(&normal, relations)?
        } else {
            let plan = cq_order::compile(&normal, &ordering)?;
            cq_order::check(&normal, &plan, &ordering)?;
            plan
        };
        let options = cq_reduce::Options {
            semijoins: if args.no_reduce {
                cq_reduce::Reductions::None
            } else {
                cq_reduce::Reductions::Masks
            },
        };
        let mut reduced = cq_reduce::compile(&plan, &options)?;
        cq_reduce::check(&plan, &reduced, &options)?;
        if !args.reduce.is_empty() || !args.skip.is_empty() {
            // Formal rule R-Advice: Advised semijoins of two top-level atoms sharing a variable run after the Masks reductions, before the stream; skipped ones are removed.
            let analysis = cq_lang_vo::contract::check(&reduced)?;
            let graph = cq_lang_cq::hypergraph::Analysis::of(&reduced.cq);
            let body = &reduced.cq.query.body;
            let pair = |target: &syn::Ident, support: &syn::Ident| -> syn::Result<(usize, usize)> {
                let (t, s) = (
                    advised_clause(body, target)?,
                    advised_clause(body, support)?,
                );
                if t == s || graph.shared(t, s).is_empty() {
                    return Err(syn::Error::new_spanned(
                        support,
                        "reduce advice needs two atoms sharing a variable",
                    ));
                }
                Ok((t, s))
            };
            let mut skipped = Vec::new();
            for (target, support) in &args.skip {
                let (t, s) = pair(target, support)?;
                let step = cq_reduce::semijoin(&analysis, &graph, t, s);
                if !reduced.valuation.steps.contains(&step) {
                    return Err(syn::Error::new_spanned(
                        target,
                        "skip advice names no Masks reduction",
                    ));
                }
                skipped.push(step);
            }
            reduced
                .valuation
                .steps
                .retain(|step| !skipped.contains(step));
            let mut steps = Vec::new();
            for (target, support) in &args.reduce {
                let (t, s) = pair(target, support)?;
                steps.push(cq_reduce::semijoin(&analysis, &graph, t, s));
            }
            let first = reduced
                .valuation
                .steps
                .iter()
                .position(|step| {
                    matches!(
                        step,
                        cq_lang_vo::Step::Atom(..) | cq_lang_vo::Step::Block(..)
                    )
                })
                .unwrap_or(reduced.valuation.steps.len());
            reduced.valuation.steps.splice(first..first, steps);
            cq_lang_vo::contract::check(&reduced)?;
        }
        let target = cq_elaborate::compile(&reduced)?;
        cq_elaborate::check(&reduced, &target)?;
        Ok(target)
    })
    .into()
}

/// RA′ → IX exact demand analysis.
#[proc_macro]
pub fn index_demands(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_ra::Program, cq_lang_ix::Program>(input.into(), |source, args| {
        no_arguments(args)?;
        cq_index_policy::compile(source)
    })
    .into()
}

/// IX → Physical with probe/keep/first/eager defaults.
/// Comma-separated `consumer = canonical` and `host_set R discharged`
/// arguments explicitly supply restricted context facts.
#[proc_macro]
pub fn lower(input: TokenStream) -> TokenStream {
    cq_stage::pass_with_context::<cq_lang_ix::Program, cq_lang_physical::Program>(
        input.into(),
        cq_physical_lowering::context,
        cq_physical_lowering::compile_with,
    )
    .into()
}

/// Physical → Structures with concrete default Rust types.
/// Optional `positional(H, ...)` arguments assert the default typed-column
/// capability for those hosts; unspecified hosts retain their original path.
/// Lower/select context facts may be interleaved with positional arguments.
#[proc_macro]
pub fn select(input: TokenStream) -> TokenStream {
    cq_stage::pass_with_context::<cq_lang_physical::Program, cq_lang_structures::Program>(
        input.into(),
        cq_structure_selection::context,
        cq_structure_selection::compile_with,
    )
    .into()
}

/// Structures → Structures representation decisions; `positions` opts in.
#[proc_macro]
pub fn represent(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_structures::Program>(input.into(), cq_represent::compile_with)
        .into()
}

/// Structures → Structures total lookahead; `batch(N)` opts into scan staging.
#[proc_macro]
pub fn stage(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_structures::Program>(input.into(), cq_staging::compile_with).into()
}

/// Structures → Sequence with checked default lowering.
// Formal rule ST-Sequence: lower Structures, check host grading, then translate Sequence.
#[proc_macro]
pub fn sequence(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_structures::Program, cq_lang_sequence::Program>(
        input.into(),
        |source, args| {
            no_arguments(args)?;
            let target = cq_sequence_lowering::compile(source)?;
            cq_sequence_lowering::check(source, &target)?;
            Ok(target)
        },
    )
    .into()
}

fn sequence_plan_target(input: &proc_macro2::TokenStream) -> bool {
    use syn::parse::Parser;
    let parser = |input: ParseStream<'_>| -> syn::Result<bool> {
        let name: syn::Ident = input.parse()?;
        if name != "arguments" {
            return Ok(false);
        }
        let arguments;
        syn::braced!(arguments in input);
        let args: proc_macro2::TokenStream = arguments.parse()?;
        let _: proc_macro2::TokenStream = input.parse()?;
        Ok(!matches!(args.to_string().as_str(), "lookup_first" | "preserve_order")
            && syn::parse2::<PlanOptions>(args.clone()).is_err()
            && syn::parse2::<syn::Path>(args).is_ok())
    };
    parser.parse2(input.clone()).unwrap_or(false)
}

fn sequence_plan_source(input: &proc_macro2::TokenStream) -> bool {
    use syn::parse::Parser;
    let parser = |input: ParseStream<'_>| -> syn::Result<bool> {
        // The checked protocol owns validation of these envelopes. Inspect only
        // enough of the packet to select the input language for this adapter.
        while input.peek(syn::Ident) {
            let name: syn::Ident = input.fork().parse()?;
            if name != "arguments" && name != "context" {
                break;
            }
            input.parse::<syn::Ident>()?;
            let content;
            syn::braced!(content in input);
            let _: proc_macro2::TokenStream = content.parse()?;
            input.parse::<syn::Token![;]>()?;
        }
        input.parse::<cq_stage::Pipeline>()?;
        input.parse::<syn::Token![;]>()?;
        input.parse::<cq_lang_sequence::Program>()?;
        Ok(true)
    };
    parser.parse2(input.clone()).unwrap_or(false)
}

/// Fill unauthored Sequence decisions using the Sequential machine and catalog.
/// Use in `decompose` directly or through `plan(mini_linq::passes::plan_sequential)`.
/// The unadvised default is robust Lookup/Expand; `preserve_order` keeps an authored order.
#[proc_macro]
pub fn plan_sequential(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_sequence::Program>(input.into(), |source, args| {
        let machine = cq_sequence_pull::machine();
        let catalog = cq_sequence_pull::catalog();
        let lookup_first = if args.is_empty() {
            true
        } else {
            let option: syn::Ident = syn::parse2(args)?;
            if option != "lookup_first" && option != "preserve_order" {
                return Err(syn::Error::new_spanned(
                    option,
                    "expected lookup_first or preserve_order",
                ));
            }
            option == "lookup_first"
        };
        let planned = if lookup_first {
            cq_sequence_plan::Planner::plan(
                &cq_sequence_plan::RuleBased,
                source,
                &machine,
                &catalog,
                &source.evidence,
            )?
        } else {
            cq_sequence_plan::decompose(source, &machine, &catalog)?
        };
        Ok(planned.program)
    })
    .into()
}

/// Validate Sequence placement against the host-only machine without rewriting it.
#[proc_macro]
pub fn sequential(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_sequence::Program>(input.into(), |source, args| {
        no_arguments(args)?;
        cq_lang_sequence::check_grading(source, &cq_sequence_pull::machine())?;
        Ok(source.clone())
    })
    .into()
}

/// Sequence → final Rust pull iterator, with optional runtime profile counters.
#[proc_macro]
pub fn translate_pull(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_sequence::Program, syn::Expr>(input.into(), |source, args| {
        no_arguments(args)?;
        let analysis = cq_lang_sequence::check_grading(source, &cq_sequence_pull::machine())?;
        let plain = cq_sequence_pull::translate(source, &analysis)?;
        let profile = cq_sequence_pull::translate_profile(source, &analysis)?;
        Ok(syn::parse_quote!(::cq_storage::profile_select! {
            plain { (#plain).into_iter() }
            counters { (#profile).into_iter() }
        }))
    })
    .into()
}

/// Generate typed source readers from relation declarations or a CQ callback.
#[proc_macro]
pub fn relation_loaders(input: TokenStream) -> TokenStream {
    load::expand(input)
}

/// Capture authored CQ and export a callback invoking a public consumer macro.
#[proc_macro]
pub fn workload_query(input: TokenStream) -> TokenStream {
    workload::expand(input)
}

#[cfg(test)]
mod tests {
    use super::{PlanOptions, expand_compile, expand_mini_linq};
    use cq_stage::Language;
    use quote::{ToTokens, quote};

    #[test]
    fn target_plan_dispatch_does_not_change_cq_planning_options() {
        for arguments in [
            quote!(),
            quote!(no_reduce),
            quote!(reduce(T with S)),
            quote!(skip(T with S)),
            quote!(order(R, S)),
        ] {
            assert!(!super::sequence_plan_target(
                &quote!(arguments { #arguments }; pipeline { next }; source)
            ));
        }
        assert!(super::sequence_plan_target(&quote!(
            arguments { ::target::plan }; pipeline { next }; source
        )));
    }

    #[test]
    fn plan_options_reduce_by_default_and_allow_explicit_opt_out() {
        assert!(!syn::parse2::<PlanOptions>(quote!()).unwrap().no_reduce);
        assert!(
            syn::parse2::<PlanOptions>(quote!(no_reduce))
                .unwrap()
                .no_reduce
        );
    }

    #[test]
    fn explicit_order_combines_with_existing_reduction_options() {
        let options: PlanOptions = syn::parse2(quote!(order(R, S), no_reduce)).unwrap();
        assert!(options.no_reduce);
        let expected: Vec<syn::Ident> = vec![syn::parse_quote!(R), syn::parse_quote!(S)];
        assert_eq!(options.order.unwrap(), expected);
        assert!(syn::parse2::<PlanOptions>(quote!(order(R), order(S))).is_err());
        assert!(syn::parse2::<PlanOptions>(quote!(order(R S))).is_err());
    }

    #[test]
    fn robust_sequence_default_keeps_authored_cq_advisory_precedence() {
        let source = quote! {
            struct Query;
            relation R(x:i32);
            answer(x) :- R(x);
        };
        let automatic = expand_compile(source.clone()).unwrap().to_string();
        assert!(automatic.contains("plan_sequential"));
        assert!(!automatic.contains("preserve_order"));
        let advised = expand_compile(quote! {
            pipeline { plan = custom_order; }
            #source
        }).unwrap().to_string();
        assert!(advised.contains("plan_sequential (preserve_order)"));
        let manual = expand_compile(quote! {
            pipeline { decompose = [authored_schedule]; }
            #source
        }).unwrap().to_string();
        assert!(manual.contains("authored_schedule"));
        assert!(!manual.contains("plan_sequential"));
    }

    #[test]
    fn sequence_entry_rejects_inapplicable_lowering_phases() {
        let config = syn::parse_quote!(pipeline { lower = custom; });
        assert!(super::sequence_pipeline(config).is_err());
    }

    #[test]
    fn sequence_entry_runs_authored_advice_before_the_selected_planner() {
        let config = syn::parse_quote!(pipeline {
            plan = custom(expected);
            decompose = [authored];
        });
        let pipeline = super::sequence_pipeline(config).unwrap();
        assert_eq!(
            pipeline.to_token_stream().to_string(),
            quote!(pipeline {
                authored,
                custom(expected),
                ::mini_linq::passes::sequential,
                ::mini_linq::passes::translate_pull
            })
            .to_string(),
        );
    }

    #[test]
    fn plan_options_reject_unknown_and_extra_arguments() {
        for arguments in [
            quote!(unknown),
            quote!(no_reduce no_reduce),
            quote!(no_reduce, no_reduce),
            quote!(no_reduce,),
            quote!(no_reduce = true),
        ] {
            assert!(syn::parse2::<PlanOptions>(arguments).is_err());
        }
    }

    fn physical(facts: proc_macro2::TokenStream) -> cq_lang_physical::Program {
        syn::parse2(quote!(physical {
            relation R(a: i32);
            #facts
            r = pipe rename R@0 { a -> x };
            return r as answer(x);
        }))
        .unwrap()
    }

    fn rejects_printed_entry<L: Language>(program: &L) {
        let printed = program.to_token_stream();
        for result in [
            expand_mini_linq(printed.clone()),
            expand_compile(quote!(pipeline { plan = custom; } #printed)),
        ] {
            assert!(result.unwrap_err().to_string().contains("CTX-Entry"));
        }
        let output = cq_stage::pass::<L, syn::Expr>(quote!(pipeline {}; #printed), |_, _| {
            panic!("unauthorized printed context reached transform")
        });
        assert!(output.to_string().contains("CTX-Entry"));
    }

    #[test]
    fn cq_entries_and_direct_pass_reject_printed_lower_ir_facts() {
        for facts in [
            quote!(consumer canonical;),
            quote!(host_set R discharged;),
            quote!(host_set R discharged; consumer canonical;),
        ] {
            let physical = physical(facts);
            rejects_printed_entry(&physical);
            let structures = cq_structure_selection::compile(&physical).unwrap();
            rejects_printed_entry(&structures);
            let sequence = cq_sequence_lowering::compile(&structures).unwrap();
            assert_eq!(sequence.context(), physical.context());
            rejects_printed_entry(&sequence);
        }
    }

    #[test]
    fn cq_entry_cannot_replay_checked_continuation_envelopes() {
        let program = physical(quote!(consumer canonical;));
        for input in [
            quote!(context { consumer = canonical }; #program),
            quote!(arguments {}; context { consumer = canonical }; #program),
        ] {
            assert!(expand_mini_linq(input.clone()).is_err());
            assert!(
                expand_compile(quote!(
                    pipeline { surface = [strip_envelope]; } #input
                ))
                .is_err()
            );
        }
    }

    #[test]
    fn plain_host_certificates_are_not_restricted_entry_facts() {
        let physical = physical(quote!(host_set R;));
        assert!(physical.context().is_empty());
        super::check_entry(&physical.to_token_stream()).unwrap();
    }

    #[test]
    fn configured_packets_are_forwarded_without_authorizing_cq_entry() {
        let cq = quote!(struct Q; relation R(a: i32); answer(x) :- R(x););
        let output = expand_compile(quote!(
            pipeline {
                lower = lower(consumer = canonical, host_set R discharged);
                select = select(positional(R));
            }
            #cq
        ))
        .unwrap();
        assert_eq!(
            output.to_string(),
            quote!(::mini_linq::passes::plan! {
                pipeline {
                    ::mini_linq::passes::index_demands,
                    lower(consumer = canonical, host_set R discharged),
                    select(positional(R)),
                    ::mini_linq::passes::represent,
                    ::mini_linq::passes::stage,
                    ::mini_linq::passes::sequence,
                    ::mini_linq::passes::sequential,
                    ::mini_linq::passes::translate_pull
                };
                #cq
            })
            .to_string()
        );
    }

    #[test]
    fn select_packet_authorization_survives_structures_continuation() {
        let physical = physical(quote!());
        let output =
            cq_stage::pass_with_context::<cq_lang_physical::Program, cq_lang_structures::Program>(
                quote!(
                    arguments { consumer = canonical, positional(R), host_set R discharged };
                    pipeline { printer };
                    #physical
                ),
                cq_structure_selection::context,
                cq_structure_selection::compile_with,
            );
        let invocation: syn::Macro = syn::parse2(output).unwrap();
        let output = cq_stage::pass::<cq_lang_structures::Program, syn::Expr>(
            invocation.tokens,
            |source, args| {
                assert!(args.is_empty());
                let context = source.context();
                assert!(context.consumer_canonical);
                assert_eq!(context.discharged_host_sets, ["R".to_owned()].into());
                Ok(syn::parse_quote!(42))
            },
        );
        assert_eq!(output.to_string(), "42");
    }
}
