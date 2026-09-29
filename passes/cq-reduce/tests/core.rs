use cq_lang_vo as vo;
struct Source {
    head: vo::syntax::Atom,
}
impl vo::CqEnv for Source {
    fn clauses(&self) -> syn::Result<Vec<vo::Clause>> {
        Ok(vec![
            vo::Clause::Atom(syn::parse_quote!(R(x))),
            vo::Clause::Guard(syn::parse_quote!(if x > 0)),
        ])
    }
}
impl vo::Env for Source {
    fn inputs(&self) -> Vec<vo::syntax::RelationDecl> {
        vec![syn::parse_quote!(relation R(a:i32);)]
    }
    fn head(&self) -> &vo::syntax::Atom {
        &self.head
    }
    fn join_tree(&self) -> Option<Vec<(usize, Option<usize>)>> {
        Some(vec![(0, None)])
    }
}

#[test]
fn reduces_a_custom_producer_and_rejects_double_reduction() {
    let source: vo::Section = syn::parse_quote!(valuation { R@0(x); if x > 0; });
    let env = Source {
        head: syn::parse_quote!(answer(x)),
    };
    let options = cq_reduce::Options {
        guards: cq_reduce::Pushdown::Atoms,
        semijoins: cq_reduce::Reductions::Tree,
    };
    let target = cq_reduce::compile(&source, &env, &options).unwrap();
    assert_eq!(
        target,
        syn::parse_quote!(valuation { reduce R@0 if x > 0; R@0(x); })
    );
    assert!(cq_reduce::check(&source, &env, &target, &options));
    assert!(cq_reduce::compile(&target, &env, &options).is_err());
}
