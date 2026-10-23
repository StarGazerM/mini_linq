use super::*;
use cq_datalog_testkit::PassCase;
use quote::quote;

#[test]
fn source_duplicate_fixture_and_interface_mutations() {
    let source: source::Program = syn::parse_quote! {
        struct Q; relation E(x:i32) = (rows); relation R(x:i32); output R;
        R(x) :- E(x); R(x) :- E(x);
    };
    let expected: source::Program = syn::parse_quote! {
        struct Q; relation E(x:i32) = (rows); relation R(x:i32); output R;
        R(x) :- E(x);
    };
    let reference = crate::reference::emit(&source).unwrap();
    let target = PassCase {
        source,
        expected_next: Some(expected),
        compile: Box::new(deduplicate_source),
        check: Box::new(source::check_rewrite),
        mutations: vec![
            Box::new(|p: &source::Program| {
                let mut p = p.clone();
                p.outputs = vec![syn::parse_quote!(E)];
                p
            }),
            Box::new(|p: &source::Program| {
                let mut p = p.clone();
                p.name = syn::parse_quote!(Changed);
                p
            }),
            Box::new(|p: &source::Program| {
                let mut p = p.clone();
                p.declarations[0].columns[0].ty = syn::parse_quote!(i64);
                p
            }),
            Box::new(|p: &source::Program| {
                let mut p = p.clone();
                p.rules.clear();
                p
            }),
        ],
        answers: None,
    }
    .run(None)
    .unwrap();
    crate::reference_tests::check(
        reference,
        crate::reference::emit(&target).unwrap(),
        quote!(let rows: [(i32,); 4] = [(0,), (1,), (1,), (-3,)];),
    );
}

#[test]
fn strata_merge_fixture_and_dependency_mutations() {
    let source: strata::Program = syn::parse_quote! {
        struct Q; relation E(x:i32) = (rows);
        relation A(x:i32); relation B(x:i32); relation C(x:i32); output A, B, C;
        A(x) :- E(x); B(x) :- E(x); C(x) :- E(x), !A(x);
        strata { component [A]; component [B]; component [C]; }
    };
    let expected: strata::Program = syn::parse_quote! {
        struct Q; relation E(x:i32) = (rows);
        relation A(x:i32); relation B(x:i32); relation C(x:i32); output A, B, C;
        A(x) :- E(x); B(x) :- E(x); C(x) :- E(x), !A(x);
        strata { component [A,B]; component [C]; }
    };
    let reference = crate::reference::emit_strata(&source).unwrap();
    let target = PassCase {
        source,
        expected_next: Some(expected),
        compile: Box::new(merge_strata),
        check: Box::new(strata::check_rewrite),
        mutations: vec![
            Box::new(|p: &strata::Program| {
                let mut p = p.clone();
                p.components.reverse();
                p
            }),
            Box::new(|p: &strata::Program| {
                let mut p = p.clone();
                let c = p.components.pop().unwrap();
                p.components[0].extend(c);
                p
            }),
            Box::new(|p: &strata::Program| {
                let mut p = p.clone();
                p.components[0].pop();
                p
            }),
            Box::new(|p: &strata::Program| {
                let mut p = p.clone();
                p.source.rules.push(p.source.rules[0].clone());
                p
            }),
        ],
        answers: None,
    }
    .run(None)
    .unwrap();
    crate::reference_tests::check(
        reference,
        crate::reference::emit_strata(&target).unwrap(),
        quote!(let rows: [(i32,); 4] = [(0,), (1,), (1,), (-3,)];),
    );
}

#[test]
fn storage_alternative_fixture_and_binding_mutations() {
    let source: source::Program = syn::parse_quote! {
        struct Q; relation E(x:i32,y:i32) = (rows); relation R(x:i32,y:i32); output R;
        R(x,y) :- E(x,y);
    };
    let strata = cq_datalog_stratify::compile(&source).unwrap();
    let incremental = cq_datalog_incremental::compile(&strata).unwrap();
    let source = cq_datalog_storage::compile(&incremental).unwrap();
    // Authored expected alternative: default scan uses payload order [0,1];
    // the plugin adds the other complete zero-key permutation and rebinds it.
    let mut expected = source.clone();
    expected.layouts.push(storage::Layout {
        relation: syn::parse_quote!(E),
        columns: vec![1, 0],
        key: 0,
        provider: syn::parse_quote!(::std::collections::HashMap<(), ::std::vec::Vec<(i32, i32,)>, ::cq_storage::HashState>),
        maintenance: syn::parse_quote!(admitted_rows),
    });
    expected.reads[0].index = 1;
    expected.reads[0].columns = vec![1, 0];
    // Storage's alpha map evaluates the embedded Incremental program.
    let reference = crate::reference_incremental::emit(&source.incremental).unwrap();
    let result = PassCase {
        source,
        expected_next: Some(expected),
        compile: Box::new(alternate_storage),
        check: Box::new(storage::check_rewrite),
        mutations: vec![
            Box::new(|p: &storage::Program| {
                let mut p = p.clone();
                p.layouts[1].key = 1;
                p
            }),
            Box::new(|p: &storage::Program| {
                let mut p = p.clone();
                p.layouts[1].columns = vec![0, 0];
                p
            }),
            Box::new(|p: &storage::Program| {
                let mut p = p.clone();
                p.reads[0].columns = vec![0, 1];
                p
            }),
            Box::new(|p: &storage::Program| {
                let mut p = p.clone();
                p.reads[0].index = usize::MAX;
                p
            }),
            Box::new(|p: &storage::Program| {
                let mut p = p.clone();
                p.incremental.strata.source.name = syn::parse_quote!(Changed);
                p
            }),
        ],
        answers: None,
    }
    .run(None)
    .unwrap();
    crate::reference_tests::check(
        reference,
        crate::reference_incremental::emit(&result.incremental).unwrap(),
        quote!(let rows: [(i32, i32); 4] = [(0, 1), (1, 2), (0, 1), (-3, 4)];),
    );
    // Independent protocol check: cq_stage invokes the rewrite contract and
    // forwards the changed payload, rather than testing a private shortcut.
    let tokens = cq_stage::rewrite::<storage::Program>(
        quote!(pipeline { downstream::consume }; #result),
        |p, _| Ok(p.clone()),
    );
    assert!(tokens.to_string().contains("downstream :: consume"));
    assert!(!tokens.to_string().contains("compile_error"));
}

#[test]
fn concrete_choices_are_nontrivial_checked_rewrites() {
    let source: source::Program = syn::parse_quote! {
        struct Choices; relation E(x:i32,y:i32) = (rows);
        relation A(x:i32,y:i32); relation B(x:i32,y:i32); output A;
        A(x,y) :- E(x,y);
        A(x,z) :- E(x,y), B(y,z);
        B(x,y) :- A(x,y);
    };
    let strata = cq_datalog_stratify::compile(&source).unwrap();
    let incremental = cq_datalog_incremental::compile(&strata).unwrap();
    let storage = cq_datalog_storage::compile(&incremental).unwrap();
    let ordered = ordered_storage(&storage).unwrap();
    assert_ne!(storage.layouts[0].provider, ordered.layouts[0].provider);
    storage::check_rewrite(&storage, &ordered).unwrap();
    let direct = direct_later_delta(&ordered).unwrap();
    assert!(
        direct
            .reads
            .iter()
            .any(|read| read.kind == "direct" && read.position > 0)
    );
    assert_ne!(direct.reads, ordered.reads);
    storage::check_rewrite(&ordered, &direct).unwrap();
    let round = cq_datalog_fork::compile(&direct).unwrap();
    let reversed = reverse_publication(&round).unwrap();
    assert!(
        round
            .context
            .strata
            .iter()
            .zip(&reversed.context.strata)
            .any(|(a, b)| a.publication != b.publication)
    );
    cq_lang_datalog_round::check_rewrite(&round, &reversed).unwrap();
}

fn pipeline_fixture() -> cq_lang_datalog_round::Program {
    let source: source::Program = syn::parse_quote! {
        struct Choices; relation E(x:i32,y:i32) = (rows);
        relation R(x:i32,y:i32); output R;
        R(*x,*y) :- E(x,y);
        R(*x,*z) :- R(x,y), E(y,z);
    };
    let strata = cq_datalog_stratify::compile(&source).unwrap();
    let incremental = cq_datalog_incremental::compile(&strata).unwrap();
    let storage = cq_datalog_storage::compile(&incremental).unwrap();
    cq_datalog_fork::compile(&storage).unwrap()
}

#[test]
fn independent_hole_arguments_preserve_the_checked_round() {
    use cq_lang_datalog_round::{HoleInterface, contract};
    let source = pipeline_fixture();
    for (batch, positions) in [(0usize, 1usize), (1, 0)] {
        let target = per_hole_pipeline(&source, quote!(#batch, #positions)).unwrap();
        assert_ne!(source, target);
        contract::check(&target).unwrap();
        cq_lang_datalog_round::check_rewrite(&source, &target).unwrap();
        for (index, (before, after)) in source.holes.iter().zip(&target.holes).enumerate() {
            assert_eq!(before.cq, after.cq);
            assert_eq!(after.interface, HoleInterface::Pull);
            assert_eq!(before.pipeline.passes.len(), after.pipeline.passes.len());
            for (old, new) in before.pipeline.passes.iter().zip(&after.pipeline.passes) {
                assert_eq!(old.path, new.path);
                let name = new.path.segments.last().unwrap().ident.to_string();
                let expected = if index == batch && name == "stage" {
                    quote!(batch(64))
                } else if index == positions && name == "represent" {
                    quote!(positions)
                } else {
                    old.arguments.clone()
                };
                assert_eq!(new.arguments.to_string(), expected.to_string());
            }
        }
        let mut restored = target;
        for (hole, original) in restored.holes.iter_mut().zip(&source.holes) {
            hole.pipeline = original.pipeline.clone();
        }
        assert_eq!(restored, source);
        let forwarded = cq_stage::rewrite::<cq_lang_datalog_round::Program>(
            quote!(arguments { #batch, #positions }; pipeline { downstream::consume }; #source),
            per_hole_pipeline,
        );
        assert!(!forwarded.to_string().contains("compile_error"));
        assert!(forwarded.to_string().contains("downstream :: consume"));
    }
}

#[test]
fn independent_hole_choices_reject_identical_missing_and_key_holes() {
    let source = pipeline_fixture();
    assert!(per_hole_pipeline(&source, quote!(0, 0)).is_err());
    assert!(per_hole_pipeline(&source, quote!(0, 999)).is_err());
    assert!(per_hole_pipeline(&source, quote!(0)).is_err());
    let source: source::Program = syn::parse_quote! {
        struct Negation; relation E(x:i32) = (rows); relation B(x:i32) = (blocked);
        relation R(x:i32); output R;
        R(*x) :- E(x), !B(x);
    };
    let strata = cq_datalog_stratify::compile(&source).unwrap();
    let incremental = cq_datalog_incremental::compile(&strata).unwrap();
    let storage = cq_datalog_storage::compile(&incremental).unwrap();
    let round = cq_datalog_fork::compile(&storage).unwrap();
    let component = &round.context.strata[0];
    let key = component.keys[0].hole;
    let rule = component.seeds[0].hole;
    assert!(per_hole_pipeline(&round, quote!(#key, #rule)).is_err());
}
