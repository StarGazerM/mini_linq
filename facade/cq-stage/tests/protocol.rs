use cq_stage::{Language, pass, rewrite};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::parse::{Parse, ParseStream};

#[derive(Clone)]
struct Number(syn::LitInt);
impl Parse for Number {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse().map(Self)
    }
}
impl ToTokens for Number {
    fn to_tokens(&self, output: &mut TokenStream) {
        self.0.to_tokens(output);
    }
}
impl Language for Number {
    fn check(&self) -> syn::Result<()> {
        if self.0.base10_parse::<u32>()? == 0 {
            Err(syn::Error::new_spanned(&self.0, "zero is inadmissible"))
        } else {
            Ok(())
        }
    }
    fn check_rewrite(source: &Self, target: &Self) -> syn::Result<()> {
        if source.0 == target.0 {
            Ok(())
        } else {
            Err(syn::Error::new_spanned(&target.0, "value changed"))
        }
    }
}
#[test]
fn target_contract_runs_before_continuation() {
    let tokens = pass::<Number, Number>(quote!(pipeline { consumer }; 1), |_, _| {
        Ok(Number(syn::parse_quote!(0)))
    });
    assert!(tokens.to_string().contains("zero is inadmissible"));
    assert!(!tokens.to_string().contains("consumer !"));
}
#[test]
fn rewrite_correspondence_is_additional_to_the_contract() {
    let tokens = rewrite::<Number>(quote!(pipeline { consumer }; 1), |_, _| {
        Ok(Number(syn::parse_quote!(2)))
    });
    assert!(tokens.to_string().contains("value changed"));
}
#[test]
fn source_contract_precedes_the_transform() {
    let tokens = pass::<Number, Number>(quote!(pipeline { consumer }; 0), |_, _| {
        panic!("invalid source reached f")
    });
    assert!(tokens.to_string().contains("zero is inadmissible"));
}
#[test]
fn opaque_arguments_and_remaining_pipeline_are_forwarded() {
    let tokens = rewrite::<Number>(
        quote!(arguments { x + (y * z) }; pipeline { other(a ! { b }), final_pass }; 1),
        |source, args| {
            assert_eq!(args.to_string(), "x + (y * z)");
            Ok(source.clone())
        },
    );
    assert_eq!(
        tokens.to_string(),
        quote!(other! { arguments { a ! { b } }; pipeline { final_pass }; 1 }).to_string()
    );
}
#[test]
fn only_a_terminal_printer_can_end_the_pipeline() {
    let tokens = pass::<Number, syn::Expr>(quote!(pipeline {}; 1), |n, _| {
        syn::parse2(n.to_token_stream())
    });
    assert_eq!(tokens.to_string(), "1");
    let tokens = rewrite::<Number>(quote!(pipeline {}; 1), |n, _| Ok(n.clone()));
    assert!(tokens.to_string().contains("before its Rust printer"));
}
#[test]
fn named_grammar_is_generic_and_rejects_duplicates() {
    let config: cq_stage::Configuration =
        syn::parse2(quote!(pipeline { plan = a(x); physical = [b, c(y)]; surface = [d]; }))
            .unwrap();
    let defaults: cq_stage::Configuration =
        syn::parse2(quote!(pipeline { plan = w; ix = x; lower = y; select = z; represent = r; stage = s; print = p; }))
            .unwrap();
    let pipeline = config.resolve(defaults).unwrap();
    assert_eq!(
        pipeline.to_token_stream().to_string(),
        quote!(pipeline { d, a(x), x, y, b, c(y), z, r, s, p }).to_string()
    );
    assert!(
        syn::parse2::<cq_stage::Configuration>(quote!(pipeline { plan = a; plan = b; })).is_err()
    );
    assert!(syn::parse2::<cq_stage::Configuration>(quote!(pipeline { unknown = a; })).is_err());
}

#[test]
fn representation_and_stage_overrides_keep_other_defaults() {
    let config: cq_stage::Configuration = syn::parse2(quote!(pipeline {
        represent = positional(positions);
        stage = chunked(batch(64));
    }))
    .unwrap();
    let defaults: cq_stage::Configuration = syn::parse2(quote!(pipeline {
        plan = p; ix = i; lower = l; select = s;
        represent = r; stage = b; print = out;
    }))
    .unwrap();
    assert_eq!(
        config
            .resolve(defaults)
            .unwrap()
            .to_token_stream()
            .to_string(),
        quote!(pipeline { p, i, l, s, positional(positions), chunked(batch(64)), out }).to_string()
    );
}

#[test]
fn sequence_planning_follows_all_authored_advice() {
    let config: cq_stage::Configuration = syn::parse_quote!(pipeline {
        plan = cq_plan; ix = ix; lower = lower; select = select;
        represent = represent; stage = stage; sequence = sequence;
        decompose = [
            authored_grade, authored_tile, authored_output,
            mini_linq::passes::plan(target::plan)
        ];
        grade = machine_grade; print = translate;
    });
    assert_eq!(
        config
            .resolve(Default::default())
            .unwrap()
            .to_token_stream()
            .to_string(),
        quote!(pipeline {
            cq_plan, ix, lower, select, represent, stage, sequence,
            authored_grade, authored_tile, authored_output,
            mini_linq::passes::plan(target::plan), machine_grade, translate
        })
        .to_string(),
    );
}

#[test]
fn target_delegation_preserves_context_and_continuation_verbatim() {
    let packet = quote!(
        context { consumer = canonical, host_set R discharged };
        pipeline { grade, printer };
        opaque source tokens
    );
    assert_eq!(
        cq_stage::delegate(quote!(arguments { target::plan }; #packet)).to_string(),
        quote!(target::plan! { #packet }).to_string(),
    );
    assert_eq!(
        cq_stage::delegate(quote!(arguments { target::plan }; pipeline {}; 1)).to_string(),
        quote!(target::plan! { pipeline {}; 1 }).to_string(),
    );
    assert!(
        cq_stage::delegate(quote!(arguments { target::plan, extra }; pipeline {}; 1))
            .to_string()
            .contains("expected a target planner macro path"),
    );
}

#[test]
fn sequence_phases_are_ordered_and_require_a_complete_target() {
    let defaults: cq_stage::Configuration = syn::parse_quote!(pipeline {
        plan = p; ix = i; lower = l; select = s;
        represent = r; stage = b; print = custom::out;
    });
    let config: cq_stage::Configuration = syn::parse_quote!(pipeline {
        surface = [surface];
        physical = [physical];
        structures = [structures];
        sequence = sequence;
        decompose = [tile(32), store];
        grade = sequential;
        print = translate_pull;
    });
    assert_eq!(
        config
            .resolve(defaults.clone())
            .unwrap()
            .to_token_stream()
            .to_string(),
        quote!(pipeline {
            surface, p, i, l, physical, s, structures, r, b,
            sequence, tile(32), store, sequential, translate_pull
        })
        .to_string()
    );
    for invalid in [
        quote!(sequence = sequence;),
        quote!(grade = sequential;),
        quote!(sequence = sequence; grade = sequential;),
        quote!(sequence = sequence; print = translate_pull;),
        quote!(grade = sequential; print = translate_pull;),
        quote!(sequence = sequence; grade = sequential; print_input = structures; print = own_push::push;),
        quote!(decompose = [tile(32)];),
    ] {
        let config: cq_stage::Configuration = syn::parse2(quote!(pipeline { #invalid })).unwrap();
        assert!(config.resolve(defaults.clone()).is_err(), "{invalid}");
    }
    let custom: cq_stage::Configuration = syn::parse_quote!(pipeline {
        sequence = lower_custom; grade = target_custom; print = custom::translate;
    });
    assert!(custom.resolve(defaults.clone()).is_ok());
    let mut sequence_defaults = defaults;
    sequence_defaults.sequence = Some(syn::parse_quote!(sequence));
    sequence_defaults.grade = Some(syn::parse_quote!(sequential));
    sequence_defaults.print = Some(syn::parse_quote!(translate_pull));
    assert!(
        cq_stage::Configuration::default()
            .resolve(sequence_defaults.clone())
            .is_ok()
    );
    let push: cq_stage::Configuration = syn::parse_quote!(pipeline {
        print_input = structures;
        print = own_push::push;
    });
    assert_eq!(
        push.resolve(sequence_defaults.clone())
            .unwrap()
            .to_token_stream()
            .to_string(),
        quote!(pipeline { p, i, l, s, r, b, own_push::push }).to_string()
    );
    let pull: cq_stage::Configuration = syn::parse_quote!(pipeline {
        print = translate_pull;
    });
    assert_eq!(
        pull.resolve(sequence_defaults)
            .unwrap()
            .to_token_stream()
            .to_string(),
        quote!(pipeline {
            p,
            i,
            l,
            s,
            r,
            b,
            sequence,
            sequential,
            translate_pull
        })
        .to_string()
    );
    // A terminal's name is opaque: this Sequence endpoint can be called `push`.
    let renamed: cq_stage::Configuration = syn::parse_quote!(pipeline {
        sequence = lower_custom; grade = target_custom;
        print_input = sequence; print = custom::push;
    });
    assert!(
        renamed
            .resolve(syn::parse_quote!(pipeline {
                plan = p; ix = i; lower = l; select = s;
                represent = r; stage = b; print = custom::out;
            }))
            .is_ok()
    );
}

#[derive(Clone)]
struct Facts(cq_stage::Context);

impl Parse for Facts {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let contents;
        syn::braced!(contents in input);
        Ok(Self(contents.parse()?))
    }
}

impl ToTokens for Facts {
    fn to_tokens(&self, output: &mut TokenStream) {
        let facts = &self.0;
        output.extend(quote!({ #facts }));
    }
}

impl Language for Facts {
    fn check(&self) -> syn::Result<()> {
        Ok(())
    }

    fn check_rewrite(_: &Self, _: &Self) -> syn::Result<()> {
        Ok(())
    }

    fn context(&self) -> cq_stage::Context {
        self.0.clone()
    }
}

fn next_input(tokens: TokenStream) -> TokenStream {
    syn::parse2::<syn::Macro>(tokens).unwrap().tokens
}

#[test]
fn recorded_facts_require_exact_envelope_on_direct_entry() {
    for facts in [
        quote!(consumer = canonical),
        quote!(host_set R discharged),
        quote!(consumer = canonical, host_set R discharged),
    ] {
        let output = pass::<Facts, Facts>(quote!(pipeline { printer }; { #facts }), |_, _| {
            panic!("unauthorized facts reached transform")
        });
        assert!(output.to_string().contains("CTX-Entry"));

        // Opaque packets at ordinary passes cannot authorize new facts.
        let output = pass::<Facts, Facts>(
            quote!(arguments { #facts }; pipeline { printer }; { #facts }),
            |_, _| panic!("ordinary pass authorized facts"),
        );
        assert!(output.to_string().contains("CTX-Entry"));
    }
}

#[test]
fn matching_lower_select_packets_cannot_replace_source_envelopes() {
    for facts in [
        quote!(consumer = canonical),
        quote!(host_set R discharged),
        quote!(consumer = canonical, host_set R discharged),
    ] {
        let output = cq_stage::pass_with_context::<Facts, Facts>(
            quote!(arguments { #facts }; pipeline { printer }; { #facts }),
            syn::parse2,
            |_, _| panic!("matching packet authorized factful reentry"),
        );
        assert!(output.to_string().contains("compile_error"));
        assert!(output.to_string().contains("CTX-Entry"));

        let output = cq_stage::pass_with_context::<Facts, Facts>(
            quote!(
                arguments { #facts };
                context { #facts };
                pipeline { printer };
                { #facts }
            ),
            syn::parse2,
            |source, _| Ok(source.clone()),
        );
        assert_eq!(
            output.to_string(),
            quote!(printer! { context { #facts }; pipeline {}; { #facts } }).to_string()
        );
    }
}

#[test]
fn packets_cannot_add_facts_to_already_factful_sources() {
    let output = cq_stage::pass_with_context::<Facts, Facts>(
        quote!(
            arguments { consumer = canonical, host_set R discharged };
            context { consumer = canonical };
            pipeline { printer };
            { consumer = canonical }
        ),
        syn::parse2,
        |_, _| panic!("packet added facts to a factful source"),
    );
    assert!(output.to_string().contains("CTX-Entry"));
    assert!(output.to_string().contains("fact-free source"));
}

#[test]
fn checked_continuation_preserves_facts_and_opaque_arguments() {
    let output = cq_stage::pass_with_context::<Facts, Facts>(
        quote!(
            arguments { consumer = canonical, host_set R discharged };
            pipeline { arbitrary(x + (y * z)), printer };
            {}
        ),
        syn::parse2,
        |_, args| Ok(Facts(syn::parse2(args)?)),
    );
    let output = rewrite::<Facts>(next_input(output), |source, args| {
        assert_eq!(args.to_string(), "x + (y * z)");
        Ok(source.clone())
    });
    let output = pass::<Facts, syn::Expr>(next_input(output), |source, args| {
        assert!(source.0.consumer_canonical);
        assert!(source.0.discharged_host_sets.contains("R"));
        assert!(args.is_empty());
        Ok(syn::parse_quote!(42))
    });
    assert_eq!(output.to_string(), "42");
}

#[test]
fn printed_program_reentry_does_not_inherit_context() {
    let source: Facts = syn::parse_quote!({ consumer = canonical, host_set R discharged });
    let pipeline: cq_stage::Pipeline = syn::parse_quote!(pipeline {
        continuation,
        printer
    });
    let invocation = pipeline.start(source.to_token_stream()).unwrap();
    let output = pass::<Facts, Facts>(next_input(invocation), |_, _| {
        panic!("printed facts were trusted on reentry")
    });
    assert!(output.to_string().contains("compile_error"));
    assert!(output.to_string().contains("CTX-Entry"));
}

#[test]
fn continuation_context_must_equal_source_not_authorize_future_facts() {
    for input in [
        quote!(
            context { consumer = canonical };
            pipeline { printer }; {}
        ),
        quote!(
            context { consumer = canonical };
            pipeline { printer }; { consumer = canonical, host_set R discharged }
        ),
        quote!(
            context { consumer = canonical, host_set R discharged };
            pipeline { printer }; { consumer = canonical }
        ),
    ] {
        let output = rewrite::<Facts>(input, |_, _| panic!("stale context reached transform"));
        assert!(output.to_string().contains("exactly match"));
        assert!(output.to_string().contains("CTX-Entry"));
    }

    let output = rewrite::<Facts>(
        quote!(
            context { consumer = canonical };
            pipeline { printer }; { consumer = canonical }
        ),
        |_, _| Ok(syn::parse_quote!({ consumer = canonical, host_set R discharged })),
    );
    assert!(output.to_string().contains("only lower/select"));
}

#[test]
fn ordinary_pass_cannot_introduce_facts_and_packets_must_record_them() {
    let output = pass::<Facts, Facts>(
        quote!(arguments { consumer = canonical }; pipeline { printer }; {}),
        |_, _| Ok(syn::parse_quote!({ consumer = canonical })),
    );
    assert!(output.to_string().contains("only lower/select"));

    let output = cq_stage::pass_with_context::<Facts, Facts>(
        quote!(arguments { consumer = canonical }; pipeline { printer }; {}),
        syn::parse2,
        |source, _| Ok(source.clone()),
    );
    assert!(output.to_string().contains("record every authorized"));
}

#[test]
fn dropping_recorded_facts_drops_the_continuation_envelope() {
    let output = pass::<Facts, Facts>(
        quote!(
            context { consumer = canonical };
            pipeline { printer }; { consumer = canonical }
        ),
        |_, _| Ok(syn::parse_quote!({})),
    );
    assert_eq!(
        output.to_string(),
        quote!(printer! { pipeline {}; {} }).to_string()
    );
}

#[test]
fn dropping_one_fact_forwards_only_remaining_facts() {
    let output = pass::<Facts, Facts>(
        quote!(
            context { consumer = canonical, host_set R discharged };
            pipeline { continuation, printer };
            { consumer = canonical, host_set R discharged }
        ),
        |_, _| Ok(syn::parse_quote!({ host_set R discharged })),
    );
    assert_eq!(
        output.to_string(),
        quote!(continuation! {
            context { host_set R discharged };
            pipeline { printer };
            { host_set R discharged }
        })
        .to_string()
    );
    let output = pass::<Facts, Facts>(next_input(output), |source, _| Ok(source.clone()));
    assert!(!output.to_string().contains("compile_error"));
}

#[test]
fn continuation_context_normalizes_raw_host_names() {
    let facts: cq_stage::Context = syn::parse_quote!(host_set r#type discharged);
    assert!(facts.discharged_host_sets.contains("type"));
    assert_eq!(
        syn::parse2::<cq_stage::Context>(facts.to_token_stream()).unwrap(),
        facts
    );
    assert!(
        syn::parse2::<cq_stage::Context>(quote!(host_set R discharged, host_set r#R discharged))
            .is_err()
    );
}

#[test]
fn observation_captures_checked_nonterminal_and_terminal_payloads() {
    let observed = rewrite::<Number>(
        quote!(pipeline {} observe { ::core::stringify }; 1),
        |source, _| Ok(source.clone()),
    );
    assert_eq!(
        observed.to_string(),
        quote!(::core::stringify! { 1 }).to_string()
    );
    let rejected = rewrite::<Number>(
        quote!(pipeline {} observe { ::core::stringify }; 1),
        |_, _| Ok(Number(syn::parse_quote!(0))),
    );
    assert!(rejected.to_string().contains("zero is inadmissible"));
    let rust = pass::<Number, syn::Expr>(
        quote!(pipeline {} observe { ::core::stringify }; 1),
        |_, _| Ok(syn::parse_quote!(unbound_input.into_iter())),
    );
    assert_eq!(
        rust.to_string(),
        quote!(::core::stringify! { unbound_input.into_iter() }).to_string()
    );
}

#[test]
fn observation_survives_continuations_without_changing_arguments() {
    let tokens = rewrite::<Number>(
        quote!(pipeline { next(opaque) } observe { receiver }; 1),
        |source, _| Ok(source.clone()),
    );
    assert_eq!(
        tokens.to_string(),
        quote!(next! { arguments { opaque }; pipeline {} observe { receiver }; 1 }).to_string()
    );
}

#[test]
fn observed_boundaries_are_prefixes_of_the_resolved_queue() {
    use cq_stage::Boundary;
    let defaults: cq_stage::Configuration = syn::parse_quote!(pipeline {
        plan = p; ix = i; lower = l; select = s; represent = r; stage = t;
        sequence = q; decompose = [d]; grade = g; print = end;
    });
    for (boundary, count) in [
        (Boundary::Cq, 0),
        (Boundary::RelationalPlan, 1),
        (Boundary::IndexRequirements, 2),
        (Boundary::Physical, 3),
        (Boundary::Structures, 4),
        (Boundary::Represent, 5),
        (Boundary::Stage, 6),
        (Boundary::Sequence, 7),
        (Boundary::Decompose, 8),
        (Boundary::Grade, 9),
        (Boundary::Rust, 10),
    ] {
        let ordinary = cq_stage::Configuration::default()
            .resolve(defaults.clone())
            .unwrap();
        let observed = cq_stage::Configuration::default()
            .resolve_observed(defaults.clone(), boundary, syn::parse_quote!(receiver))
            .unwrap();
        assert_eq!(observed.passes.len(), count);
        assert_eq!(
            observed
                .passes
                .iter()
                .map(ToTokens::to_token_stream)
                .collect::<Vec<_>>()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ordinary.passes[..count]
                .iter()
                .map(ToTokens::to_token_stream)
                .collect::<Vec<_>>()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
        );
    }
}
