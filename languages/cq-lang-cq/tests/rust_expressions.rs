use cq_lang_cq as cq;
use cq_lang_ra as ra;
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
    assert_eq!(source.program.inputs().count(), 1);
    assert!(source.program.output().is_some());
    assert!(matches!(
        source.program.query.head.args[0],
        syn::Expr::Macro(_)
    ));
    let atom = source.program.query.body[0].atom().unwrap();
    assert_eq!(atom.variables().count(), 1);
    assert!(matches!(atom.args[1], syn::Expr::Call(_)));
}

#[test]
fn declaration_classification_preserves_source_order_and_rejects_duplicates() {
    let inputs = ["relation R(a:i32);", "relation S(a:i32);"];
    for position in 0..=inputs.len() {
        let mut declarations = inputs.to_vec();
        declarations.insert(position, "relation answer(value:String);");
        let source: cq::Module = syn::parse_str(&format!(
            "struct P; {} answer(format!(\"{{x}}\")) :- R(x), S(x);",
            declarations.join(" ")
        ))
        .unwrap();
        cq::contract::check(&source).unwrap();
        assert_eq!(source.program.declarations[position].name, "answer");
        assert_eq!(
            source
                .program
                .inputs()
                .map(|d| d.name.to_string())
                .collect::<Vec<_>>(),
            ["R", "S"]
        );
        assert_eq!(
            syn::parse2::<cq::Module>(source.to_token_stream()).unwrap(),
            source
        );
        let mut duplicate = source;
        duplicate
            .program
            .declarations
            .push(duplicate.program.declarations[position].clone());
        assert!(cq::contract::check(&duplicate).is_err());
    }
}

#[test]
fn final_rust_expressions_require_semicolons_and_malformed_annotations_fail() {
    for body in [
        "R(x), if x.field",
        "R(x), if *x > 0",
        "R(x), let y: i32 = x.method()",
    ] {
        let prefix = format!("struct P; relation R(a:i32); answer(x) :- {body}");
        syn::parse_str::<cq::Module>(&format!("{prefix};")).unwrap();
        assert!(syn::parse_str::<cq::Module>(&format!("{prefix}.")).is_err());
    }
    assert!(
        syn::parse_str::<cq::Module>("struct P; relation R(a:i32); answer(x) :- R(x), let y: = 1;")
            .is_err()
    );
}

#[test]
fn comparison_recognition_respects_the_total_i32_fragment() {
    for (text, integer) in [
        ("x >= -2147483648", i32::MIN),
        ("(x) < 0x7fff_ffffi32", i32::MAX),
    ] {
        let expr: syn::Expr = syn::parse_str(text).unwrap();
        let predicate = cq::Predicate::from_expr(&expr).unwrap();
        assert_eq!(predicate.right, cq::Term::Integer(integer));
    }
    for text in [
        "x < 2147483648",
        "x == 1u32",
        "x == 1.0",
        "x + 1 == 2",
        "f(x) == 0",
        "#[cfg(any())] (x == 0)",
    ] {
        let expr: syn::Expr = syn::parse_str(text).unwrap();
        assert!(cq::Predicate::from_expr(&expr).is_none(), "{text}");
    }
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
            output r2 as answer(x);
        }
    };
    assert!(ra::contract::check_module(&source).is_err());
}

#[test]
fn result_types_are_explicit_at_the_public_rust_boundary() {
    let source: cq::Module = syn::parse_quote! {
        struct P; relation R(value:i32); relation answer(value:String);
        answer(format!("{x}")) :- R(x);
    };
    cq::contract::check(&source).unwrap();
    let mut missing = source.clone();
    missing.program.declarations.pop();
    assert!(cq::contract::check(&missing).is_err());
    let mut renamed = source.clone();
    renamed.program.declarations[1].name = syn::parse_quote!(other);
    assert!(cq::contract::check(&renamed).is_err());
    let mut wrong_arity = source;
    wrong_arity.program.declarations[1]
        .columns
        .push(syn::parse_quote!(other:i32));
    assert!(cq::contract::check(&wrong_arity).is_err());

    let typed: cq::Module = syn::parse_quote! {
        struct P; relation R(value:i32);
        answer(label) :- R(x), let label: String = format!("{x}");
    };
    cq::contract::check(&typed).unwrap();
}
