use cq_lang_ix as ix;
use quote::{ToTokens, quote};

// Authored IX, independent of index analysis or a physical implementation.
fn source(indexes: proc_macro2::TokenStream) -> ix::Program {
    syn::parse_quote! {
            relational {
    relation R(a:i32, b:i32);
            relation S(z:i32, b:i32, a:i32);

                r = rename R {a -> a, b -> b};
                s = rename S {z -> z, b -> b, a -> a};
                reduced = semijoin s with r;
                joined = natural_join r with reduced;
                again = semijoin joined with reduced;
                output again as answer(a,b,z);
            }
            indexes { #indexes }
        }
}

fn valid() -> ix::Program {
    source(quote! { i0 = @r[0,1]; i1 = @reduced[0,1]; i2 = @reduced[0,1,2]; })
}

#[test]
fn complete_ix_round_trips_and_retains_exact_demands() {
    let source = valid();
    ix::contract::check(&source).unwrap();
    assert_eq!(
        syn::parse2::<ix::Program>(source.to_token_stream()).unwrap(),
        source
    );
    assert_eq!(
        ix::signature(&source.indexes[1].requirement),
        ("reduced".into(), vec![0, 1])
    );
}

#[test]
fn same_exact_descriptor_is_shared_by_multiple_operations() {
    let mut source = source(quote! { i0 = @r[0,1]; i1 = @reduced[0,1]; });
    source.ra.plan.definitions[3].operator = syn::parse_quote!(semijoin r with reduced);
    source.ra.plan.definitions[4].operator = syn::parse_quote!(natural_join joined with reduced);
    ix::contract::check(&source).unwrap();
}

#[test]
fn invalid_descriptor_is_rejected_with_a_specific_diagnostic() {
    let cases = [
        (quote! { i0 = @reduced[1,2]; }, "i0"),
        (quote! { i0 = @s[0,1]; }, "i0"),
        (quote! { spare = @s[0]; }, "spare"),
        (
            quote! { i0 = @reduced[0,1]; extra = @reduced[0,1]; },
            "reduced",
        ),
        (quote! { i0 = @r[0,1]; i0 = @reduced[0,1]; }, "i0"),
        (quote! { i0 = @reduced[1,0]; }, "reduced"),
        (quote! { i0 = @reduced[0,0]; }, "reduced"),
        (quote! { i0 = @reduced[0,3]; }, "reduced"),
        (quote! { i0 = @R[0,1]; }, "R"),
    ];
    for (indexes, diagnostic) in cases {
        let error = ix::contract::check(&source(indexes))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(diagnostic),
            "expected {diagnostic:?}; got {error:?}"
        );
    }
}

#[test]
fn missing_demands_are_left_for_physical_correspondence() {
    for indexes in [quote! {}, quote! { partial = @reduced[0,1]; }] {
        ix::contract::check(&source(indexes)).unwrap();
    }
}

#[test]
fn cartesian_and_nonemptiness_joins_share_a_unit_tuple_index() {
    let source: ix::Program = syn::parse_quote! {
            relational {
    relation R(a:i32);
            relation S(b:i32);

                r = rename R {a -> a};
                s = rename S {b -> b};
                reduced = semijoin r with s;
                joined = natural_join reduced with s;
                output joined as answer(a,b);
            }
            indexes { i0 = @s[]; }
        };
    ix::contract::check(&source).unwrap();
}

#[test]
fn invalid_relational_sources_report_errors_instead_of_panicking() {
    let mut source = valid();
    source.ra.plan.definitions[2].operator = syn::parse_quote!(semijoin s with missing);
    assert!(
        std::panic::catch_unwind(|| ix::contract::check(&source))
            .unwrap()
            .is_err()
    );
}

#[test]
fn each_definition_requires_its_terminating_semicolon() {
    let mut malformed = valid();
    malformed.indexes.pop_punct();
    assert!(
        ix::contract::check(&malformed)
            .unwrap_err()
            .to_string()
            .contains("i2")
    );
}
