use cq_ir::rust_access_plan;
use quote::{ToTokens, quote};

fn triangle_plan_tokens() -> proc_macro2::TokenStream {
    quote! {
        triangle(x, y, z) => ((x.clone(), y.clone(), z.clone(),)) :-
            R(x, y) => for (x, y,) in (rows0.iter()),
            S(y, z) => for (z,) in
                (index0.get(&(y.clone(),)).into_iter().flatten()),
            T(z, x) => if (index1.contains(&(z.clone(), x.clone(),))).
    }
}

#[test]
fn annotated_triangle_round_trips_and_is_well_formed() {
    let first: rust_access_plan::Plan = syn::parse2(triangle_plan_tokens()).unwrap();
    rust_access_plan::contract::check(&first).unwrap();
    assert!(rust_access_plan::contract::well_formed(&first));

    let second: rust_access_plan::Plan = syn::parse2(first.to_token_stream()).unwrap();
    assert_eq!(first, second);
}

#[test]
fn output_is_one_rust_expression_for_the_lazy_result_iterator() {
    let plan: rust_access_plan::Plan = syn::parse2(triangle_plan_tokens()).unwrap();

    let syn::Expr::Tuple(output) = plan.head.output else {
        panic!("the fixture should return one Rust tuple expression per result")
    };
    assert_eq!(output.elems.len(), 3);
}

#[test]
fn access_shape_and_for_pattern_follow_source_order_bindings() {
    let invalid = [
        // Array/slice patterns belonged to the old homogeneous-row backend.
        quote! {
            answer(x) => ((x.clone(),)) :-
                Seed(x) => for [x] in (rows0.iter()).
        },
        // A clause with fresh variables enumerates; it cannot be a predicate.
        quote! {
            answer(x) => ((x.clone(),)) :-
                Seed(x) => if (rows0.contains(&(x.clone(),))).
        },
        // A fully bound occurrence is a predicate; it cannot bind again.
        quote! {
            answer(x) => ((x.clone(),)) :-
                Seed(x) => for (x,) in (rows0.iter()),
                Keep(x) => for () in (index0.get(&(x.clone(),)).into_iter()).
        },
        // The binding pattern follows fresh relation columns, not an arbitrary
        // variable ordering.
        quote! {
            answer(x, y) => ((x.clone(), y.clone(),)) :-
                Pair(x, y) => for (y, x,) in (rows0.iter()).
        },
        // Bound key variables are not returned by a proper-key lookup.
        quote! {
            answer(x, z) => ((x.clone(), z.clone(),)) :-
                Seed(x) => for (x,) in (rows0.iter()),
                Edge(x, z) => for (x, z,) in (index0.get(&(x.clone(),)).into_iter()).
        },
        // The output atom cannot mention a variable that no access bound.
        quote! {
            answer(y) => ((y.clone(),)) :-
                Seed(x) => for (x,) in (rows0.iter()).
        },
    ];

    for tokens in invalid {
        let plan: rust_access_plan::Plan = syn::parse2(tokens).unwrap();
        assert!(rust_access_plan::contract::check(&plan).is_err());
        assert!(!rust_access_plan::contract::well_formed(&plan));
    }
}

#[test]
fn raw_identifiers_compare_as_logical_symbols_in_binding_patterns() {
    let plan: rust_access_plan::Plan = syn::parse2(quote! {
        answer(r#x, z) => ((r#x.clone(), z.clone(),)) :-
            Seed(x) => for (r#x,) in (rows0.iter()),
            Edge(r#x, z) => for (z,) in (index0.get(&(r#x.clone(),)).into_iter()).
    })
    .unwrap();

    rust_access_plan::contract::check(&plan).unwrap();
}
