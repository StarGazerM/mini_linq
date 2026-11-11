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
