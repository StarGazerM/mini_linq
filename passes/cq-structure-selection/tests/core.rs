use cq_lang_physical::{self as p, LogicalOp};
use std::collections::{BTreeMap, BTreeSet};
struct Source {
    heading: Vec<String>,
    head: p::syntax::Atom,
}
impl p::IxEnv for Source {
    fn results(&self) -> Vec<(String, LogicalOp)> {
        vec![
            ("r".into(), LogicalOp::Rename),
            (
                "j".into(),
                LogicalOp::Join {
                    left: "r".into(),
                    right: "r".into(),
                },
            ),
        ]
    }
    fn heading(&self, _: &str) -> Option<&[String]> {
        Some(&self.heading)
    }
    fn output(&self) -> String {
        "j".into()
    }
    fn demands(&self) -> BTreeSet<(String, Vec<usize>)> {
        [("r".into(), vec![0])].into()
    }
    fn host_of(&self, result: &str) -> Option<(String, BTreeMap<String, String>)> {
        (result == "r").then(|| ("R".into(), [("x".into(), "a".into())].into()))
    }
    fn select_condition(&self, _: &str) -> Option<&syn::Expr> {
        None
    }
}
impl p::Env for Source {
    fn inputs(&self) -> Vec<p::syntax::RelationDecl> {
        vec![syn::parse_quote!(#[index(std::collections::BTreeMap<_, Vec<_>>)] relation R(a:i32);)]
    }
    fn head(&self) -> &p::syntax::Atom {
        &self.head
    }
    fn bindings(&self, _: &str) -> BTreeMap<String, syn::Ident> {
        [("x".into(), syn::parse_quote!(x))].into()
    }
    fn computed(&self, _: &str) -> BTreeSet<String> {
        BTreeSet::new()
    }
    fn extension(&self, _: &str) -> Option<(Vec<p::syntax::ColumnDecl>, syn::Expr)> {
        None
    }
}
#[test]
fn selects_advised_storage_with_no_reference_prefix() {
    let env = Source {
        heading: vec!["x".into()],
        head: syn::parse_quote!(answer(x)),
    };
    let source = syn::parse_quote!(physical { r = let rename; j = pipe probe[x] r, r; return j; });
    let policy = syn::parse_quote!(policy { R canon; });
    let target = cq_structure_selection::compile(&source, &env, &policy).unwrap();
    assert_eq!(
        target,
        syn::parse_quote!(structures { R canon; r = std::collections::BTreeMap<_, Vec<_>> [x]; })
    );
    assert!(cq_structure_selection::check(
        &source, &env, &target, &policy
    ));
    assert!(cq_structure_selection::compile(&source, &env, &syn::parse_quote!(policy {})).is_err());
}
