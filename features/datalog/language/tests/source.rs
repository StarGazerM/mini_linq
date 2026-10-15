use cq_lang_datalog::{Program, contract};

#[test]
fn declaration_rule_and_output_obligations_are_checked() {
    for source in [
        "struct P; relation E(x:i32) = (e); relation A(x:i32); output A;",
        "struct P; relation E() = (e); relation A(x:i32); output A; A(1) :- E();",
        "struct P; relation E(x:i32,x:i32) = (e); relation A(x:i32); output A; A(*x) :- E(x,y);",
        "struct P; relation E(x:i32) = (e); relation E(x:i32); output E; E(*x) :- E(x);",
        "struct P; relation E(x:i32) = (e); relation A(x:i32); output Missing; A(*x) :- E(x);",
        "struct P; relation E(x:i32) = (e); relation A(x:i32); output E; A(*x) :- E(x);",
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
        let source = format!("struct P; relation E(x:i32) = (e); relation A(x:i32); output A; A(*x) :- {body};");
        let program: Program = syn::parse_str(&source).unwrap();
        let error = contract::check(&program).unwrap_err().to_string();
        assert!(error.contains("aggregates and nested blocks"), "{error}");
    }
}

#[test]
fn recursive_heads_copy_positive_columns_even_in_seed_rules() {
    for head in ["*x + 1", "x", "42", "(*owned).clone()", "(*x).clone(1)"] {
        let source = format!("struct P; relation E(x:i32) = (e); relation A(x:i32); output A; A({head}) :- E(x), let owned = x; A(*x) :- A(x);");
        let program: Program = syn::parse_str(&source).unwrap();
        assert!(contract::check(&program).is_err(), "{source}");
    }
    for head in ["*x", "(*x).clone()", "((*x)).clone()"] {
        let source = format!("struct P; relation E(x:String) = (e); relation A(x:String); output A; A({head}) :- E(x); A({head}) :- A(x);");
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
        "E(x), !E(y)", "!E(x), E(x)", "E(x), !E(*x)",
        "E(x), !E(1)", "E(x), !E(x,y)", "E(x), !Missing(x)",
        "E(x), let y = x, !E(y)",
    ] {
        let source = format!("struct P; relation E(x:i32) = (e); relation A(x:i32); output A; A(*x) :- {body};");
        let program: Program = syn::parse_str(&source).unwrap();
        assert!(contract::check(&program).is_err(), "{source}");
    }
}
