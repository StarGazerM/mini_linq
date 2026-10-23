use cq_syntax::algebra::{AggregateKind, Group};
use quote::ToTokens;
use std::collections::BTreeMap;

#[test]
fn grouped_fold_syntax_round_trips() {
    let group: Group = syn::parse_quote! {
        group rows by [key] {
            n: usize = count();
            total: i64 = sum(value);
            lo: Option<i64> = min(value);
            hi: Option<i64> = max(value);
        }
    };
    assert_eq!(group.aggregates[0].kind, AggregateKind::Count);
    assert_eq!(
        syn::parse2::<Group>(group.to_token_stream()).unwrap(),
        group
    );
    group
        .check(&BTreeMap::from([
            ("key".into(), syn::parse_quote!(i32)),
            ("value".into(), syn::parse_quote!(i64)),
            ("identity".into(), syn::parse_quote!(usize)),
        ]))
        .unwrap();
}

#[test]
fn fold_payloads_require_fresh_names_input_variables_and_correct_types() {
    let input = BTreeMap::from([("x".into(), syn::parse_quote!(i32))]);
    for text in [
        "group r by [missing] { n: usize = count(); }",
        "group r by [x,x] { n: usize = count(); }",
        "group r by [] { x: usize = count(); }",
        "group r by [] { n: i32 = count(); }",
        "group r by [] { n: usize = count(x); }",
        "group r by [] { n: i32 = sum(); }",
        "group r by [] { n: i32 = sum(y); }",
        "group r by [] { n: i32 = sum(*x + 1); }",
        "group r by [] { n: i64 = sum(x); }",
        "group r by [] { n: i32 = min(x); }",
        "group r by [] { n: Option<i64> = max(x); }",
        "group r by [] {}",
    ] {
        let group: Group = syn::parse_str(text).unwrap();
        assert!(group.check(&input).is_err(), "{text}");
    }
}
