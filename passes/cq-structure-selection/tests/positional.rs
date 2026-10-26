use cq_lang_physical as physical;
use cq_lang_structures::{self as ir, Structure};
use quote::{ToTokens, quote};
fn physical() -> physical::Program {
    syn::parse_quote! { physical {
        relation L(a:i64,b:i64); relation R(c:i64,d:i64);
        l = pipe rename L@0 {a -> x,b -> y};
        r = pipe rename R@1 {c -> x,d -> z};
        s = let point select r if (z = 1);
        t = let semi eager [x] l,s;
        u = pipe semi eager [x] s,t;
        j = pipe probe first [x] u,t;
        out = pipe canon project j keep {x,y}; return out as answer(x,y);
    }}
}
fn selected() -> ir::Program {
    cq_structure_selection::compile_with(&physical(), quote!(positional(L, R))).unwrap()
}
#[test]
fn decisions_roundtrip_and_physical_is_identical() {
    let source = physical();
    let target = selected();
    assert_eq!(source, target.physical);
    assert_eq!(
        source.to_token_stream().to_string(),
        target.physical.to_token_stream().to_string()
    );
    assert_eq!(
        syn::parse2::<ir::Program>(target.to_token_stream()).unwrap(),
        target
    );
    let facts = ir::contract::check(&target).unwrap();
    assert_eq!(facts.selections.len(), 2);
    assert_eq!(facts.keysets.len(), 2);
    assert_eq!(facts.reads.len(), 3);
    assert!(facts.retained.is_empty());
    assert_eq!(
        cq_structure_selection::compile_with(&source, quote!()).unwrap(),
        cq_structure_selection::compile(&source).unwrap()
    );
}
#[test]
fn lineage_and_summary_forgery_are_rejected_independently() {
    for alteration in 0..7 {
        let mut target = selected();
        for e in &mut target.structures.entries {
            match (&mut e.structure, alteration) {
                (Structure::Selection { origin, .. }, 0) => {
                    origin.base = syn::parse_quote!(L);
                    break;
                }
                (Structure::Selection { origin, .. }, 1) => {
                    origin.columns.pop();
                    break;
                }
                (Structure::Selection { origin, .. }, 2) => {
                    origin.columns[1].column = origin.columns[0].column.clone();
                    break;
                }
                (
                    Structure::Read {
                        predicate: ir::ReadPredicate::Membership { build, .. },
                        ..
                    },
                    3,
                ) => {
                    *build = syn::parse_quote!(j);
                    break;
                }
                (Structure::Read { marker, .. }, 4) => {
                    *marker = syn::parse_quote!(::cq_load::ColumnAt<99>);
                    break;
                }
                (Structure::KeySet { ty, .. }, 5) => {
                    *ty = syn::parse_quote!(::std::collections::HashSet<_>);
                    break;
                }
                (Structure::KeySet { ty, .. }, 6) => {
                    *ty = syn::parse_quote!(::cq_storage::AdaptiveKeySet<_,::cq_storage::BorrowedI64TupleCodec,64>);
                    break;
                }
                _ => {}
            }
        }
        assert!(
            ir::contract::check(&target).is_err(),
            "forgery {alteration}"
        );
    }
}
#[test]
fn representation_rewrite_cannot_change_an_execution_choice() {
    let source = selected();
    let mut target = source.clone();
    if let physical::Operator::Semi(_, k) = &mut target.physical.program.definitions[3].operator {
        k.choice = Some(syn::parse_quote!(lazy));
    }
    assert!(ir::check_rewrite(&source, &target).is_err());
}
#[test]
fn duplicate_and_missing_summaries_and_false_payload_indices_fail() {
    let source = selected();
    let mut duplicate = source.clone();
    duplicate
        .structures
        .entries
        .push(source.structures.entries.last().unwrap().clone());
    assert!(ir::contract::check(&duplicate).is_err());
    for kind in ["read", "build"] {
        let mut target = source.clone();
        target.structures.entries.retain(|e| {
            if kind == "read" {
                !matches!(e.structure, Structure::Read { .. })
            } else {
                !matches!(e.structure, Structure::Built { .. })
            }
        });
        assert!(ir::contract::check(&target).is_err());
    }
}

#[test]
fn index_only_rhs_prunes_dependent_candidates_without_changing_the_old_representation() {
    let physical = syn::parse_quote! { physical {
        relation L(a:i64,b:i64); relation H(c:i64); relation R(d:i64,e:i64);
        l = pipe rename L@0 {a -> x,b -> y}; h = pipe rename H@1 {c -> x};
        a = let semi eager [x] l,h;
        r = pipe rename R@2 {d -> x,e -> z};
        t = let semi eager [x] r,a;
        out = pipe canon project t keep {x,z}; return out as answer(x,z);
    }};
    let original = cq_structure_selection::compile(&physical).unwrap();
    let selected =
        cq_structure_selection::compile_with(&physical, quote!(positional(L, H, R))).unwrap();
    assert_eq!(original, selected);
}

fn join_filter_plan(branch: bool) -> ir::Program {
    let build = if branch {
        quote! {
            h = pipe rename H@2 {e -> z,f -> w};
            b = let probe first [z] s,h;
        }
    } else {
        quote!(b = let project s keep {x,z};)
    };
    let source: physical::Program = syn::parse2(quote! { physical {
        relation L(a:i64,b:i64); relation R(c:i64,d:i64); relation H(e:i64,f:i64);
        l = pipe rename L@0 {a -> x,b -> y};
        r = pipe rename R@1 {c -> x,d -> z};
        s = pipe range select r if (z > 1);
        #build
        j = pipe probe first [x] l,b;
        out = pipe canon project j keep {x,y,z}; return out as answer(x,y,z);
    }})
    .unwrap();
    cq_structure_selection::compile_with(&source, quote!(positional(L, R, H))).unwrap()
}

#[test]
fn join_filters_record_exact_type_and_covering_probe() {
    for branch in [false, true] {
        let selected = join_filter_plan(branch);
        let facts = ir::contract::check(&selected).unwrap();
        assert_eq!(facts.join_filters["l"].len(), 1);
        let filter = &facts.join_filters["l"][0];
        assert_eq!(
            filter.ty,
            syn::parse_quote!(::cq_storage::AdaptiveKeySet<i64, ::cq_storage::I64Codec, 67108864>)
        );
        assert_eq!(filter.covering, "j");
        assert_eq!(
            syn::parse2::<ir::Program>(selected.to_token_stream()).unwrap(),
            selected
        );
        for forgery in 0..4 {
            let mut bad = selected.clone();
            for entry in &mut bad.structures.entries {
                if let Structure::JoinFilter { filter, .. } = &mut entry.structure {
                    match forgery {
                        0 => filter.covering = syn::parse_quote!(out),
                        1 => filter.build = syn::parse_quote!(s),
                        2 => filter.key = syn::parse_quote!([z]),
                        _ => filter.marker = syn::parse_quote!(::cq_load::ColumnAt<1>),
                    }
                }
            }
            assert!(ir::contract::check(&bad).is_err(), "forgery {forgery}");
        }
    }
}

#[test]
fn join_filter_cannot_cross_an_opaque_guard() {
    let mut selected = join_filter_plan(true);
    let index = selected
        .physical
        .program
        .definitions
        .iter()
        .position(|d| d.name == "j")
        .unwrap();
    selected.physical.program.definitions.insert(
        index,
        syn::parse_quote!(g = pipe guard l if (panic!("must not disappear"));),
    );
    if let physical::Operator::Probe(_, k) =
        &mut selected.physical.program.definitions[index + 1].operator
    {
        k.left = syn::parse_quote!(g);
    }
    assert!(ir::contract::check(&selected).is_err());
}

/// A filter reaches the scan through an intermediate probe only on a key
/// column the scan itself carries; a key bound by that probe's build is not
/// a scan column, so filtering the scan by it would be unsound.
#[test]
fn join_filters_cross_probes_only_on_scan_key_columns() {
    let plan = |key: proc_macro2::TokenStream| {
        let source: physical::Program = syn::parse2(quote! { physical {
            relation L(a:i64,b:i64); relation R(c:i64,d:i64); relation H(e:i64,f:i64);
            l = pipe rename L@0 {a -> x,b -> y};
            h = pipe rename H@2 {e -> y,f -> w};
            m = let range select h if (w > 1);
            p = pipe probe first [y] l,m;
            r = pipe rename R@1 {c -> #key,d -> z};
            s = pipe range select r if (z > 1);
            b = let project s keep {#key,z};
            j = pipe probe first [#key] p,b;
            out = pipe canon project j keep {x,y,z}; return out as answer(x,y,z);
        }})
        .unwrap();
        let selected =
            cq_structure_selection::compile_with(&source, quote!(positional(L, R, H))).unwrap();
        let facts = ir::contract::check(&selected).unwrap();
        let mut found: Vec<_> = facts
            .join_filters
            .get("l")
            .into_iter()
            .flatten()
            .map(|f| f.covering.to_string())
            .collect();
        found.sort();
        found
    };
    assert!(plan(quote!(x)).contains(&"j".to_owned()));
    assert!(!plan(quote!(w)).contains(&"j".to_owned()));
}

fn repeated_filter_plan(distinct_build: bool, distinct_layout: bool) -> ir::Program {
    let version = distinct_build.then(|| quote!(c = let project b keep {x,z};));
    let build = if distinct_build { quote!(c) } else { quote!(b) };
    let key = if distinct_layout {
        quote!(x, z)
    } else {
        quote!(x)
    };
    let first = if distinct_layout || distinct_build {
        quote!(probe first)
    } else {
        quote!(semi eager)
    };
    let projection = distinct_build.then(|| quote!(p = pipe project q keep {x,y};));
    let input = if distinct_build { quote!(p) } else { quote!(q) };
    let source: physical::Program = syn::parse2(quote! { physical {
        relation L(a:i64,b:i64); relation R(c:i64,d:i64);
        l = pipe rename L@0 {a -> x,b -> y};
        r = pipe rename R@1 {c -> x,d -> z};
        s = pipe range select r if (z > 1);
        b = let project s keep {x,z};
        #version
        q = pipe #first [x] l,b;
        #projection
        j = pipe probe first [#key] #input,#build;
        out = pipe canon project j keep {x,z}; return out as answer(x,z);
    }})
    .unwrap();
    let selected = cq_structure_selection::compile_with(&source, quote!(positional(L, R))).unwrap();
    assert_eq!(selected.physical, source, "exact operators must remain");
    selected
}

#[test]
fn equivalent_filter_witnesses_share_checked_runtime_work() {
    let selected = repeated_filter_plan(false, false);
    let witnesses: Vec<_> = selected
        .structures
        .entries
        .iter()
        .filter_map(|entry| match &entry.structure {
            Structure::JoinFilter { filter, .. } if entry.name == "l" => Some(filter),
            _ => None,
        })
        .collect();
    assert_eq!(witnesses.len(), 2, "retain both validation witnesses");
    assert_eq!(
        ir::contract::check(&selected).unwrap().join_filters["l"].len(),
        1
    );
    assert_eq!(
        syn::parse2::<ir::Program>(selected.to_token_stream()).unwrap(),
        selected
    );

    // A witness that would otherwise be coalesced is still checked.
    for mutation in 0..2 {
        let mut invalid = selected.clone();
        for entry in &mut invalid.structures.entries {
            if let Structure::JoinFilter { filter, .. } = &mut entry.structure {
                if filter.covering == "j" {
                    if mutation == 0 {
                        filter.marker = syn::parse_quote!(::cq_load::ColumnAt<1>);
                    } else {
                        filter.covering = syn::parse_quote!(out);
                    }
                }
            }
        }
        assert!(ir::contract::check(&invalid).is_err());
    }

    let mut different_type = selected;
    for entry in &mut different_type.structures.entries {
        if let Structure::JoinFilter { filter, .. } = &mut entry.structure {
            if filter.covering == "j" {
                filter.ty = syn::parse_quote!(OtherConservativeFilter);
            }
        }
    }
    assert_eq!(
        ir::contract::check(&different_type).unwrap().join_filters["l"].len(),
        2
    );
}

#[test]
fn filter_build_versions_and_key_layouts_remain_distinct() {
    let versions = repeated_filter_plan(true, false);
    assert_eq!(
        ir::contract::check(&versions).unwrap().join_filters["l"].len(),
        2
    );
    let layouts = repeated_filter_plan(false, true);
    let facts = ir::contract::check(&layouts).unwrap();
    assert_eq!(facts.join_filters["l"].len(), 2);
    assert_eq!(
        facts.join_filters["l"]
            .iter()
            .filter(|f| f.key == syn::parse_quote!([x]))
            .count(),
        2
    );
}

#[test]
fn filters_do_not_share_across_occurrences_of_one_host() {
    let source: physical::Program = syn::parse_quote! { physical {
        relation L(a:i64,b:i64); relation R(c:i64,d:i64);
        l = pipe rename L@0 {a -> x,b -> y};
        m = pipe rename L@2 {a -> x,b -> y};
        r = pipe rename R@1 {c -> x,d -> z};
        b = let range select r if (z > 1);
        p = pipe probe first [x] l,b;
        q = pipe probe first [x] m,b;
        out = pipe canon union p,q; return out as answer(x,y,z);
    }};
    let selected = cq_structure_selection::compile_with(&source, quote!(positional(L, R))).unwrap();
    let facts = ir::contract::check(&selected).unwrap();
    assert_eq!(facts.join_filters["l"].len(), 1);
    assert_eq!(facts.join_filters["m"].len(), 1);
    assert_eq!(selected.physical, source);
}

#[test]
fn filters_preserve_distinct_scalar_key_mappings_and_markers() {
    let source: physical::Program = syn::parse_quote! { physical {
        relation L(a:i64,b:i64); relation R(c:i64,d:i64);
        l = pipe rename L@0 {a -> x,b -> y};
        r = pipe rename R@1 {c -> x,d -> y};
        b = let range select r if (y > 1);
        q = pipe semi eager [x,y] l,b;
        j = pipe probe first [x,y] q,b;
        out = pipe canon project j keep {x,y}; return out as answer(x,y);
    }};
    let selected = cq_structure_selection::compile_with(&source, quote!(positional(L, R))).unwrap();
    let facts = ir::contract::check(&selected).unwrap();
    let filters = &facts.join_filters["l"];
    assert_eq!(filters.len(), 2);
    assert_ne!(filters[0].key, filters[1].key);
    assert_ne!(filters[0].marker, filters[1].marker);
}
