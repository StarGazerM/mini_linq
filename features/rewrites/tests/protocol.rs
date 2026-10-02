use cq_lang_physical as physical;
use cq_rewrite_passes::{canon_prefixes, merge_joins};
use cq_rewrite_test_driver::{capture_physical, corpus_ix, corpus_physical};
use quote::quote;

#[test]
fn default_lower_can_start_at_corpus_ix_and_forward_named_arguments() {
    let captured = corpus_ix! {
        intersection; pipeline {
            mini_linq::passes::lower,
            cq_rewrites::merge_joins(r2),
            capture_physical
        }
    };
    let output: physical::Program = syn::parse_str(captured).unwrap();
    assert_eq!(
        output,
        syn::parse_str(include_str!("fixtures/merge.physical")).unwrap()
    );
}

#[test]
fn rewritten_corpus_physical_flows_into_the_default_selector() {
    let captured = corpus_physical! {
        fine_triangle; pipeline {
            cq_rewrites::canon_prefixes(r1),
            mini_linq::passes::select,
            cq_rewrites::retype(dedup => ::std::collections::BTreeSet<_>),
            cq_rewrite_test_driver::capture_structures
        }
    };
    let output: cq_testkit::structures::Program = syn::parse_str(captured).unwrap();
    assert_eq!(
        output.physical,
        syn::parse_str(include_str!("fixtures/canon.physical")).unwrap()
    );
    let facts = cq_testkit::structures::contract::check(&output).unwrap();
    assert_eq!(facts.dedup.len(), 2);
    for ty in facts.dedup.values() {
        assert_eq!(*ty, syn::parse_quote!(::std::collections::BTreeSet<_>));
    }
}

#[test]
fn language_correspondence_is_enforced_before_continuation() {
    let source = cq_testkit::SCENARIOS
        .iter()
        .find(|s| s.name == "intersection")
        .unwrap()
        .parse_physical()
        .unwrap();
    let result = cq_stage::rewrite::<physical::Program>(
        quote! {
            pipeline { next }; #source
        },
        |source, _| {
            let mut invalid = source.clone();
            invalid.program.head.args = invalid.program.head.args.into_iter().rev().collect();
            Ok(invalid)
        },
    );
    assert!(result.to_string().contains("compile_error"));
    assert!(result.to_string().contains("RA erasure"));
    assert!(!result.to_string().contains("next !"));
}

#[test]
fn argument_and_queue_errors_are_not_ignored() {
    let source = cq_testkit::SCENARIOS
        .iter()
        .find(|s| s.name == "intersection")
        .unwrap()
        .parse_physical()
        .unwrap();
    for result in [
        cq_stage::rewrite(
            quote!(arguments { missing }; pipeline { next }; #source),
            merge_joins,
        ),
        cq_stage::rewrite(quote!(pipeline {}; #source), canon_prefixes),
    ] {
        assert!(result.to_string().contains("compile_error"));
    }
    let result = cq_stage::rewrite(
        quote! {
            arguments { r2 }; pipeline { next(payload => Custom<_>), last }; #source
        },
        merge_joins,
    );
    let expected =
        syn::parse_str::<physical::Program>(include_str!("fixtures/merge.physical")).unwrap();
    assert_eq!(
        result.to_string(),
        quote! {
            next! { arguments { payload => Custom<_> }; pipeline { last }; #expected }
        }
        .to_string()
    );
}
