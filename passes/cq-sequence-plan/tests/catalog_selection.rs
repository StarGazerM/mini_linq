use cq_lang_sequence::{CanonToken, Capability, Catalog, ProviderToken, structures};
use cq_sequence_plan::select_from_catalog;
use structures::Structure;

fn h05() -> structures::Program {
    syn::parse_str(include_str!("fixtures/h05.structures")).unwrap()
}

// Target tokens are data, not a dependency on a GPU implementation. These are
// the provider/canon tokens exported by the GPU Sequence target's catalog.
fn gpu_catalog() -> Catalog {
    let levels = vec![vec!["host".into(), "device".into()]];
    Catalog {
        providers: vec![ProviderToken {
            ty: syn::parse_quote!(::cq_gpu_sequence_runtime::RangeProvider),
            elements: None,
            capabilities: vec![Capability::Exact, Capability::Range, Capability::Membership],
            levels: levels.clone(),
        }],
        canons: vec![CanonToken {
            ty: syn::parse_quote!(::cq_gpu_sequence_runtime::canon::PackedCanon),
            levels: levels.clone(),
            max_packed_bits: Some(128),
        }, CanonToken {
            ty: syn::parse_quote!(::cq_gpu_sequence_runtime::canon::WideCanon),
            levels,
            max_packed_bits: None,
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
    wrong_level.levels.clear();
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
fn rejection_names_structure_and_catalog_read_obligation() {
    let source = cq_structure_selection::compile(&h05().physical).unwrap();
    let mut catalog = gpu_catalog();
    catalog.providers[0].capabilities.clear();
    let error = select_from_catalog(&source, &catalog)
        .unwrap_err()
        .to_string();
    assert!(error.contains("nation_index"), "{error}");
    assert!(error.contains("Exact"), "{error}");
    assert!(error.contains("read obligation"), "{error}");
    assert_eq!(
        error,
        select_from_catalog(&source, &catalog)
            .unwrap_err()
            .to_string()
    );

    let mut catalog = gpu_catalog();
    for canon in &mut catalog.canons {
        canon.levels.clear();
    }
    let error = select_from_catalog(&source, &catalog)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("answer") && error.contains("canon") && error.contains("read obligation"),
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
    catalog.providers[0].capabilities = vec![Capability::Exact, Capability::Range];
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

fn host_keyed_read() -> structures::Program {
    syn::parse_quote! {
        physical {
            relation L(a:i64); relation R(a:i64,b:i64);
            l = pipe rename L@0 {a -> a};
            r = pipe rename R@1 {a -> a,b -> b};
            answer = pipe canon probe first [a] l,r;
            return answer as answer(a,b);
        }
        structures {
            l host scan = HostColumns [];
            r host scan = HostColumns [];
            r host answer = HostColumns [a];
            answer canon = Canon;
        }
    }
}

#[test]
fn integral_host_fallback_is_retained_with_a_catalog_range_index() {
    let source = host_keyed_read();
    let selected = select_from_catalog(&source, &gpu_catalog()).unwrap();
    structures::physical::check_rewrite(&source.physical, &selected.physical).unwrap();
    assert_eq!(
        selected,
        select_from_catalog(&source, &gpu_catalog()).unwrap()
    );
    assert_eq!(
        selected,
        select_from_catalog(&selected, &gpu_catalog()).unwrap()
    );
    let analysis = structures::contract::check(&selected).unwrap();
    assert_eq!(
        analysis.built_on("r", &["a".into()]).map(|(ty, _)| ty),
        Some(&gpu_catalog().providers[0].ty)
    );
    for before in &source.structures.entries {
        if matches!(&before.structure, Structure::Host { reader, .. } if reader == "scan") {
            assert!(
                selected.structures.entries.contains(before),
                "host column scans remain unchanged"
            );
        }
    }
}

#[test]
fn unsupported_host_range_read_rejects_instead_of_skipping_the_obligation() {
    let mut catalog = gpu_catalog();
    catalog.providers[0].capabilities = vec![Capability::Exact, Capability::Membership];
    let error = select_from_catalog(&host_keyed_read(), &catalog)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("`r`") && error.contains("Range") && error.contains("host"),
        "{error}"
    );
}

#[test]
fn host_range_index_selection_does_not_pick_an_exact_only_token() {
    let mut catalog = gpu_catalog();
    let mut exact = catalog.providers[0].clone();
    exact.ty = syn::parse_quote!(ExactOnly);
    exact.capabilities = vec![Capability::Exact];
    catalog.providers.insert(0, exact);
    let selected = select_from_catalog(&host_keyed_read(), &catalog).unwrap();
    let analysis = structures::contract::check(&selected).unwrap();
    assert_eq!(
        analysis.built_on("r", &["a".into()]).map(|(ty, _)| ty),
        Some(&catalog.providers[1].ty)
    );
}

#[test]
fn catalog_element_capability_checks_both_built_and_host_fallback_rows() {
    for retained in [false, true] {
        let mut source: structures::Program = syn::parse_quote! {
            physical {
                relation L(a:i64); relation R(a:i64,b:String);
                l = pipe rename L@0 {a -> a};
                r = pipe rename R@1 {a -> a,b -> b};
                answer = pipe canon probe first [a] l,r;
                return answer as answer(a,b);
            }
            structures {
                l host scan = HostColumns [];
                r host scan = HostColumns [];
                r host answer = HostColumns [a];
                answer canon = Canon;
            }
        };
        if retained {
            source.physical.program.definitions[1].mode =
                structures::physical::Mode::Let(Default::default());
            source.structures.entries[2] = syn::parse_quote!(r = HostIndex [a];);
        }
        let mut catalog = gpu_catalog();
        catalog.providers[0].elements = Some(vec![syn::parse_quote!(i64)]);
        assert!(select_from_catalog(&source, &catalog).is_err());
        let mut host = catalog.providers[0].clone();
        host.ty = syn::parse_quote!(HostIndex);
        host.elements = None;
        host.levels = vec![vec!["host".into()]];
        catalog.providers.push(host);
        let selected = select_from_catalog(&source, &catalog).unwrap();
        let analysis = structures::contract::check(&selected).unwrap();
        assert_eq!(
            analysis.built_on("r", &["a".into()]).map(|(ty, _)| ty),
            Some(&syn::parse_quote!(HostIndex))
        );
    }
}

#[test]
fn unused_string_payload_is_projected_from_renamed_provider() {
    for retained in [false, true] {
        let mut physical: structures::physical::Program = syn::parse_quote! {
            physical {
                relation L(a:i64); relation R(a:i64,unused:String);
                l = pipe rename L@0 {a -> a};
                r = pipe rename R@1 {a -> a,unused -> unused};
                joined = pipe probe first [a] l,r;
                answer = pipe canon project joined keep {a};
                return answer as answer(a);
            }
        };
        if retained {
            physical.program.definitions[1].mode =
                structures::physical::Mode::Let(Default::default());
        }
        let source = cq_structure_selection::compile(&physical).unwrap();
        let mut catalog = gpu_catalog();
        catalog.providers[0].elements = Some(vec![syn::parse_quote!(i64)]);
        let selected = select_from_catalog(&source, &catalog).unwrap();
        let analysis = structures::contract::check(&selected).unwrap();
        assert_eq!(analysis.layouts["r"], ["a"]);
        assert_eq!(
            analysis.built_on("r", &["a".into()]).map(|(ty, _)| ty),
            Some(&catalog.providers[0].ty)
        );
    }
}

#[test]
fn canon_selection_uses_complete_heading_width() {
    for (third, expected) in [(false, "PackedCanon"), (true, "WideCanon")] {
        let mut source: structures::Program = syn::parse_quote! {
            physical {
                relation R(a:i64,b:i64,c:i64);
                r = pipe rename R@0 {a -> a,b -> b,c -> c};
                answer = pipe canon project r keep {a,b,c};
                return answer as answer(a,b,c);
            }
            structures {
                r host scan = HostColumns [];
                answer canon = Canon;
            }
        };
        if !third {
            source = syn::parse_quote! {
                physical {
                    relation R(a:i64,b:i64);
                    r = pipe rename R@0 {a -> a,b -> b};
                    answer = pipe canon project r keep {a,b};
                    return answer as answer(a,b);
                }
                structures {
                    r host scan = HostColumns [];
                    answer canon = Canon;
                }
            };
        }
        let selected = select_from_catalog(&source, &gpu_catalog()).unwrap();
        structures::check_rewrite(&source, &selected).unwrap();
        let analysis = structures::contract::check(&selected).unwrap();
        let syn::Type::Path(ty) = &analysis.dedup["answer"] else {
            panic!("provider path")
        };
        assert_eq!(ty.path.segments.last().unwrap().ident, expected);
        if third {
            let mut catalog = gpu_catalog();
            catalog.canons.pop();
            assert!(select_from_catalog(&source, &catalog).is_err());
        }
    }
}
