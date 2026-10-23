use cq_lang_cq as cq;
use quote::ToTokens;
use std::collections::BTreeSet;

#[test]
fn negation_preserves_terms_without_binding_or_creating_rust_steps() {
    let query: cq::Program = syn::parse_quote! {
        struct Q;
        relation R(a:i32);
        relation S(a:i32,b:i32,c:f64,d:i32,e:i32);
        answer(x) :- R(x), !S(x,x,_,-3,parameter);
    };
    cq::contract::check(&query).unwrap();
    cq::normal::check(&query).unwrap();
    assert_eq!(cq::normal::normalize(&query).unwrap(), query);
    assert_eq!(
        syn::parse2::<cq::Program>(query.to_token_stream()).unwrap(),
        query
    );
    assert!(!query.query.is_basic());
    let names = cq::blocks::bound(&query.query.body);
    assert_eq!(names.keys().map(String::as_str).collect::<Vec<_>>(), ["x"]);
    let cq::BodyItem::Negation(negation) = &query.query.body[1] else {
        panic!("negation must remain a relational clause");
    };
    assert!(query.query.body[1].rust_step().is_none());
    assert!(query.query.body[1].atom().is_none());
    let logical = names.into_keys().collect();
    assert_eq!(
        negation
            .variables(&logical)
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["x", "x"]
    );
}

#[test]
fn negation_checks_written_scope_inputs_arity_and_term_grammar() {
    for (body, expected) in [
        ("!S(x,_), R(x)", "before it is bound"),
        ("R(x), !S(y,_), let y:i32 = 1", "before it is bound"),
        ("R(x), !Missing(x)", "not declared as an input"),
        ("R(x), !answer(x)", "not declared as an input"),
        ("R(x), !S(x)", "declared arity is 2"),
        ("R(x), !S(x,1,2)", "declared arity is 2"),
        ("R(x), !S(x + 1,_)", "negation arguments"),
        ("R(x), !S(f(x),_)", "negation arguments"),
        ("R(x), !S(*x,_)", "negation arguments"),
        ("R(x), !S(module::parameter,_)", "negation arguments"),
    ] {
        let query: cq::Program = syn::parse_str(&format!(
            "struct Q; relation R(a:i32); relation S(a:i32,b:i32); answer(x) :- {body};"
        ))
        .unwrap();
        let error = cq::contract::check(&query).unwrap_err().to_string();
        assert!(error.contains(expected), "{body}: {error}");
    }
    let fresh: cq::Program = syn::parse_quote! {
        struct Q; relation R(a:i32); relation S(a:i32);
        answer(y) :- R(x), !S(y);
    };
    let error = cq::contract::check(&fresh).unwrap_err().to_string();
    assert!(
        error.contains("not bound"),
        "negation cannot introduce y: {error}"
    );
}

#[test]
fn negation_only_queries_support_existential_emptiness_and_parameters() {
    for argument in ["_", "parameter", "-7"] {
        let query: cq::Program = syn::parse_str(&format!(
            "struct Q; relation R(a:i32); relation answer(a:bool); answer(true) :- !R({argument});"
        ))
        .unwrap();
        cq::contract::check(&query).unwrap();
        assert!(cq::blocks::bound(&query.query.body).is_empty());
        let normal = cq::normal::normalize(&query).unwrap();
        assert!(matches!(normal.query.body[0], cq::BodyItem::Negation(_)));
        assert!(matches!(normal.query.body[1], cq::BodyItem::Let(_)));
        assert_eq!(cq::normal::normalize(&normal).unwrap(), normal);
    }
}

#[test]
fn negation_requires_leibniz_only_for_compared_columns() {
    for argument in ["x", "parameter", "1.0"] {
        let query: cq::Program = syn::parse_str(&format!(
            "struct Q; relation R(a:i32); relation S(a:f64); answer(x) :- R(x), !S({argument});"
        ))
        .unwrap();
        let error = cq::contract::check(&query).unwrap_err().to_string();
        assert!(error.contains("Leibniz"), "{argument}: {error}");
    }
    let wildcard: cq::Program = syn::parse_quote! {
        struct Q; relation R(a:i32); relation S(a:f64,b:f32);
        answer(x) :- R(x), !S(_,_);
    };
    cq::normal::check(&wildcard).unwrap();
    for source in [
        "struct Q; relation R(a:f64,b:i32); relation S(a:i32); answer(y) :- R(x,y), !S(x);",
        "struct Q; relation R(a:i32); relation S(a:i32); answer(y) :- R(y), let x:f64 = 0.0, !S(x);",
    ] {
        let query: cq::Program = syn::parse_str(source).unwrap();
        let error = cq::normal::check(&query).unwrap_err().to_string();
        assert!(error.contains("Leibniz"), "{source}: {error}");
    }
}

#[test]
fn blocks_count_negation_uses_but_not_bindings_or_rust() {
    for operator in ["|", "else"] {
        let query: cq::Program = syn::parse_str(&format!(
            "struct Q; relation R(a:i32); relation S(a:i32,b:i32); answer(x) :- R(x), {{ !S(x,_) }} {operator} {{ !S(x,parameter) }};"
        )).unwrap();
        cq::normal::check(&query).unwrap();
        assert_eq!(cq::normal::normalize(&query).unwrap(), query);
        let cq::BodyItem::Block(block) = &query.query.body[1] else {
            panic!("block")
        };
        let facts = block.analyze(&BTreeSet::from(["x".into()]));
        assert_eq!(
            facts.free.keys().map(String::as_str).collect::<Vec<_>>(),
            ["x"]
        );
        assert!(facts.interface.is_empty());
        assert!(facts.locals.iter().all(BTreeSet::is_empty));
        assert!(!facts.rust);
    }
    for body in [
        "R(x), { R(y) } | { R(z) }, !S(y,_)",
        "R(x), { !S(y,_), R(y) } | { R(y) }",
        "R(x), { { !S(y,_), R(y) } | { R(y) } } | { R(y) }",
    ] {
        let query: cq::Program = syn::parse_str(&format!(
            "struct Q; relation R(a:i32); relation S(a:i32,b:i32); answer(x) :- {body};"
        ))
        .unwrap();
        let error = cq::contract::check(&query).unwrap_err().to_string();
        assert!(error.contains("before it is bound"), "{body}: {error}");
    }
}

#[test]
fn multi_rule_alpha_renaming_preserves_negation_correlations_and_captures() {
    let query: cq::Program = syn::parse_quote! {
        struct Q; relation R(a:i32,b:i32); relation S(a:i32,b:i32,c:i32);
        answer(x) :- R(x,y), !S(x,y,parameter);
        answer(x) :- R(x,y), !S(y,x,parameter);
    };
    cq::normal::check(&query).unwrap();
    assert_eq!(cq::normal::normalize(&query).unwrap(), query);
    assert_eq!(
        syn::parse2::<cq::Program>(query.to_token_stream()).unwrap(),
        query
    );
    let cq::BodyItem::Block(block) = &query.query.body[0] else {
        panic!("union")
    };
    for (index, branch) in block.branches.iter().enumerate() {
        let cq::BodyItem::Positive { atom } = &branch.body[0] else {
            panic!("atom")
        };
        let cq::BodyItem::Negation(negation) = &branch.body[1] else {
            panic!("negation")
        };
        assert_eq!(negation.atom.args[0], atom.args[index]);
        assert_eq!(negation.atom.args[1], atom.args[1 - index]);
        assert_eq!(negation.atom.args[2], syn::parse_quote!(parameter));
    }
}
