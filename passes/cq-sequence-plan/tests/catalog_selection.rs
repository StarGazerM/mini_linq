use cq_lang_sequence::{CanonToken, Capability, Catalog, ProviderToken, structures};
use cq_sequence_plan::select_from_catalog;
use structures::Structure;

fn h05() -> structures::Program {
    syn::parse_str(include_str!("fixtures/h05.structures")).unwrap()
}

// Target tokens are data, not a dependency on a GPU implementation. These are
// the provider/canon tokens exported by the GPU Sequence target's catalog.
fn gpu_catalog() -> Catalog {
    let levels = vec![vec!["host".into()], vec!["host".into(), "device".into()]];
    Catalog {
        providers: vec![ProviderToken {
            ty: syn::parse_quote!(::cq_gpu_sequence_runtime::RangeProvider),
            capabilities: vec![Capability::Exact, Capability::Range, Capability::Membership],
            levels: levels.clone(),
        }],
        canons: vec![CanonToken {
            ty: syn::parse_quote!(::cq_gpu_sequence_runtime::canon::PackedCanon),
            levels,
        }],
        codecs: vec![],
    }
}

fn provider_type(structure: &Structure) -> Option<&syn::Type> {
    match structure {
        Structure::Built { keyed, .. } => Some(&keyed.ty),
        Structure::Rows { ty, .. } | Structure::Dedup { ty, .. } => Some(ty),
        _ => None,
    }
}

#[test]
fn gpu_catalog_selects_h05_authored_provider_types_from_defaults() {
    let authored = h05();
    let defaults = cq_structure_selection::compile(&authored.physical).unwrap();
    let selected = select_from_catalog(&defaults, &gpu_catalog()).unwrap();
    structures::check_rewrite(&defaults, &selected).unwrap();
    assert_eq!(selected.physical, defaults.physical);
    let mut count = 0;
    for (before, after) in defaults
        .structures
        .entries
        .iter()
        .zip(&selected.structures.entries)
    {
        if let Some(ty) = provider_type(&after.structure) {
            let expected = authored
                .structures
                .entries
                .iter()
                .find(|entry| {
                    entry.name == after.name
                        && std::mem::discriminant(&entry.structure)
                            == std::mem::discriminant(&after.structure)
                })
                .unwrap();
            assert_eq!(
                Some(ty),
                provider_type(&expected.structure),
                "{}",
                after.name
            );
            count += 1;
        } else {
            assert_eq!(after, before, "host adapters must remain unchanged");
        }
    }
    assert_eq!(count, 8);
}

#[test]
fn selection_is_deterministic_and_uses_first_satisfying_token() {
    let source = cq_structure_selection::compile(&h05().physical).unwrap();
    let mut catalog = gpu_catalog();
    let mut wrong_level = catalog.providers[0].clone();
    wrong_level.ty = syn::parse_quote!(WrongLevel);
    wrong_level.levels = vec![vec!["host".into(), "device".into()]];
    let mut membership = catalog.providers[0].clone();
    membership.ty = syn::parse_quote!(MembershipOnly);
    membership.capabilities = vec![Capability::Membership];
    let mut later = catalog.providers[0].clone();
    later.ty = syn::parse_quote!(Later);
    catalog.providers.insert(0, membership);
    catalog.providers.insert(0, wrong_level);
    catalog.providers.push(later);
    let selected = select_from_catalog(&source, &catalog).unwrap();
    assert_eq!(selected, select_from_catalog(&source, &catalog).unwrap());
    assert_eq!(selected, select_from_catalog(&selected, &catalog).unwrap());
    let supplier = selected
        .structures
        .entries
        .iter()
        .find(|entry| {
            entry.name == "supplier_index" && matches!(entry.structure, Structure::Built { .. })
        })
        .unwrap();
    assert_eq!(
        provider_type(&supplier.structure),
        Some(&syn::parse_quote!(MembershipOnly))
    );
    let nation = selected
        .structures
        .entries
        .iter()
        .find(|entry| {
            entry.name == "nation_index" && matches!(entry.structure, Structure::Built { .. })
        })
        .unwrap();
    assert_eq!(
        provider_type(&nation.structure),
        Some(&catalog.providers[2].ty)
    );
}

#[test]
fn rejection_names_structure_capability_and_level() {
    let source = cq_structure_selection::compile(&h05().physical).unwrap();
    let mut catalog = gpu_catalog();
    catalog.providers[0].capabilities.clear();
    let error = select_from_catalog(&source, &catalog)
        .unwrap_err()
        .to_string();
    assert!(error.contains("nation_index"), "{error}");
    assert!(error.contains("Exact"), "{error}");
    assert!(error.contains("host"), "{error}");
    assert_eq!(
        error,
        select_from_catalog(&source, &catalog)
            .unwrap_err()
            .to_string()
    );

    let mut catalog = gpu_catalog();
    catalog.canons[0].levels.clear();
    let error = select_from_catalog(&source, &catalog)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("answer") && error.contains("canon") && error.contains("host"),
        "{error}"
    );
}

#[test]
fn one_provider_must_satisfy_all_reads_not_separate_tokens() {
    let source: structures::Program = syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32);
            l = pipe rename L@0 {a -> a};
            r = let rename R@1 {a -> a,b -> b};
            j = pipe probe first [a] l,r;
            projected = pipe project j keep {a};
            answer = pipe canon semi eager [a] projected,r;
            return answer as answer(a);
        }
        structures {
            l host scan = Reader [];
            r host scan = Reader [];
            r = Index [a];
            answer canon = Canon;
        }
    };
    let mut catalog = gpu_catalog();
    let mut membership = catalog.providers[0].clone();
    membership.capabilities = vec![Capability::Membership];
    catalog.providers[0].capabilities = vec![Capability::Exact];
    catalog.providers.push(membership);
    let error = select_from_catalog(&source, &catalog)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("`r`") && error.contains("Exact") && error.contains("Membership"),
        "{error}"
    );
    catalog.providers[0]
        .capabilities
        .push(Capability::Membership);
    select_from_catalog(&source, &catalog).unwrap();
}
