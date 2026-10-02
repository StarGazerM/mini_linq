use cq_stage::Language;
use cq_testkit::{SCENARIOS, physical};
use quote::{ToTokens, quote};
use stacked_ir::{physical_to_trace as enter, trace, trace_to_physical as leave};

fn source() -> physical::Program {
    SCENARIOS
        .iter()
        .find(|s| s.name == "scan")
        .unwrap()
        .parse_physical()
        .unwrap()
}

#[test]
fn external_language_forwards_the_remaining_queue_and_opaque_arguments() {
    let source = source();
    let entered = enter::compile(&source).unwrap();
    let output = cq_stage::pass::<physical::Program, trace::Program>(
        quote! {
            pipeline { consumer::leave(opaque { a + (b * c) }), consumer::select, consumer::print };
            #source
        },
        |source, args| {
            assert!(args.is_empty());
            enter::compile(source)
        },
    );
    assert_eq!(
        output.to_string(),
        quote! {
            consumer::leave! {
                arguments { opaque { a + (b * c) } };
                pipeline { consumer::select, consumer::print };
                #entered
            }
        }
        .to_string()
    );

    let output = cq_stage::pass::<trace::Program, physical::Program>(
        quote! { pipeline { consumer::select, consumer::print }; #entered },
        |source, args| {
            assert!(args.is_empty());
            leave::compile(source)
        },
    );
    assert_eq!(
        output.to_string(),
        quote! {
            consumer::select! { pipeline { consumer::print }; #source }
        }
        .to_string()
    );
}

#[test]
fn trace_rewrites_use_the_language_correspondence_check() {
    let source = enter::compile(&source()).unwrap();
    let output = cq_stage::rewrite::<trace::Program>(
        quote! { pipeline { consumer::leave }; #source },
        |source, _| Ok(source.clone()),
    );
    assert_eq!(
        output.to_string(),
        quote! {
            consumer::leave! { pipeline {}; #source }
        }
        .to_string()
    );
    let output = cq_stage::rewrite::<trace::Program>(
        quote! { pipeline { consumer::leave }; #source },
        |source, _| {
            let mut target = source.clone();
            target.physical.program.head.relation = syn::parse_quote!(changed_answer);
            Ok(target)
        },
    );
    assert!(output.to_string().contains("compile_error"));
}

#[test]
fn trace_is_checked_at_both_boundaries_and_cannot_end_a_pipeline() {
    let source = source();
    let output = cq_stage::pass::<physical::Program, trace::Program>(
        quote! { pipeline {}; #source },
        |source, _| enter::compile(source),
    );
    assert!(
        output
            .to_string()
            .contains("pipeline ended before its Rust printer")
    );

    let mut invalid = enter::compile(&source).unwrap();
    invalid.physical.program.returned = syn::parse_quote!(missing);
    let output = cq_stage::pass::<trace::Program, physical::Program>(
        quote! { pipeline { consumer::select }; #invalid },
        |_, _| panic!("invalid source must not reach the pass"),
    );
    assert!(output.to_string().contains("compile_error"));

    let output = cq_stage::pass::<physical::Program, trace::Program>(
        quote! { pipeline { consumer::leave }; #source },
        |_, _| Ok(invalid),
    );
    assert!(output.to_string().contains("compile_error"));
    assert!(!trace::Program::terminal());
    assert!(!source.to_token_stream().is_empty());
}
