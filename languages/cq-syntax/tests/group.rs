use cq_syntax::algebra::{AggregateKind, Group};
use quote::ToTokens;
use std::collections::{BTreeMap, BTreeSet};

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
        .check(
            &BTreeMap::from([
                ("key".into(), syn::parse_quote!(i32)),
                ("value".into(), syn::parse_quote!(i64)),
                ("identity".into(), syn::parse_quote!(usize)),
            ]),
            &BTreeSet::new(),
        )
        .unwrap();
}

#[test]
fn captures_are_shared_keys_and_arguments_but_not_outputs() {
    let input = BTreeMap::from([("x".into(), syn::parse_quote!(i32))]);
    let captures = BTreeSet::from(["one".to_string()]);
    let group: Group = syn::parse_str("group r by [one] { n: i64 = sum(one); }").unwrap();
    group.check(&input, &captures).unwrap();
    assert!(group.check(&input, &BTreeSet::new()).is_err());
    let shadow: Group = syn::parse_str("group r by [] { one: usize = count(); }").unwrap();
    assert!(shadow.check(&input, &captures).is_err());
}

#[test]
fn fold_payloads_require_fresh_names_and_input_variables() {
    let input = BTreeMap::from([("x".into(), syn::parse_quote!(i32))]);
    for text in [
        "group r by [missing] { n: usize = count(); }",
        "group r by [x,x] { n: usize = count(); }",
        "group r by [] { x: usize = count(); }",
        "group r by [] { n: usize = count(x); }",
        "group r by [] { n: i32 = sum(); }",
        "group r by [] { n: i32 = sum(y); }",
        "group r by [] { n: i32 = sum(*x + 1); }",
        "group r by [] {}",
    ] {
        let group: Group = syn::parse_str(text).unwrap();
        assert!(group.check(&input, &BTreeSet::new()).is_err(), "{text}");
    }
}

#[test]
fn fold_types_are_left_to_rustc() {
    let input = BTreeMap::from([("x".into(), syn::parse_quote!(f32))]);
    for text in [
        "group r by [] { n: CountAlias = count(); }",
        "group r by [] { n: TotalAlias = sum(x); }",
        "group r by [] { n: MinimumAlias = min(x); }",
        "group r by [] { n: MaximumAlias = max(x); }",
    ] {
        syn::parse_str::<Group>(text)
            .unwrap()
            .check(&input, &BTreeSet::new())
            .unwrap();
    }
}
