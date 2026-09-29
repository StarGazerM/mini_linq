use cq_lang_physical::{IxEnv, LogicalOp, Section, contract};
use std::collections::{BTreeMap, BTreeSet};
struct Dialect;
impl IxEnv for Dialect {
    fn results(&self) -> Vec<(String, LogicalOp)> {
        vec![("u".into(), LogicalOp::Unit)]
    }
    fn heading(&self, result: &str) -> Option<&[String]> {
        (result == "u").then_some(&[])
    }
    fn output(&self) -> String {
        "u".into()
    }
    fn demands(&self) -> BTreeSet<(String, Vec<usize>)> {
        BTreeSet::new()
    }
    fn host_of(&self, _: &str) -> Option<(String, BTreeMap<String, String>)> {
        None
    }
    fn select_condition(&self, _: &str) -> Option<&syn::Expr> {
        None
    }
}
#[test]
fn checks_without_an_ix_or_ra_type() {
    let section: Section = syn::parse_quote!(physical { u = pipe unit; return u; });
    contract::check(&section, &Dialect).unwrap();
    let bad: Section = syn::parse_quote!(physical { u = alias unit; return u; });
    assert!(contract::check(&bad, &Dialect).is_err());
}
