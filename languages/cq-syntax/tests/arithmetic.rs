use cq_syntax::arithmetic::{expression, predicate};
use quote::ToTokens;
use std::collections::BTreeMap;

fn types() -> BTreeMap<String, syn::Type> {
    [
        ("price".into(), syn::parse_quote!(i64)),
        ("discount".into(), syn::parse_quote!(i64)),
        ("date".into(), syn::parse_quote!(i32)),
        ("text".into(), syn::parse_quote!(String)),
    ]
    .into()
}

#[test]
fn recognizes_defined_arithmetic_and_range_predicates() {
    for source in [
        "price * (100 - discount)",
        "*price + *discount",
        "price / 7",
        "price / -1",
        "date - 365",
    ] {
        let lowered = expression(&syn::parse_str(source).unwrap(), &types(), None).unwrap();
        assert!(
            lowered.to_token_stream().to_string().contains("wrapping_"),
            "{source}"
        );
    }
    for source in [
        "price * discount > 10",
        "date + 1 >= 365 && date - 1 < 900",
        "!(price / -1 == 0)",
    ] {
        assert!(
            predicate(&syn::parse_str(source).unwrap(), &types()).is_some(),
            "{source}"
        );
    }
}

#[test]
fn rejects_partial_or_untyped_rust_without_extracting_boolean_branches() {
    for source in [
        "price / discount",
        "price / 0",
        "price / -0",
        "price % 3",
        "price << 1",
        "price.checked_mul(discount)",
        "price.wrapping_mul(discount)",
        "price + captured",
        "text + text",
        "price + date",
        "price + 1.0",
        "price + 1u32",
    ] {
        assert!(
            expression(&syn::parse_str(source).unwrap(), &types(), None).is_none(),
            "{source}"
        );
    }
    for source in [
        "price / discount > 0",
        "price * discount > 0 || opaque()",
        "!(price > 0 && opaque())",
    ] {
        assert!(
            predicate(&syn::parse_str(source).unwrap(), &types()).is_none(),
            "{source}"
        );
    }
}

#[test]
fn lowered_arithmetic_is_not_rust_panicking_arithmetic() {
    let lowered = expression(
        &syn::parse_quote!(*price * (100 - *discount) + *price / -1),
        &types(),
        None,
    )
    .unwrap();
    let tokens = lowered.to_token_stream().to_string();
    for operation in [
        "wrapping_mul",
        "wrapping_sub",
        "wrapping_add",
        "wrapping_div",
    ] {
        assert!(tokens.contains(operation), "{tokens}");
    }
}

#[test]
fn total_conditionals_and_lossless_widening_close_the_grammar() {
    let declared = types();
    for source in [
        "if *price == 1 || *price == 2 { 1 } else { 0 }",
        "if *discount > 0 { *price * *discount } else { *price / -1 }",
        "i64::from(*date) + *price",
        "i128::from(*price) * 10000",
    ] {
        let expected: syn::Type = if source.starts_with("i128") {
            syn::parse_quote!(i128)
        } else {
            syn::parse_quote!(i64)
        };
        assert!(
            expression(&syn::parse_str(source).unwrap(), &declared, Some(&expected)).is_some(),
            "{source}"
        );
    }
    for source in [
        "*price as i32",
        "i32::from(*price)",
        "u64::from(*price)",
        "if opaque() { 1 } else { 0 }",
        "if *price > 0 { *price / *discount } else { 0 }",
        "if *price > 0 { let x = 1; x } else { 0 }",
    ] {
        assert!(
            expression(
                &syn::parse_str(source).unwrap(),
                &declared,
                Some(&syn::parse_quote!(i64))
            )
            .is_none(),
            "{source}"
        );
    }
}

#[test]
fn decimal_coefficients_use_the_canonical_projection_not_a_method_promise() {
    let declared = [
        ("price".into(), syn::parse_quote!(Decimal)),
        ("discount".into(), syn::parse_quote!(cq_load::Decimal<2>)),
    ]
    .into();
    let result = expression(
        &syn::parse_quote!(price.scaled() * (100 - discount.scaled())),
        &declared,
        Some(&syn::parse_quote!(i64)),
    )
    .unwrap()
    .to_token_stream()
    .to_string();
    assert!(result.contains(":: cq_load :: Decimal :: scaled"));
    assert!(
        expression(
            &syn::parse_quote!(price.scaled() * discount.scaled()),
            &types(),
            Some(&syn::parse_quote!(i64))
        )
        .is_none()
    );
}

#[test]
fn typed_splitting_preserves_let_chain_shadowing() {
    let e = syn::parse_quote!(let Some(price) = captured && *price * 2 > 0);
    let (total, residual) = cq_syntax::guard::split_arithmetic(&e, &types());
    assert!(total.is_empty());
    assert_eq!(residual, Some(e));
}

#[test]
fn unannotated_reference_binding_is_not_a_numeric_projection() {
    let binding = syn::parse_quote!(bind input with output = (price));
    assert!(cq_syntax::arithmetic::binding(&binding, &types()).is_none());
    let binding = syn::parse_quote!(bind input with output: i64 = (*price));
    assert!(cq_syntax::arithmetic::binding(&binding, &types()).is_some());
}

#[test]
fn conditional_comparisons_keep_relational_trait_obligations() {
    let lowered = expression(
        &syn::parse_quote!(if text == "high" { 1 } else { 0 }),
        &types(),
        Some(&syn::parse_quote!(i64)),
    )
    .unwrap()
    .to_token_stream()
    .to_string();
    assert!(lowered.contains("relational_string_eq"));
}
