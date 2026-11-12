use cq_gpu_datalog_core::{encode, storage::Symbols};
use cq_lang_datalog_round::cq;
use quote::quote;
use std::collections::BTreeMap;

fn symbols(source: &cq::Program) -> Symbols {
    let mut symbols: Symbols = syn::parse_quote!(symbols(IStr => [0] Global));
    symbols.0[0].logical_type_id = source.declarations[0].columns[0].logical_type_id;
    symbols
}

#[test]
fn symbol_holes_use_existing_domain_capture_and_fresh_type_ids() {
    let source: cq::Program = syn::parse_quote! {
        struct Out;
        relation E(a:IStr, n:i32) = (&edges);
        relation Out(a:IStr, n:i32);
        Out(x,n) :- E(x,n), where x == sym("needle");
    };
    let map = BTreeMap::from([("E".into(), vec![0,1]), ("Out".into(), vec![0,1])]);
    let declarations = encode::declarations(&source, &map, &symbols(&source)).unwrap();
    let encoded = encode::compile(&source, quote!(#(#declarations)*)).unwrap();
    assert_eq!(encoded.declarations[0].columns[0].ty, syn::parse_quote!(u32));
    assert_eq!(encoded.declarations[0].columns[1].ty, syn::parse_quote!(i32));
    assert_ne!(encoded.declarations[0].columns[0].logical_type_id, encoded.declarations[0].columns[1].logical_type_id);
    assert!(encoded.declarations[0].columns[0].domain.is_some());
    assert!(cq::normal::normalize(&encoded).is_ok());
    assert_eq!(source.declarations[0].columns[0].ty, syn::parse_quote!(IStr));
}

#[test]
fn compared_columns_from_different_components_are_rejected() {
    let source: cq::Program = syn::parse_quote! {
        struct Out;
        relation E(a:IStr) = (&left);
        relation F(a:IStr) = (&right);
        relation Out(a:IStr);
        Out(x) :- E(x), F(x);
    };
    let map = BTreeMap::from([("E".into(), vec![0]), ("F".into(), vec![1]), ("Out".into(), vec![0])]);
    let declarations = encode::declarations(&source, &map, &symbols(&source)).unwrap();
    assert!(encode::compile(&source, quote!(#(#declarations)*)).is_err());
}

fn adapt(source: &cq::Program) -> cq::Program {
    let map = BTreeMap::from([("E".into(), vec![0]), ("Out".into(), vec![0])]);
    let declarations = encode::declarations(source, &map, &symbols(source)).unwrap();
    encode::compile(source, quote!(#(#declarations)*)).unwrap()
}

fn assert_restored(actual: &syn::Expr, original: &syn::Expr, domain: &syn::Expr) {
    let expected: syn::Expr = syn::parse_quote!({
        let x = &(#domain).decode(*x).expect("invalid symbol code at Rust boundary");
        #original
    });
    assert_eq!(actual, &expected);
    let syn::Expr::Block(block) = actual else { panic!("missing inverse bindings") };
    let Some(syn::Stmt::Expr(preserved, None)) = block.block.stmts.last() else {
        panic!("missing original expression")
    };
    assert_eq!(preserved, original);
    assert_eq!(quote!(#preserved).to_string(), quote!(#original).to_string());
}

#[test]
fn inverse_bindings_preserve_closure_and_inner_let_shadowing() {
    for original in [
        syn::parse_quote!((|x| x.len())("local") == x.len()),
        syn::parse_quote!({ let outer = x.len(); let x = "local"; x.len() == outer }),
        syn::parse_quote!(predicate!(x)),
    ] {
        let source: cq::Program = syn::parse_quote! {
            struct Out;
            relation E(a:IStr) = (&edges);
            relation Out(a:IStr);
            Out(x) :- E(x), if #original;
        };
        let encoded = adapt(&source);
        let cq::BodyItem::Filter(filter) = &encoded.query.body[1] else { panic!("filter") };
        let domain = &encoded.declarations[0].columns[0].domain.as_ref().unwrap().expression;
        assert_restored(&filter.condition, &original, domain);
    }
}

#[test]
fn locally_shadowed_names_do_not_decode_unused_rule_variables() {
    let original: syn::Expr = syn::parse_quote!({
        let x = "local";
        (|x: &str| x.len())(x) > 0
    });
    let source: cq::Program = syn::parse_quote! {
        struct Out;
        relation E(a:IStr) = (&edges);
        relation Out(a:IStr);
        Out(x) :- E(x), if #original;
    };
    let encoded = adapt(&source);
    let cq::BodyItem::Filter(filter) = &encoded.query.body[1] else { panic!("filter") };
    assert_eq!(filter.condition, original);
}

#[test]
fn inverse_bindings_cover_let_for_and_encoded_head_results() {
    let original: syn::Expr = syn::parse_quote!({
        let outer = x.clone();
        let x = "local";
        (|x| x)(outer)
    });
    let source: cq::Program = syn::parse_quote! {
        struct Out;
        relation E(a:IStr) = (&edges);
        relation Out(a:IStr);
        Out(#original) :- E(x), let y = #original, for z in #original;
    };
    let encoded = adapt(&source);
    let domain = &encoded.declarations[0].columns[0].domain.as_ref().unwrap().expression;
    let cq::BodyItem::Let(binding) = &encoded.query.body[1] else { panic!("let") };
    assert_restored(&binding.expression, &original, domain);
    let cq::BodyItem::For(binding) = &encoded.query.body[2] else { panic!("for") };
    assert_restored(&binding.expression, &original, domain);
    let restored = &binding.expression;
    let expected_head: syn::Expr = syn::parse_quote!(
        (#domain).encode_required(&(#restored)).expect("head symbol is absent from its frozen result domain")
    );
    assert_eq!(encoded.query.head.args[0], expected_head);

    let source: cq::Program = syn::parse_quote! {
        struct Out;
        relation E(a:IStr) = (&edges);
        relation Out(a:IStr);
        Out(y) :- E(x), let y: IStr = #original;
    };
    let encoded = adapt(&source);
    let cq::BodyItem::Let(binding) = &encoded.query.body[1] else { panic!("let") };
    let expected_value: syn::Expr = syn::parse_quote!(
        (#domain).encode_required(&(#restored)).expect("created symbol is absent from its frozen result domain")
    );
    assert_eq!(binding.expression, expected_value);
    assert_eq!(binding.annotation.as_ref().unwrap().ty, syn::parse_quote!(u32));
}
