use cq_lang_sequence::{Catalog, structures};
use structures::{ProviderCapability as Support, Structure};

fn source(decision: &str) -> structures::Program {
    let decision: proc_macro2::TokenStream = decision.parse().unwrap();
    let mut program: structures::Program = syn::parse_quote! {
        physical {
            relation L(a:i64); relation R(a:i64,b:i64);
            l = pipe rename L@0 {a -> x};
            r = let rename R@1 {a -> x,b -> y};
            s = pipe semi eager [x] l,r;
            p = let canon probe first [x] s,r;
            return p as answer(x,y);
        }
        structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; original = Original { reads [exact, range, membership], supports [expand, keys], build inferred, join_filter }; sort = ::cq_storage::dedup::Sort { reads [distinct, comparator_rows], build inferred, shared }; filter = Filter { reads [membership], build inferred }; }
            l host scan = @reader as Reader [];
            r host scan = @reader as Reader [];
            r = @original as Original [x] supports [expand, keys];
            p rows = @sort as ::cq_storage::dedup::Sort;
            p canon = rows;
            #decision
        }
    };
    program.structures.declare(&[]).unwrap();
    program
}
fn catalog(supports: Vec<Support>) -> Catalog {
    let supports = supports.iter().map(|support| match support {
        Support::Retain => quote::quote!(retain),
        Support::Expand => quote::quote!(expand),
        Support::Keys => quote::quote!(keys),
    });
    // Read capabilities alone must not license retained/expanded/filter decisions.
    let declared: structures::Providers = syn::parse_quote!(providers {
        read_only = ReadOnly { reads [exact, range, membership], build inferred, join_filter };
        selected_alias = SelectedAlias {
            reads [exact, range, membership], supports [#(#supports),*], build inferred, join_filter
        };
        sort = ::cq_storage::dedup::Sort { reads [distinct, comparator_rows], build inferred, shared };
    });
    let mut catalog = Catalog::default();
    for declaration in declared.declarations {
        catalog.register(declaration, vec![vec!["host".into()]]);
    }
    catalog
}
fn selected_provider(program: &structures::Program) -> &structures::Provider {
    program
        .structures
        .entries
        .iter()
        .find_map(|entry| match &entry.structure {
            Structure::Built { keyed, .. } if entry.name == "r" => Some(&keyed.provider),
            _ => None,
        })
        .unwrap()
}

#[test]
fn catalog_retyping_preserves_live_support_obligations() {
    for (decision, capability) in [
        (
            "s share = lookup r [x] first s force p; s repr = tuples witness { r [x] at s as _ };",
            Support::Retain,
        ),
        ("p stage = batch 4 expand;", Support::Expand),
        (
            "l join_filter = @filter as Filter build r [x] covering s marker ::cq_load::ColumnAt<0>;",
            Support::Keys,
        ),
    ] {
        let source = source(decision);
        structures::contract::check(&source).unwrap();
        let selected =
            cq_sequence_plan::select_from_catalog(&source, &catalog(vec![capability])).unwrap();
        assert_eq!(
            selected_provider(&selected),
            &syn::parse_quote!(@selected_alias as SelectedAlias)
        );
        assert_eq!(selected.physical, source.physical);
        let facts = structures::contract::check(&selected).unwrap();
        assert!(facts.provider_support("r", &["x".into()], None, capability));
        assert!(cq_sequence_plan::select_from_catalog(&source, &catalog(Vec::new())).is_err());
    }
}

#[test]
fn expansion_support_implies_retention_but_not_the_reverse() {
    let retained = source(
        "s share = lookup r [x] first s force p; s repr = tuples witness { r [x] at s as _ };",
    );
    cq_sequence_plan::select_from_catalog(&retained, &catalog(vec![Support::Expand])).unwrap();
    let expanded = source("p stage = batch 4 expand;");
    assert!(
        cq_sequence_plan::select_from_catalog(&expanded, &catalog(vec![Support::Retain])).is_err()
    );
}

#[test]
fn unused_promises_are_replaced_instead_of_leaking_to_new_provider() {
    let selected =
        cq_sequence_plan::select_from_catalog(&source(""), &catalog(Vec::new())).unwrap();
    assert_eq!(
        selected_provider(&selected),
        &syn::parse_quote!(@read_only as ReadOnly)
    );
    let facts = structures::contract::check(&selected).unwrap();
    for capability in [Support::Retain, Support::Expand, Support::Keys] {
        assert!(!facts.provider_support("r", &["x".into()], None, capability));
    }
    // The new provider cannot acquire an expansion merely by inheriting the old packet.
    let mut invalid = selected;
    invalid
        .structures
        .entries
        .push(syn::parse_quote!(p stage = batch 4 expand;));
    assert!(structures::contract::check(&invalid).is_err());
}
