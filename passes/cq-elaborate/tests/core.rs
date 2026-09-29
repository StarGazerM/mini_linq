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
fn elaborates_with_only_the_vo_schema_and_clauses() {
    let source: vo::Section = syn::parse_quote!(valuation { R@0(x); if x > 0; });
    let env = Source {
        head: syn::parse_quote!(answer(x)),
    };
    let target = cq_elaborate::compile(&source, &env).unwrap();
    assert_eq!(
        target,
        syn::parse_quote!(relational {
            r0 = rename R { a -> x }; r1 = select r0 if (x > 0); r2 = project r1 keep {r#x}; output r2 as answer(x);
        })
    );
    assert!(cq_elaborate::check(&source, &env, &target));
}
