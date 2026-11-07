use cq_lang_sequence::{Catalog, structures};
use cq_sequence_plan::select_from_catalog;
use structures::{Capability, Declaration, Provider, Structure};

fn h05() -> structures::Program {
    syn::parse_str(include_str!("fixtures/h05.structures")).unwrap()
}

// Target tokens are data, not a dependency on a GPU implementation. These are
// the provider tokens exported by the GPU Sequence target's catalog, including
// its Distinct providers of canonical rows records.
fn gpu_catalog() -> Catalog {
    let declared: structures::Providers = syn::parse_quote!(providers {
        range_provider = ::cq_gpu_sequence_runtime::RangeProvider {
            reads [exact, range, membership], build inferred
        };
        radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> {
            reads [distinct], build inferred
        };
    });
    let mut catalog = Catalog::default();
    for declaration in declared.declarations {
        catalog.register(declaration, vec![vec!["host".into(), "device".into()]]);
    }
    catalog
}

fn declaration<'a>(catalog: &'a mut Catalog, token: &str) -> &'a mut Declaration {
    catalog
        .declarations
        .iter_mut()
        .find(|declaration| declaration.token == token)
        .unwrap()
}

/// Register `declaration` at the range provider's levels, `index` in catalog order.
fn insert(catalog: &mut Catalog, index: usize, declaration: Declaration) {
    let levels = catalog.providers[0].levels.clone();
    catalog.register(declaration, levels);
    let entry = catalog.providers.pop().unwrap();
    catalog.providers.insert(index, entry);
}

fn provider(structure: &Structure) -> Option<&Provider> {
    match structure {
        Structure::Built { keyed, .. } => Some(&keyed.provider),
        Structure::Rows { provider, .. } => Some(provider),
        _ => None,
    }
}

#[test]
fn gpu_catalog_selects_h05_authored_provider_types_from_defaults() {
    let authored = h05();
    let defaults = cq_structure_selection::compile(&authored.physical).unwrap();
    // No type-width guessing: the full-row set token accepts the recorded
    // packed row codec regardless of its eventual lane count.
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
        if let Some(selected) = provider(&after.structure) {
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
                Some(selected),
                provider(&expected.structure),
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
    insert(
        &mut catalog,
        0,
        syn::parse_quote!(membership_only = MembershipOnly { reads [membership], build inferred };),
    );
    insert(
        &mut catalog,
        0,
        syn::parse_quote!(wrong_level = WrongLevel { reads [exact, range, membership], build inferred };),
    );
    catalog.providers[0].levels.clear();
    let later: Declaration =
        syn::parse_quote!(later = Later { reads [exact, range, membership], build inferred };);
    insert(&mut catalog, 4, later);
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
        provider(&supplier.structure),
        Some(&syn::parse_quote!(@membership_only as MembershipOnly))
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
        provider(&nation.structure),
        Some(&catalog.providers[2].provider)
    );
}

#[test]
fn unavailable_read_or_canonicalization_cannot_be_selected() {
    let source = cq_structure_selection::compile(&h05().physical).unwrap();
    let mut missing_read = gpu_catalog();
    declaration(&mut missing_read, "range_provider")
        .facts
        .reads
        .clear();
    assert!(select_from_catalog(&source, &missing_read).is_err());
    let mut missing_canon = gpu_catalog();
    let catalog = gpu_catalog();
    for entry in &mut missing_canon.providers {
        if catalog
            .facts(&entry.provider)
            .unwrap()
            .reads(&Capability::Distinct)
        {
            entry.levels.clear();
        }
    }
    assert!(select_from_catalog(&source, &missing_canon).is_err());
}

#[test]
fn one_provider_must_satisfy_all_reads_not_separate_tokens() {
    let source: structures::Program = syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32);
            l = pipe rename L {a -> a};
            r = let rename R {a -> a,b -> b};
            j = pipe probe first [a] l,r;
            projected = pipe project j keep {a};
            answer = let canon semi eager [a] projected,r;
            return answer as answer(a);
        }
        structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; index = Index { reads [exact, range, membership], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
            l host scan = @reader as Reader [];
            r host scan = @reader as Reader [];
            r = @index as Index [a];
            answer rows = @sort as ::cq_storage::dedup::Sort;
            answer canon = rows;
        }
    };
    let mut catalog = gpu_catalog();
    declaration(&mut catalog, "range_provider").facts.reads =
        vec![Capability::Exact, Capability::Range];
    insert(
        &mut catalog,
        2,
        syn::parse_quote!(membership = Membership { reads [membership], build inferred };),
    );
    assert!(select_from_catalog(&source, &catalog).is_err());
    declaration(&mut catalog, "range_provider")
        .facts
        .reads
        .push(Capability::Membership);
    select_from_catalog(&source, &catalog).unwrap();
}

fn host_keyed_read() -> structures::Program {
    syn::parse_quote! {
        physical {
            relation L(a:i64); relation R(a:i64,b:i64);
            l = pipe rename L {a -> a};
            r = pipe rename R {a -> a,b -> b};
            answer = let canon probe first [a] l,r;
            return answer as answer(a,b);
        }
        structures { providers { host_columns = HostColumns { reads [exact, range, membership], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
            l host scan = @host_columns as HostColumns [];
            r host scan = @host_columns as HostColumns [];
            r host result(answer) = @host_columns as HostColumns [a];
            answer rows = @sort as ::cq_storage::dedup::Sort;
            answer canon = rows;
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
        analysis
            .built_on("r", &["a".into()])
            .map(|(provider, _)| provider),
        Some(&gpu_catalog().providers[0].provider)
    );
    for before in &source.structures.entries {
        if matches!(&before.structure, Structure::Host { reader, .. } if matches!(reader, cq_lang_sequence::structures::ReaderRef::Scan))
        {
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
    declaration(&mut catalog, "range_provider").facts.reads =
        vec![Capability::Exact, Capability::Membership];
    assert!(select_from_catalog(&host_keyed_read(), &catalog).is_err());
}

#[test]
fn host_range_index_selection_does_not_pick_an_exact_only_token() {
    let mut catalog = gpu_catalog();
    insert(
        &mut catalog,
        0,
        syn::parse_quote!(exact_only = ExactOnly { reads [exact], build inferred };),
    );
    let selected = select_from_catalog(&host_keyed_read(), &catalog).unwrap();
    let analysis = structures::contract::check(&selected).unwrap();
    assert_eq!(
        analysis
            .built_on("r", &["a".into()])
            .map(|(provider, _)| provider),
        Some(&catalog.providers[1].provider)
    );
}

#[test]
fn unused_code_payload_is_projected_from_renamed_provider() {
    for retained in [false, true] {
        let mut physical: structures::physical::Program = syn::parse_quote! {
            physical {
                relation L(a:i64); relation R(a:i64,unused:StringCode);
                l = pipe rename L {a -> a};
                r = pipe rename R {a -> a,unused -> unused};
                joined = pipe probe first [a] l,r;
                answer = let canon project joined keep {a};
                return answer as answer(a);
            }
        };
        if retained {
            physical.program.definitions[1].mode =
                structures::physical::Mode::Let(Default::default());
        }
        let source = cq_structure_selection::compile(&physical).unwrap();
        let catalog = gpu_catalog();
        let selected = select_from_catalog(&source, &catalog).unwrap();
        let analysis = structures::contract::check(&selected).unwrap();
        assert_eq!(analysis.layouts["r"], ["a"]);
        assert_eq!(
            analysis
                .built_on("r", &["a".into()])
                .map(|(provider, _)| provider),
            Some(&catalog.providers[0].provider)
        );
    }
}

#[test]
fn canon_selection_requires_codec_bounds_not_type_width_guesses() {
    for (third, expected) in [(false, "radix_set"), (true, "radix_set")] {
        let mut source: structures::Program = syn::parse_quote! {
            physical {
                relation R(a:i64,b:i64,c:i64);
                r = pipe rename R {a -> a,b -> b,c -> c};
                answer = let canon project r keep {a,b,c};
                return answer as answer(a,b,c);
            }
            structures { providers { host_columns = HostColumns { reads [exact, range, membership], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
                r host scan = @host_columns as HostColumns [];
                answer rows = @sort as ::cq_storage::dedup::Sort;
                answer canon = rows;
            }
        };
        if !third {
            source = syn::parse_quote! {
                physical {
                    relation R(a:i64,b:i64);
                    r = pipe rename R {a -> a,b -> b};
                    answer = let canon project r keep {a,b};
                    return answer as answer(a,b);
                }
                structures { providers { host_columns = HostColumns { reads [exact, range, membership], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
                    r host scan = @host_columns as HostColumns [];
                    answer rows = @sort as ::cq_storage::dedup::Sort;
                    answer canon = rows;
                }
            };
        }
        let selected = select_from_catalog(&source, &gpu_catalog()).unwrap();
        structures::check_rewrite(&source, &selected).unwrap();
        let analysis = structures::contract::check(&selected).unwrap();
        assert_eq!(
            analysis.canonical["answer"].rows_provider().unwrap().token,
            expected
        );
        if third {
            let mut catalog = gpu_catalog();
            catalog.providers.pop();
            assert!(select_from_catalog(&source, &catalog).is_err());
        }
    }
}
