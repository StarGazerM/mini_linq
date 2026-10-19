use cq_lang_cq as cq;
use quote::{ToTokens, quote};
use std::collections::BTreeSet;

#[test]
fn rust_expressions_and_patterns_survive_source_round_trip() {
    for body in [
        "R(x), if accept(x)",
        "R(x), if *x > 0 && *x < 10",
        "R(x), if { let x = 1; x > 0 }",
        "R(x), if match x { 0 => false, _ => true }",
        "R(x), if predicate!(x)",
        "R(x), let f = |a, b| a + b, if f(*x, 1) > 0",
        "R(x), let y: Vec<(i32, i32)> = make::<i32, i32>(x), if y.is_empty()",
        "R(x), for y in values(x), if accept(y)",
        "R(x), for (ref y, mut z) in pairs(x), if accept(y)",
        "R(x), for y in values!(x), let z = y",
        "R(x), for y in { let x = 1; values(x) }, if accept(y)",
        "R(x), R(f(x))",
        "R(x), R({ let y = f(x); y })",
        "R(x), R(42)",
        "R(x), R(_)",
    ] {
        let source = format!("struct P; relation R(value:i32); answer(x) :- {body};");
        let first: cq::Program = syn::parse_str(&source).unwrap();
        cq::contract::check(&first).unwrap();
        let second: cq::Program = syn::parse2(first.to_token_stream()).unwrap();
        assert_eq!(first, second, "{body}");
    }
}

#[test]
fn generators_preserve_normal_form_and_opaque_scope() {
    let source: cq::Program = syn::parse_quote! {
        struct P;
        relation R(value:i32);
        relation answer(value:i32);
        answer(y) :- R(x), for y in values!(x), if accept(y);
    };
    cq::contract::check(&source).unwrap();
    let normal = cq::normal::normalize(&source).unwrap();
    cq::normal::check(&normal).unwrap();
    assert_eq!(normal, source);
    let cq::BodyItem::For(generator) = &source.query.body[1] else {
        panic!("generator must remain structurally distinct");
    };
    assert!(generator.binding().annotation.is_none());
    let step = source.query.body[1].rust_step().unwrap();
    assert_eq!(step.bindings[0], "y");
    let environment = BTreeSet::from(["x".to_owned()]);
    assert_eq!(step.free_variables.dependencies(&environment), environment);
}

#[test]
fn generators_require_fresh_patterns_and_written_scope() {
    for body in [
        "R(x), for x in values(x)",
        "R(x), for y in values(z), let z = 1",
        "R(x), if accept(y), for y in values(x)",
    ] {
        let source: cq::Program = syn::parse_str(&format!(
            "struct P; relation R(value:i32); answer(x) :- {body};"
        ))
        .unwrap();
        assert!(cq::contract::check(&source).is_err(), "{body}");
    }
}

#[test]
fn generator_blocks_and_multiple_rules_preserve_scope() {
    for body in [
        "R(x), { for y in left(x) } | { for y in right(x) }",
        "R(x), { for y in left(x) } else { for y in right(x) }",
    ] {
        let source: cq::Program = syn::parse_str(&format!(
            "struct P; relation R(value:i32); relation answer(value:i32); answer(y) :- {body};"
        ))
        .unwrap();
        cq::contract::check(&source).unwrap();
        cq::normal::check(&cq::normal::normalize(&source).unwrap()).unwrap();
        assert_eq!(
            syn::parse2::<cq::Program>(source.to_token_stream()).unwrap(),
            source
        );
    }
    let source: cq::Program = syn::parse_quote! {
        struct P;
        relation R(value:i32);
        relation answer(value:i32);
        answer(y) :- R(x), for y in left(x);
        answer(y) :- R(x), for y in right(x);
    };
    cq::contract::check(&source).unwrap();
    assert_eq!(
        cq::blocks::walk(&source.query.body)
            .iter()
            .filter(|item| matches!(item, cq::BodyItem::For(_)))
            .count(),
        2
    );
    let escaped: cq::Program = syn::parse_quote! {
        struct P;
        relation R(value:i32);
        relation answer(value:i32);
        answer(z) :- R(x), { for y in left(x), let z = y } | { let z = x }, if accept(y);
    };
    assert!(cq::contract::check(&escaped).is_err());
}

#[test]
fn arguments_are_expressions_and_result_declaration_is_not_an_input() {
    let source: cq::Program = syn::parse_quote! {
        struct P;
        relation R(a:i32,b:i32);
        relation answer(value:String);
        answer(format!("{x}")) :- R(x, f(x));
    };
    cq::contract::check(&source).unwrap();
    assert_eq!(source.inputs().count(), 1);
    assert!(source.output().is_some());
    assert!(matches!(source.query.head.args[0], syn::Expr::Macro(_)));
    let atom = source.query.body[0].atom().unwrap();
    assert_eq!(atom.variables().count(), 1);
    assert!(matches!(atom.args[1], syn::Expr::Call(_)));
}

#[test]
fn declaration_classification_preserves_source_order_and_rejects_duplicates() {
    let inputs = ["relation R(a:i32);", "relation S(a:i32);"];
    for position in 0..=inputs.len() {
        let mut declarations = inputs.to_vec();
        declarations.insert(position, "relation answer(value:String);");
        let source: cq::Program = syn::parse_str(&format!(
            "struct P; {} answer(format!(\"{{x}}\")) :- R(x), S(x);",
            declarations.join(" ")
        ))
        .unwrap();
        cq::contract::check(&source).unwrap();
        assert_eq!(source.declarations[position].name, "answer");
        assert_eq!(
            source
                .inputs()
                .map(|d| d.name.to_string())
                .collect::<Vec<_>>(),
            ["R", "S"]
        );
        assert_eq!(
            syn::parse2::<cq::Program>(source.to_token_stream()).unwrap(),
            source
        );
        let mut duplicate = source;
        duplicate
            .declarations
            .push(duplicate.declarations[position].clone());
        assert!(cq::contract::check(&duplicate).is_err());
    }
}

#[test]
fn malformed_binding_type_is_rejected() {
    assert!(
        syn::parse_str::<cq::Program>(
            "struct P; relation R(a:i32); answer(x) :- R(x), let y: = 1;"
        )
        .is_err()
    );
}

#[test]
fn lexical_scope_excludes_local_bindings_and_retains_macro_environments() {
    let environment: BTreeSet<_> = ["x", "y", "z", "field"].map(String::from).into();
    for (expr, expected) in [
        (
            quote!({
                let x = y;
                x + z
            }),
            vec!["y", "z"],
        ),
        (quote!((|x| x + y)(z)), vec!["y", "z"]),
        (
            quote!(match x {
                Some(y) if y > z => y,
                _ => z,
            }),
            vec!["x", "z"],
        ),
        (quote!(x.field), vec!["x"]),
        (
            quote!({
                for x in y {
                    let z = x;
                }
                z
            }),
            vec!["y", "z"],
        ),
        (quote!(format!("{x}")), vec!["x", "y", "z", "field"]),
    ] {
        let expr: syn::Expr = syn::parse2(expr).unwrap();
        assert_eq!(
            cq::rust::free_variables(&expr).dependencies(&environment),
            expected.into_iter().map(String::from).collect()
        );
    }
}

#[test]
fn logical_forward_references_and_shadowing_are_errors() {
    for body in [
        "R(f(x)), R(x)",
        "if accept(x), R(x)",
        "let y = f(x), R(x)",
        "R(x), let x = f(x)",
    ] {
        let source: cq::Program = syn::parse_str(&format!(
            "struct P; relation R(value:i32); answer(x) :- {body};"
        ))
        .unwrap();
        assert!(cq::contract::check(&source).is_err(), "{body}");
    }
}

#[test]
fn result_types_are_explicit_at_the_public_rust_boundary() {
    let source: cq::Program = syn::parse_quote! {
        struct P; relation R(value:i32); relation answer(value:String);
        answer(format!("{x}")) :- R(x);
    };
    cq::contract::check(&source).unwrap();
    let mut missing = source.clone();
    missing.declarations.pop();
    assert!(cq::contract::check(&missing).is_err());
    let mut renamed = source.clone();
    renamed.declarations[1].name = syn::parse_quote!(other);
    assert!(cq::contract::check(&renamed).is_err());
    let mut wrong_arity = source;
    wrong_arity.declarations[1]
        .columns
        .push(syn::parse_quote!(other:i32));
    assert!(cq::contract::check(&wrong_arity).is_err());

    let typed: cq::Program = syn::parse_quote! {
        struct P; relation R(value:i32);
        answer(label) :- R(x), let label: String = format!("{x}");
    };
    cq::contract::check(&typed).unwrap();
}
