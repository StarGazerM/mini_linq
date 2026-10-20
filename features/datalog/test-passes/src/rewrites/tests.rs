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
    PassCase {
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
    PassCase {
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
    });
    expected.reads[0].index = 1;
    expected.reads[0].columns = vec![1, 0];
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
    // Independent protocol check: cq_stage invokes the rewrite contract and
    // forwards the changed payload, rather than testing a private shortcut.
    let tokens = cq_stage::rewrite::<storage::Program>(
        quote!(pipeline { downstream::consume }; #result),
        |p, _| Ok(p.clone()),
    );
    assert!(tokens.to_string().contains("downstream :: consume"));
    assert!(!tokens.to_string().contains("compile_error"));
}
