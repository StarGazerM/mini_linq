use cq_ir::cq::{self, BodyItem};
use quote::ToTokens;

#[test]
fn all_rust_expression_positions_parse_and_roundtrip() {
    for body in [
        "R(42)",
        "R(_)",
        "R(x.field)",
        "R(f::<A, B>(x, y))",
        "R((|a, b| a + b)(x, y))",
        "R({ let x = y; x })",
        "R(x), if x.field",
        "R(x), if *x > 0",
        "R(x), if check!(x)",
        "R(x), if match x { Some(y) => accept(y), _ => false }",
        "R(x), if let Some(y) = x",
        "R(x), let f = |a, b| a + b",
        "R(x), let (a, b): (i32, i32) = pair::<i32, i32>(x)",
        "R(x), !R(transform(x))",
        "R(x), agg total = sum(value) in R(value)",
        "R(x), agg (lo, hi) = stats::<i32>(value) in R(value)",
        "R(x), agg total = (factory(config, |a, b| a + b))(value) in R(transform(x, value))",
        "R(x), agg total = (aggregators[index])(left, right) in R((left, right))",
        "r#agg(x)",
    ] {
        let text = format!("struct P; relation R(a:i32); answer(f(x)) :- {body};");
        let parsed: cq::Module = syn::parse_str(&text).unwrap_or_else(|e| panic!("{body}: {e}"));
        assert_eq!(
            syn::parse2::<cq::Module>(parsed.to_token_stream()).unwrap(),
            parsed
        );
    }
}

#[test]
fn clauses_expose_typed_rust_leaves_to_the_supplied_helper() {
    let source: cq::Module = syn::parse_quote! {
        struct P; relation R(a:i32);
        answer(x) :- R(transform(x)),
            if { let local = limit; local > x },
            let y = build(x),
            agg total = (factory(config))(value) in R(combine(x, value));
    };
    let BodyItem::Positive { atom } = &source.program.query.body[0] else {
        panic!()
    };
    let BodyItem::Filter(filter) = &source.program.query.body[1] else {
        panic!()
    };
    let BodyItem::Let(binding) = &source.program.query.body[2] else {
        panic!()
    };
    let BodyItem::Aggregate(aggregate) = &source.program.query.body[3] else {
        panic!()
    };
    let cq::Aggregator::Expression { expression, .. } = &aggregate.aggregator else {
        panic!()
    };
    for (expr, expected) in [
        (&atom.args[0], vec!["transform", "x"]),
        (&filter.condition, vec!["limit", "x"]),
        (&binding.expression, vec!["build", "x"]),
        (expression, vec!["config", "factory"]),
        (&aggregate.atom.args[0], vec!["combine", "value", "x"]),
    ] {
        assert_eq!(
            cq::rust::free_variables(expr)
                .names
                .into_iter()
                .collect::<Vec<_>>(),
            expected
        );
    }
    assert_eq!(aggregate.arguments[0], "value");
    assert_eq!(cq::rust::bindings(&aggregate.pattern)[0], "total");
}

#[test]
fn malformed_present_syntax_is_rejected() {
    for body in [
        "R(x), let y: = 1",
        "R(x), let y =",
        "R(x), if",
        "R(f::<A, B>(x,)) junk",
        "R(x), agg y = sum(value + 1) in R(value)",
        "R(x), agg y = (factory(config) junk)(value) in R(value)",
        "R(x), agg y = sum(value) R(value)",
    ] {
        let text = format!("struct P; relation R(a:i32); answer(x) :- {body};");
        assert!(syn::parse_str::<cq::Module>(&text).is_err(), "{body}");
    }
}

#[test]
fn parsing_extensions_does_not_bypass_the_semantic_contracts() {
    for body in [
        "R(f(x)), R(x)",
        "if accept(x), R(x)",
        "let y = x, R(x)",
        "!R(x)",
        "agg y = sum(x) in R(x)",
    ] {
        let text = format!("struct P; relation R(a:i32); answer(x) :- {body};");
        let source: cq::Module = syn::parse_str(&text).unwrap();
        assert!(cq::contract::check(&source).is_err(), "{body}");
    }
}
