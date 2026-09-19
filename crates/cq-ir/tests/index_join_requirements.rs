use cq_ir::index_requirements as ix;
use quote::{ToTokens, quote};

// Authored IX, independent of index analysis or a physical implementation.
fn source(indexes: proc_macro2::TokenStream, uses: proc_macro2::TokenStream) -> ix::Module {
    syn::parse_quote! {
        struct Composite;
        relation R(a:i32, b:i32);
        relation S(z:i32, b:i32, a:i32);
        answer(a,b,z) :- R(a,b), S(z,b,a);
        relational {
            r = rename R {a -> a, b -> b};
            s = rename S {z -> z, b -> b, a -> a};
            reduced = semijoin s with r;
            joined = natural_join r with reduced;
            again = semijoin joined with reduced;
            output again as answer(a,b,z);
        }
        indexes { #indexes }
        uses { #uses }
    }
}

fn valid() -> ix::Module {
    source(
        quote! { i0 = @r[0,1]; i1 = @reduced[0,1]; i2 = @reduced[0,1,2]; },
        quote! { reduced.right = i0; joined.right = i1; again.right = i2; },
    )
}

#[test]
fn complete_ix_round_trips_and_retains_exact_use_references() {
    let source = valid();
    ix::contract::check(&source).unwrap();
    assert_eq!(
        syn::parse2::<ix::Module>(source.to_token_stream()).unwrap(),
        source
    );
    assert_eq!(source.indexes[1].name, source.uses[1].index);
    assert_eq!(
        ix::signature(&source.indexes[1].requirement),
        ("reduced".into(), vec![0, 1])
    );
}

#[test]
fn same_exact_descriptor_is_shared_by_multiple_operations() {
    let mut source = source(
        quote! { i0 = @r[0,1]; i1 = @reduced[0,1]; },
        quote! { reduced.right = i0; joined.right = i1; again.right = i1; },
    );
    source.relational_plan.plan.definitions[3].operator =
        syn::parse_quote!(semijoin r with reduced);
    source.relational_plan.plan.definitions[4].operator =
        syn::parse_quote!(natural_join joined with reduced);
    ix::contract::check(&source).unwrap();
    assert_eq!(source.uses[1].index, source.uses[2].index);
}

#[test]
fn invalid_descriptor_or_use_is_rejected_with_a_specific_diagnostic() {
    let cases = [
        (
            quote! { i0 = @r[0,1]; i1 = @reduced[0,1]; i2 = @reduced[0,1,2]; },
            quote! { reduced.right = i0; joined.right = i1; },
            "missing index use `again.right`",
        ),
        (
            quote! { i0 = @r[0,1]; i1 = @reduced[0,1]; i2 = @reduced[0,1,2]; },
            quote! { reduced.right = i0; joined.right = missing; again.right = i2; },
            "undefined index",
        ),
        (
            quote! { i0 = @r[0,1]; i1 = @reduced[0,1]; i2 = @reduced[0,1,2]; },
            quote! { reduced.right = i0; joined.right = i1; again.right = i2; joined.right = i1; },
            "duplicate indexed operand use",
        ),
        (
            quote! { i0 = @r[0,1]; i1 = @reduced[0,1]; i2 = @reduced[0,1,2]; },
            quote! { reduced.right = i0; joined.right = i1; again.right = i2; s.right = i0; },
            "NaturalJoin or Semijoin",
        ),
        (
            quote! { i0 = @r[0,1]; i1 = @reduced[1,2]; i2 = @reduced[0,1,2]; },
            quote! { reduced.right = i0; joined.right = i1; again.right = i2; },
            "exact RHS version @reduced on logical columns [0, 1]",
        ),
        (
            quote! { i0 = @r[0,1]; i1 = @s[0,1]; i2 = @reduced[0,1,2]; },
            quote! { reduced.right = i0; joined.right = i1; again.right = i2; },
            "exact RHS version @reduced",
        ),
        (
            quote! { i0 = @r[0,1]; i1 = @reduced[0,1]; i2 = @reduced[0,1,2]; spare = @s[0]; },
            quote! { reduced.right = i0; joined.right = i1; again.right = i2; },
            "unused index definition",
        ),
        (
            quote! { i0 = @r[0,1]; i1 = @reduced[0,1]; i2 = @reduced[0,1,2]; extra = @reduced[0,1]; },
            quote! { reduced.right = i0; joined.right = i1; again.right = i2; },
            "duplicate index descriptor",
        ),
        (
            quote! { i0 = @r[0,1]; i0 = @reduced[0,1]; i2 = @reduced[0,1,2]; },
            quote! { reduced.right = i0; joined.right = i0; again.right = i2; },
            "duplicate index identity",
        ),
        (
            quote! { i0 = @r[0,1]; i1 = @reduced[1,0]; i2 = @reduced[0,1,2]; },
            quote! { reduced.right = i0; joined.right = i1; again.right = i2; },
            "strictly increasing",
        ),
        (
            quote! { i0 = @r[0,1]; i1 = @reduced[0,3]; i2 = @reduced[0,1,2]; },
            quote! { reduced.right = i0; joined.right = i1; again.right = i2; },
            "exceeds heading arity",
        ),
        (
            quote! { i0 = @R[0,1]; i1 = @reduced[0,1]; i2 = @reduced[0,1,2]; },
            quote! { reduced.right = i0; joined.right = i1; again.right = i2; },
            "exact RA result",
        ),
    ];
    for (indexes, uses, diagnostic) in cases {
        let error = ix::contract::check(&source(indexes, uses))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(diagnostic),
            "expected {diagnostic:?}; got {error:?}"
        );
    }
}

#[test]
fn cartesian_and_nonemptiness_uses_explicitly_share_a_unit_tuple_index() {
    let source: ix::Module = syn::parse_quote! {
        struct Cartesian;
        relation R(a:i32);
        relation S(b:i32);
        answer(a,b) :- R(a), S(b);
        relational {
            r = rename R {a -> a};
            s = rename S {b -> b};
            reduced = semijoin r with s;
            joined = natural_join reduced with s;
            output joined as answer(a,b);
        }
        indexes { i0 = @s[]; }
        uses { reduced.right = i0; joined.right = i0; }
    };
    ix::contract::check(&source).unwrap();
    let mut missing = source;
    missing.indexes.clear();
    missing.uses.clear();
    assert!(
        ix::contract::check(&missing)
            .unwrap_err()
            .to_string()
            .contains("missing index use")
    );
}

#[test]
fn no_legacy_index_overlay_or_operand_spelling_is_accepted() {
    let relational = valid().relational_plan;
    for suffix in [
        quote! { indexes { @r[0,1]; } },
        quote! { indexes { i0 = r[0,1]; } uses {} },
        quote! { indexes {} },
        quote! { indexes {} uses { joined.left = i0; } },
    ] {
        assert!(syn::parse2::<ix::Module>(quote! { #relational #suffix }).is_err());
    }
}

#[test]
fn invalid_relational_sources_report_errors_instead_of_panicking() {
    let mut source = valid();
    source.relational_plan.plan.definitions[2].operator =
        syn::parse_quote!(semijoin s with missing);
    assert!(
        std::panic::catch_unwind(|| ix::contract::check(&source))
            .unwrap()
            .is_err()
    );
}

#[test]
fn each_definition_and_use_requires_its_terminating_semicolon() {
    let valid = valid();
    let mut definitions = valid.clone();
    definitions.indexes.pop_punct();
    let mut uses = valid;
    uses.uses.pop_punct();
    for malformed in [definitions, uses] {
        assert!(
            ix::contract::check(&malformed)
                .unwrap_err()
                .to_string()
                .contains("must end with a semicolon")
        );
    }
}
