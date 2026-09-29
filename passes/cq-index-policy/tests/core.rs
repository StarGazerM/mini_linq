use cq_lang_ra as ra;
struct Source {
    inputs: Vec<ra::syntax::RelationDecl>,
    head: ra::syntax::Atom,
}
impl ra::Env for Source {
    fn inputs(&self) -> &[ra::syntax::RelationDecl] {
        &self.inputs
    }
    fn head(&self) -> &ra::syntax::Atom {
        &self.head
    }
}
#[test]
fn derives_exact_demands_from_a_section_and_custom_schema() {
    let env = Source {
        inputs: vec![syn::parse_quote!(relation R(a:i32);)],
        head: syn::parse_quote!(answer(x)),
    };
    let source = syn::parse_quote!(relational { l = rename R {a -> x}; r = rename R {a -> x}; j = natural_join l with r; output j as answer(x); });
    let target = cq_index_policy::compile(&source, &env).unwrap();
    assert_eq!(target, syn::parse_quote!(indexes { i0 = @r[0]; }));
    let target_env = cq_index_policy::Env::new(&source, &env).unwrap();
    cq_lang_ix::contract::check(&target, &target_env).unwrap();
    assert!(cq_index_policy::check(&source, &env, &target));
    assert!(!cq_index_policy::check(
        &source,
        &env,
        &syn::parse_quote!(indexes {})
    ));
}
