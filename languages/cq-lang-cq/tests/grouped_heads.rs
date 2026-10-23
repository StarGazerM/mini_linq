use cq_lang_cq as cq;
use quote::ToTokens;

#[test]
fn grouped_heads_capture_named_valuation_not_wildcard_witnesses() {
    let source: cq::Program = syn::parse_quote! {
        struct Grouped;
        relation R(k: i32, id: i32, price: i64, hidden: i32);
        relation answer(k: i32, n: usize, total: i64, lo: Option<i64>, hi: Option<i64>);
        answer(k, count(), sum(price), min(price), max(price)) :- R(k, id, price, _);
    };
    cq::contract::check(&source).unwrap();
    let normal = cq::normal::normalize(&source).unwrap();
    let group = normal.query.group.as_ref().unwrap();
    assert_eq!(
        group
            .valuation
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["id", "k", "price"]
    );
    assert_eq!(
        group
            .keys
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["k"]
    );
    assert_eq!(group.aggregates.len(), 4);
    assert_eq!(normal.query.head.variables().count(), 5);
    assert!(
        normal
            .query
            .body
            .iter()
            .all(|item| !matches!(item, cq::BodyItem::Let(_)))
    );
    assert_eq!(cq::normal::normalize(&normal).unwrap(), normal);
    assert_eq!(
        syn::parse2::<cq::Program>(normal.to_token_stream()).unwrap(),
        normal
    );
}

#[test]
fn aggregate_expression_bindings_follow_the_written_rust_chain() {
    let source: cq::Program = syn::parse_quote! {
        struct Expressions;
        relation R(id: i32, x: i64);
        relation answer(n: usize, a: i64, b: Option<i64>);
        answer(count(), sum(first(x)), min(second(x))) :- R(id, x), if guard(x);
    };
    let normal = cq::normal::normalize(&source).unwrap();
    let group = normal.query.group.as_ref().unwrap();
    assert!(group.keys.is_empty());
    assert_eq!(group.valuation.len(), 4);
    assert!(matches!(normal.query.body[1], cq::BodyItem::Filter(_)));
    let cq::BodyItem::Let(first) = &normal.query.body[2] else {
        panic!("first argument binding")
    };
    let cq::BodyItem::Let(second) = &normal.query.body[3] else {
        panic!("second argument binding")
    };
    assert_eq!(first.expression.to_token_stream().to_string(), "first (x)");
    assert_eq!(
        second.expression.to_token_stream().to_string(),
        "second (x)"
    );
    assert!(
        group
            .aggregates
            .iter()
            .filter_map(|a| a.expression.as_ref())
            .all(|e| cq::rust::variable(e).is_some())
    );
}

#[test]
fn declared_types_and_aggregate_arity_are_required() {
    for text in [
        "struct P; relation R(x:i32); answer(count()) :- R(x);",
        "struct P; relation R(x:i32); relation answer(n:usize); answer(count(x)) :- R(x);",
        "struct P; relation R(x:i32); relation answer(n:i32); answer(sum()) :- R(x);",
        "struct P; relation R(x:i32); relation answer(n:i32); answer(count()) :- R(x);",
        "struct P; relation R(x:i32); relation answer(n:i32); answer(min(x)) :- R(x);",
        "struct P; relation R(x:i32); relation answer(k:i32,n:usize); answer(*x+1,count()) :- R(x);",
    ] {
        let source: cq::Program = syn::parse_str(text).unwrap();
        assert!(cq::normal::normalize(&source).is_err(), "{text}");
    }
}

#[test]
fn grouped_multi_rule_heads_have_an_explicit_scope_diagnostic() {
    let error = syn::parse_str::<cq::Program>(
        "struct P; relation R(x:i32); relation answer(n:usize); answer(count()) :- R(x); answer(count()) :- R(x);"
    ).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("grouped heads require a single rule")
    );
}

#[test]
fn plain_heads_are_unchanged_and_group_keys_keep_head_order() {
    let plain: cq::Program = syn::parse_quote! { struct P; relation R(x:i32); answer(x) :- R(x); };
    assert_eq!(cq::normal::normalize(&plain).unwrap(), plain);
    let grouped: cq::Program = syn::parse_quote! {
        struct P; relation R(x:i32,y:i32);
        relation answer(y:i32,x:i32,n:usize);
        answer(y,x,count()) :- R(x,y);
    };
    let normal = cq::normal::normalize(&grouped).unwrap();
    assert_eq!(
        normal
            .query
            .group
            .unwrap()
            .keys
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["y", "x"]
    );
}
