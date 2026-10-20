use cq_lang_datalog_source::{Program, contract};

#[test]
fn declaration_rule_and_output_obligations_are_checked() {
    for source in [
        "struct P; relation E(x:i32) = (e); relation A(x:i32); output A;",
        "struct P; relation E() = (e); relation A(x:i32); output A; A(1) :- E();",
        "struct P; relation E(x:i32,x:i32) = (e); relation A(x:i32); output A; A(*x) :- E(x,y);",
        "struct P; relation E(x:i32) = (e); relation E(x:i32); output E; E(*x) :- E(x);",
        "struct P; relation E(x:i32) = (e); relation A(x:i32); output Missing; A(*x) :- E(x);",
        "struct P; relation E(x:i32) = (e); relation A(x:i32); output A; Missing(*x) :- E(x);",
        "struct P; relation E(x:i32) = (e); relation A(x:i32); output A; A(*x) :- Missing(x);",
        "struct P; relation E(x:i32) = (e); relation A(x:i32); output A; A(*x) :- E(x,y);",
        "struct P; relation E(x:i32) = (e); relation A(x:i32); output A; A(*x,*x) :- E(x);",
    ] {
        let program: Program = syn::parse_str(source).unwrap();
        assert!(contract::check(&program).is_err(), "{source}");
    }
}

#[test]
fn aggregates_and_nested_blocks_are_rejected_at_the_source_boundary() {
    for body in [
        "E(x), agg n = count(y) in E(y)",
        "{ E(x) } | { E(x) }",
        "{ E(x) } else { E(x) }",
        "E(x), custom! { arbitrary text }",
    ] {
        let source = format!(
            "struct P; relation E(x:i32) = (e); relation A(x:i32); output A; A(*x) :- {body};"
        );
        let program: Program = syn::parse_str(&source).unwrap();
        let error = contract::check(&program).unwrap_err().to_string();
        assert!(error.contains("aggregates and nested blocks"), "{error}");
    }
}

#[test]
fn recursive_heads_accept_computations_and_let_bindings_in_seeds_and_steps() {
    for head in ["*x + 1", "42", "owned", "*x"] {
        let source = format!(
            "struct P; relation E(x:i32) = (e); relation A(x:i32); output A; A({head}) :- E(x), let owned = *x + 1; A({head}) :- A(x), if *x < 10, let owned = *x + 1;"
        );
        let program: Program = syn::parse_str(&source).unwrap();
        contract::check(&program).unwrap();
    }
    for head in ["*x", "(*x).clone()", "((*x)).clone()"] {
        let source = format!(
            "struct P; relation E(x:String) = (e); relation A(x:String); output A; A({head}) :- E(x); A({head}) :- A(x);"
        );
        let program: Program = syn::parse_str(&source).unwrap();
        contract::check(&program).unwrap();
    }
}

#[test]
fn computed_acyclic_heads_and_safe_negative_wildcards_remain_supported() {
    let program: Program = syn::parse_quote! {
        struct P;
        relation E(x:i32,y:i32) = (e);
        relation Blocked(x:i32,y:i32) = (blocked);
        relation A(x:i32);
        output A;
        A(*x + *y) :- E(x,y), !Blocked(x,_), !Blocked(_,_);
    };
    contract::check(&program).unwrap();
}

#[test]
fn source_negation_does_not_introduce_bindings_or_accept_computed_keys() {
    for body in [
        "E(x), !E(y)",
        "!E(x), E(x)",
        "E(x), !E(*x)",
        "E(x), !E(x + 1)",
        "E(x), !E(x,y)",
        "E(x), !Missing(x)",
        "E(x), let y = x, !E(y)",
    ] {
        let source = format!(
            "struct P; relation E(x:i32) = (e); relation A(x:i32); output A; A(*x) :- {body};"
        );
        let program: Program = syn::parse_str(&source).unwrap();
        assert!(contract::check(&program).is_err(), "{source}");
    }
}

#[test]
fn ordered_outputs_round_trip_and_reject_invalid_lists() {
    use quote::ToTokens;
    let program: Program = syn::parse_quote! {
        struct P;
        relation E(x:i32) = (e);
        relation A(x:i32); relation B(x:i32);
        output B, A;
        A(*x) :- E(x);
        B(*x + 1) :- E(x);
    };
    contract::check(&program).unwrap();
    let expected: Vec<syn::Ident> = vec![syn::parse_quote!(B), syn::parse_quote!(A)];
    assert_eq!(program.outputs, expected);
    assert_eq!(program, syn::parse2(program.to_token_stream()).unwrap());
    for outputs in [
        vec![],
        vec![syn::parse_quote!(A), syn::parse_quote!(A)],
        vec![syn::parse_quote!(A), syn::parse_quote!(Missing)],
    ] {
        let mut changed = program.clone();
        changed.outputs = outputs;
        assert!(contract::check(&changed).is_err());
    }
    assert!(syn::parse_str::<Program>("struct P; output;").is_err());
    assert!(syn::parse_str::<Program>("struct P; output A,;").is_err());
}

#[test]
fn negative_constants_are_closed_over_the_rule_environment() {
    for constant in [
        "1",
        "-1",
        "symbol(\"abstract\")",
        "(host_constant)",
        "String::from(\"λ\")",
    ] {
        let source = format!(
            "struct P; relation E(x:i32) = (e); relation A(x:i32); output A; A(*x) :- E(x), !E({constant});"
        );
        contract::check(&syn::parse_str::<Program>(&source).unwrap()).unwrap();
    }
    for body in [
        "E(x), !E(symbol(x))",
        "!E(symbol(x)), E(x)",
        "E(x), let y = x, !E(symbol(y))",
        "E(x), !E(hidden!())",
    ] {
        let source = format!(
            "struct P; relation E(x:i32) = (e); relation A(x:i32); output A; A(*x) :- {body};"
        );
        assert!(contract::check(&syn::parse_str::<Program>(&source).unwrap()).is_err());
    }
}

#[test]
fn edb_outputs_are_declared_and_can_stand_alone() {
    use quote::ToTokens;
    for source in [
        "struct P; relation E(x:String) = (e); output E;",
        "struct P; relation E(x:String) = (e); relation F(x:i32) = (f); output F, E;",
        "struct P; relation E(x:String) = (e); relation A(x:String); output E, A; A((*x).clone()) :- E(x);",
        "struct P; relation E(x:String) = (e); relation A(x:String); output A, E; A((*x).clone()) :- E(x);",
    ] {
        let program: Program = syn::parse_str(source).unwrap();
        contract::check(&program).unwrap();
        assert_eq!(
            program,
            syn::parse2::<Program>(program.to_token_stream()).unwrap()
        );
    }
}

