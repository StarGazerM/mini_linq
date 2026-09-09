use cq_ir::{cq, relational_plan as ra};
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
        "R(x), R(f(x))",
        "R(x), R({ let y = f(x); y })",
        "R(x), R(42)",
        "R(x), R(_)",
    ] {
        let source = format!("struct P; relation R(value:i32); answer(x) :- {body};");
        let first: cq::Module = syn::parse_str(&source).unwrap();
        cq::contract::check(&first).unwrap();
        let second: cq::Module = syn::parse2(first.to_token_stream()).unwrap();
        assert_eq!(first, second, "{body}");
    }
}

#[test]
fn arguments_are_expressions_and_result_declaration_is_not_an_input() {
    let source: cq::Module = syn::parse_quote! {
        struct P;
        relation R(a:i32,b:i32);
        relation answer(value:String);
        answer(format!("{x}")) :- R(x, f(x));
    };
    cq::contract::check(&source).unwrap();
    assert_eq!(source.program.inputs.len(), 1);
    assert!(source.program.output.is_some());
    assert!(matches!(
        source.program.query.head.args[0],
        syn::Expr::Macro(_)
    ));
    let atom = source.program.query.body[0].atom().unwrap();
    assert_eq!(atom.variables().count(), 1);
    assert!(matches!(atom.args[1], syn::Expr::Call(_)));
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
            cq::rust::uses(&expr, &environment),
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
        let source: cq::Module = syn::parse_str(&format!(
            "struct P; relation R(value:i32); answer(x) :- {body};"
        ))
        .unwrap();
        assert!(cq::contract::check(&source).is_err(), "{body}");
    }
}

#[test]
fn relational_contract_rejects_pruned_expression_dependencies() {
    let source: ra::Module = syn::parse_quote! {
        struct P; relation R(a:i32,b:i32); answer(x) :- R(x,y);
        relational {
            r0 = rename R {a -> x, b -> y};
            r1 = project r0 keep {x};
            r2 = select r1 if (accept(y));
            output r2 as answer(x).
        }
    };
    assert!(ra::contract::check(&source).is_err());
}

#[test]
fn result_types_are_explicit_at_the_public_rust_boundary() {
    let source: cq::Module = syn::parse_quote! {
        struct P; relation R(value:i32); relation answer(value:String);
        answer(format!("{x}")) :- R(x);
    };
    cq::contract::check(&source).unwrap();
    let mut missing = source.clone();
    missing.program.output = None;
    assert!(cq::contract::check(&missing).is_err());
    let mut renamed = source.clone();
    renamed.program.output.as_mut().unwrap().name = syn::parse_quote!(other);
    assert!(cq::contract::check(&renamed).is_err());
    let mut wrong_arity = source;
    wrong_arity
        .program
        .output
        .as_mut()
        .unwrap()
        .columns
        .push(syn::parse_quote!(other:i32));
    assert!(cq::contract::check(&wrong_arity).is_err());

    let typed: cq::Module = syn::parse_quote! {
        struct P; relation R(value:i32);
        answer(label) :- R(x), let label: String = format!("{x}");
    };
    cq::contract::check(&typed).unwrap();
}
