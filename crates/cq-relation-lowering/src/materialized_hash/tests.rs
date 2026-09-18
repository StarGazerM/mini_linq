use super::*;

// Authored boundary input: this component test does not invoke another pass.
fn source() -> cq_ir::index_requirements::Module {
    syn::parse_quote! {
        struct Independent;
        relation R(a:i32,b:i32);
        relation S(a:i32,b:i32);
        answer(x,z) :- R(x,y), S(y,z);
        relational {
            r = rename R {a -> x,b -> y};
            s = rename S {a -> y,b -> z};
            joined = natural_join r with s;
            result = project joined keep {x,z};
            output result as answer(x,z);
        }
        indexes { @s[0]; }
    }
}

#[test]
fn caller_selected_access_is_checked_without_a_frontend_dependency() {
    let source = source();
    let access = syn::parse_quote!(application::selected_access);
    let target = compile(&source, &access).unwrap();
    check(&source, &target, &access).unwrap();
    let other = syn::parse_quote!(application::other_access);
    assert!(check(&source, &target, &other).is_err());
}

#[test]
fn missing_required_index_is_rejected_at_the_component_boundary() {
    let mut source = source();
    source.indexes.clear();
    assert!(compile(&source, &syn::parse_quote!(application::selected_access)).is_err());
}
