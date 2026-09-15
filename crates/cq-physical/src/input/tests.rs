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
        indexes { S[0]; }
    }
}

#[test]
fn caller_selected_access_is_checked_without_a_frontend_dependency() {
    let source = source();
    let access = syn::parse_quote!(application::selected_access);
    let target = compile_input_indexed(&source, &access).unwrap();
    check_input_indexed(&source, &target, &access).unwrap();
    let other = syn::parse_quote!(application::other_access);
    assert!(check_input_indexed(&source, &target, &other).is_err());
}

#[test]
fn missing_required_index_is_rejected_at_the_component_boundary() {
    let mut source = source();
    source.indexes.clear();
    assert!(
        compile_input_indexed(&source, &syn::parse_quote!(application::selected_access)).is_err()
    );
}

#[test]
fn physical_coverage_rejects_a_missing_required_key_but_allows_extras() {
    let missing: index_requirements::Module = syn::parse2(quote! {
        struct MissingRequiredIndex;
        relation Seed(c0: i32);
        relation Edge(c0: i32, c1: i32);
        answer(x, z) :- Seed(x), Edge(z, x);
        relational {
            r0 = rename Seed {c0 -> x};
            r1 = rename Edge {c0 -> z, c1 -> x};
            r2 = natural_join r0 with r1;
            r3 = project r2 keep {x, z};
            output r3 as answer(x, z);
        }
        indexes {}
    })
    .unwrap();
    index_requirements::contract::check(&missing).unwrap();
    assert!(!super::domain::covers_required_indexes(&missing));

    let with_extra: index_requirements::Module = syn::parse2(quote! {
        struct ExtraIndex;
        relation Seed(c0: i32);
        relation Edge(c0: i32, c1: i32);
        answer(x, z) :- Seed(x), Edge(z, x);
        relational {
            r0 = rename Seed {c0 -> x};
            r1 = rename Edge {c0 -> z, c1 -> x};
            r2 = natural_join r0 with r1;
            r3 = project r2 keep {x, z};
            output r3 as answer(x, z);
        }
        indexes { Edge[0]; Edge[1]; }
    })
    .unwrap();
    index_requirements::contract::check(&with_extra).unwrap();
    assert!(super::domain::covers_required_indexes(&with_extra));

    let row_only: index_requirements::Module = syn::parse2(quote! {
        struct RowOnly;
        relation R(c0: i32, c1: i32);
        answer(x) :- R(x, y);
        relational {
            r0 = rename R {c0 -> x, c1 -> y};
            r1 = project r0 keep {x};
            output r1 as answer(x);
        }
        indexes {}
    })
    .unwrap();
    index_requirements::contract::check(&row_only).unwrap();
    assert!(super::domain::covers_required_indexes(&row_only));
}
