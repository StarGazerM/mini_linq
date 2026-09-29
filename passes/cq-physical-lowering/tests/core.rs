use cq_lang_ix::{self as ix, Operation};
struct Source {
    heading: Vec<String>,
    head: ix::syntax::Atom,
}
impl ix::RaEnv for Source {
    fn heading(&self, result: &str) -> Option<&[String]> {
        matches!(result, "l" | "r" | "j").then_some(&self.heading)
    }
    fn joins(&self) -> Vec<ix::Join> {
        vec![ix::Join {
            result: "j".into(),
            left: "l".into(),
            right: "r".into(),
            semi: false,
        }]
    }
}
impl ix::Env for Source {
    fn inputs(&self) -> Vec<ix::syntax::RelationDecl> {
        vec![syn::parse_quote!(relation R(a:i32);)]
    }
    fn operations(&self) -> Vec<(syn::Ident, Operation)> {
        let rename = || Operation::Rename {
            host: syn::parse_quote!(R),
            mapping: [(syn::parse_quote!(x), syn::parse_quote!(a))].into(),
        };
        vec![
            (syn::parse_quote!(l), rename()),
            (syn::parse_quote!(r), rename()),
            (
                syn::parse_quote!(j),
                Operation::Join {
                    left: syn::parse_quote!(l),
                    right: syn::parse_quote!(r),
                },
            ),
        ]
    }
    fn returned(&self) -> syn::Ident {
        syn::parse_quote!(j)
    }
    fn head(&self) -> &ix::syntax::Atom {
        &self.head
    }
}
#[test]
fn lowers_custom_logical_operations_and_requires_demands() {
    let env = Source {
        heading: vec!["x".into()],
        head: syn::parse_quote!(answer(x)),
    };
    let source = syn::parse_quote!(indexes { i0 = @r[0]; });
    let policy = syn::parse_quote!(policy { R canon; });
    let target = cq_physical_lowering::compile(&source, &env, &policy).unwrap();
    assert_eq!(
        target,
        syn::parse_quote!(physical { l = alias rename; r = let rename; j = pipe probe[x] l, r; return j; })
    );
    assert!(cq_physical_lowering::check(&source, &env, &target, &policy));
    assert!(cq_physical_lowering::compile(&syn::parse_quote!(indexes {}), &env, &policy).is_err());
}
