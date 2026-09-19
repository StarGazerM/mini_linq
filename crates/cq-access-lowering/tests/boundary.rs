//! Access-to-Pipeline contracts tested without another compiler implementation.
use cq_access_lowering::to_pipeline::compile as compile_pull;
use cq_ir::{iterator_pipeline, rust_access_plan};
use quote::quote;

fn annotated_query() -> rust_access_plan::Plan {
    let source: rust_access_plan::Plan = syn::parse2(quote! {
        answer(x, y) => ((
            ::core::clone::Clone::clone(x),
            ::core::clone::Clone::clone(y),
        )) :-
            Left(x) => bind (x,) <- (left.iter()),
            Right(x, y) => bind (y,) <-
                (right.get(&(::core::clone::Clone::clone(x),)).into_iter().flatten()),
            Different(x, y) => if (*x != *y);
    })
    .unwrap();
    rust_access_plan::contract::check(&source).unwrap();
    source
}

#[test]
fn pull_exposes_every_binary_operator_and_intermediate_binding() {
    let actual = compile_pull(&annotated_query()).unwrap();
    iterator_pipeline::contract::check(&actual).unwrap();

    let expected: iterator_pipeline::Pipeline = syn::parse2(quote! {
        iter0 = scan (x,) in (left.iter()) yield (x,);
        iter1 = join iter0 as (x,) with (y,) in (
            right.get(&(::core::clone::Clone::clone(x),)).into_iter().flatten()
        ) yield (x, y,);
        iter2 = filter iter1 as (x, y,) if (*x != *y);
        iter3 = project iter2 as (x, y,) yield ((
            ::core::clone::Clone::clone(x),
            ::core::clone::Clone::clone(y),
        ));
        iter4 = distinct iter3;
        return iter4;
    })
    .unwrap();

    assert_eq!(actual, expected);
}

fn rejects<T>(lower: impl FnOnce() -> syn::Result<T>, diagnostic: &str) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(lower))
        .unwrap_or_else(|_| panic!("a public compiler boundary panicked on invalid IR"));
    let error = result.err().expect("invalid IR must return a diagnostic");
    assert!(error.to_string().contains(diagnostic), "{error}");
}

#[test]
fn access_checks_fresh_bindings_before_pipeline_construction() {
    let source: rust_access_plan::Plan = syn::parse_quote! {
        answer(x) => ((x,)) :- R(x) => if (true);
    };
    rejects(
        || cq_access_lowering::to_pipeline::compile(&source),
        "fresh variables",
    );
}

#[test]
fn correspondence_requires_a_valid_access_source() {
    let source: rust_access_plan::Plan = syn::parse_quote! {
        answer() => (()) :- ;
    };
    let invalid_source: rust_access_plan::Plan = syn::parse_quote! {
        answer(missing) => (()) :- ;
    };
    let target: iterator_pipeline::Pipeline = syn::parse_quote! {
        iter0 = unit yield ();
        iter1 = project iter0 as () yield (());
        iter2 = distinct iter1;
        return iter2;
    };

    assert!(rust_access_plan::contract::well_formed(&source));
    assert!(!rust_access_plan::contract::well_formed(&invalid_source));
    assert!(iterator_pipeline::contract::well_formed(&target));
    assert!(cq_access_lowering::to_pipeline::check(&source, &target));
    rejects(|| compile_pull(&invalid_source), "not bound");
    assert!(!cq_access_lowering::to_pipeline::check(
        &invalid_source,
        &target,
    ));
}

#[test]
fn scalar_items_and_empty_bodies_lower_through_the_existing_pull_pass() {
    let scalar: rust_access_plan::Plan = syn::parse_quote! {
        answer(row, field) => ((row, field)) :-
        Rows(row) => bind row <- (opaque_rows),
        Fields(row, field) => bind field <- (fields(row));
    };
    let pipeline = compile_pull(&scalar).unwrap();
    assert!(cq_access_lowering::to_pipeline::check(&scalar, &pipeline));
    let iterator_pipeline::Operator::Scan(scan) = &pipeline.definitions[0].operator else {
        panic!("the input item should remain the scan's binding");
    };
    assert!(matches!(scan.item_pattern, syn::Pat::Ident(_)));
    let iterator_pipeline::Operator::Join(join) = &pipeline.definitions[1].operator else {
        panic!("the dependent item should remain the join's binding");
    };
    assert!(matches!(join.item_pattern, syn::Pat::Ident(_)));

    let unit: rust_access_plan::Plan = syn::parse_quote! {
        answer() => (()) :- ;
    };
    let pipeline = compile_pull(&unit).unwrap();
    assert!(cq_access_lowering::to_pipeline::check(&unit, &pipeline));
    assert!(matches!(
        pipeline.definitions[0].operator,
        iterator_pipeline::Operator::Unit(_)
    ));
    assert_eq!(pipeline.definitions.len(), 3);
}
