use cq_lang_cq as cq;
use quote::ToTokens;
#[test]
fn complete_program_preserves_declarations_and_attributes() {
    let source: cq::Program = syn::parse_quote! {
        pub struct Query;
        relation Z(a:i32,b:i32);
        relation A(a:i32);
        answer(x) :- Z(x,y), A(y);
    };
    cq::contract::check(&source).unwrap();
    assert_eq!(
        source
            .inputs()
            .map(|r| r.name.to_string())
            .collect::<Vec<_>>(),
        ["Z", "A"]
    );
    assert_eq!(
        syn::parse2::<cq::Program>(source.to_token_stream()).unwrap(),
        source
    );
}
#[test]
fn declaration_arity_scope_and_output_violations_are_diagnostic() {
    for (text, construct) in [
        ("struct Q; relation R(); answer(x) :- R(x);", "R"),
        (
            "struct Q; relation R(a:i32); relation R(a:i32); answer(x) :- R(x);",
            "R",
        ),
        (
            "struct Q; relation R(a:i32,a:i32); answer(x) :- R(x,y);",
            "a",
        ),
        (
            "struct Q; relation R(a:i32); answer(x) :- Missing(x);",
            "Missing",
        ),
        ("struct Q; relation R(a:i32,b:i32); answer(x) :- R(x);", "R"),
        ("struct Q; relation R(a:i32); answer(y) :- R(x);", "y"),
        (
            "struct Q; relation R(a:i32); answer(x) :- if x > 0, R(x);",
            "x",
        ),
        (
            "struct Q; relation R(a:i32); answer(x) :- R(x), let x = 1;",
            "x",
        ),
    ] {
        let source = syn::parse_str(text).unwrap();
        let error = cq::contract::check(&source).unwrap_err().to_string();
        assert!(error.contains(construct), "{construct}: {error}");
    }
}

#[test]
fn raw_identifiers_preserve_names_but_do_not_bypass_relation_collisions() {
    let source: cq::Program = syn::parse_quote! {
        struct Q;
        relation R(a: i32);
        r#relation(x) :- R(x);
    };
    cq::contract::check(&source).unwrap();
    assert_eq!(
        syn::parse2::<cq::Program>(source.to_token_stream()).unwrap(),
        source
    );
    for text in [
        "struct Q; relation R(a: i32); relation r#R(a: i32); answer(x) :- R(x);",
        "struct Q; relation R(a: i32); r#R(x) :- R(x);",
    ] {
        let error = cq::contract::check(&syn::parse_str(text).unwrap())
            .unwrap_err()
            .to_string();
        assert!(error.contains('R'), "{error}");
    }
}

#[test]
fn column_types_preserve_lifetimes_and_nested_generic_arguments() {
    let source: cq::Program = syn::parse_quote! {
        struct Q;
        relation R(c0: &'static str, c1: Option<Vec<i32>>);
        answer(x, y) :- R(x, y);
    };
    cq::contract::check(&source).unwrap();
    let reparsed: cq::Program = syn::parse2(source.to_token_stream()).unwrap();
    assert_eq!(
        reparsed.declarations[0].columns[0].ty,
        syn::parse_quote!(&'static str)
    );
    assert_eq!(
        reparsed.declarations[0].columns[1].ty,
        syn::parse_quote!(Option<Vec<i32>>)
    );
    assert_eq!(reparsed, source);
}

#[test]
fn declarations_keep_leaf_holes_and_reject_attributes() {
    let source: cq::Program = syn::parse_quote! { struct Q; relation R(a:i32,b:&str) = (make_rows(config)); answer(x,y) :- R(x,y); };
    cq::contract::check(&source).unwrap();
    assert_eq!(
        syn::parse2::<cq::Program>(source.to_token_stream()).unwrap(),
        source
    );
    for attr in ["host(canon)", "index(T)", "other"] {
        let text = format!("struct Q; #[{attr}] relation R(a:i32); answer(x) :- R(x);");
        assert!(syn::parse_str::<cq::Program>(&text).is_err());
    }
    for value in ["rows", "()"] {
        assert!(
            syn::parse_str::<cq::Program>(&format!(
                "struct Q; relation R(a:i32) = {value}; answer(x) :- R(x);"
            ))
            .is_err()
        );
    }
}

#[test]
fn extension_clauses_preserve_opaque_tokens_and_cannot_enter_the_core() {
    let mut source: cq::Program = syn::parse_quote! {
        struct Q; relation R(a:i32);
        answer(x) :- R(x), neg! { S(x) strange => [ nested!{a + b} ] };
    };
    let cq::BodyItem::Extension { name, tokens } = &source.query.body[1] else {
        panic!("extension")
    };
    assert_eq!(name, "neg");
    assert_eq!(
        tokens.to_string(),
        quote::quote!(S(x) strange => [nested!{a + b}]).to_string()
    );
    assert_eq!(
        syn::parse2::<cq::Program>(source.to_token_stream()).unwrap(),
        source
    );
    assert_eq!(
        cq::contract::check(&source).unwrap_err().to_string(),
        "extension `neg!` was not expanded"
    );
    let stacked: cq::Program = syn::parse_str(
        &source
            .to_token_stream()
            .to_string()
            .replace("neg !", "agg !"),
    )
    .unwrap();
    assert_eq!(
        cq::contract::check(&stacked).unwrap_err().to_string(),
        "extension `agg!` was not expanded"
    );
    source.query.body.pop();
    cq::contract::check(&source).unwrap();
}
