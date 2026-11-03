use cq_lang_physical as physical;
use cq_lang_structures as structures;
use cq_structure_selection::compile;
use cq_testkit::corpus as scenarios;
use quote::ToTokens;

/// Entry rewrites in these tests name base tokens and `custom`: declare them
/// and prune replaced ones before checking (STR-Provider).
fn declared(program: &structures::Program) -> structures::Program {
    let mut declarations = cq_lang_structures::provider::base::declarations();
    declarations.push(syn::parse_quote!(
        custom = Custom { reads [exact, range, membership], build inferred };
    ));
    let mut program = program.clone();
    program.structures.declare(&declarations).unwrap();
    program
}

fn check(source: &physical::Program, target: &structures::Program) -> syn::Result<()> {
    cq_structure_selection::check(source, &declared(target))
}

fn contract(program: &structures::Program) -> syn::Result<structures::Analysis> {
    structures::contract::check(&declared(program))
}

#[test]
fn cartesian_probe_scans_retained_rows_and_rejects_empty_key_indexes() {
    let source: physical::Program = syn::parse_quote!(physical {
        relation r0(c0:i32); relation r1(c0:i32,c1:i32);
        left = pipe rename r0@0 {c0 -> v0};
        right = let rename r1@1 {c0 -> v1, c1 -> v2};
        joined = pipe probe first [] left,right;
        result = let canon project joined keep {v0,v2};
        return result as answer(v0,v2);
    });
    let mut target = compile(&source).unwrap();
    let facts = contract(&target).unwrap();
    assert!(facts.built.get("right").is_none_or(Vec::is_empty));
    check(&source, &target).unwrap();
    target.structures.entries.push(syn::parse_quote!(
        right = @hash_index as ::cq_storage::HashIndex<'_, _, _> [];
    ));
    assert!(check(&source, &target).is_err());
}

#[test]
fn nonleading_native_point_uses_scan_and_rejects_authored_native() {
    let source: physical::Program = syn::parse_quote!(physical {
        relation R(a:i32,b:i32) host [b] reads [exact];
        r = pipe rename R@0 {a -> x,b -> y};
        p = let canon point select r if (y = 2);
        return p as answer(x,y);
    });
    let mut target = compile(&source).unwrap();
    let analysis = contract(&target).unwrap();
    assert!(analysis.natives.is_empty());
    target.structures.entries.push(syn::parse_quote!(
        r native result(p) = exact [y];
    ));
    assert!(contract(&target).is_err());
}

#[test]
fn full_row_positive_probes_record_membership_without_a_filtered_source() {
    for mode in [quote::quote!(pipe), quote::quote!(let)] {
        let source: physical::Program = syn::parse_quote! { physical {
            relation L(a: CallerOwnedAlias, id:i64);
            relation R(a: CallerOwnedAlias);
            l = pipe rename L@0 {a -> x,id -> id};
            r = #mode rename R@1 {a -> x};
            joined = pipe probe first [x] l,r;
            result = let canon project joined keep {id};
            return result as answer(id);
        }};
        let target = compile(&source).unwrap();
        let facts = contract(&target).unwrap();
        assert!(facts.membership.contains(&("r".into(), vec!["x".into()])));
        assert!(!facts.filtered.contains_key("r"));
        let provider = facts
            .built
            .get("r")
            .map(|entries| entries[0].1.clone())
            .or_else(|| {
                facts
                    .fallbacks
                    .iter()
                    .find(|((name, _, key), _)| name == "r" && key == &["x"])
                    .map(|(_, ty)| ty.clone())
            })
            .unwrap();
        assert_eq!(
            provider,
            syn::parse_quote!(@dense_membership as ::cq_storage::DenseMembership<_, 16777216>)
        );
        check(&source, &target).unwrap();
    }
}

#[test]
fn raw_membership_proof_rejects_unbound_payload_columns() {
    let mut source: structures::Program = syn::parse_quote! {
        physical {
            relation L(a:i64); relation R(a:i64,b:i64);
            l = pipe rename L@0 {a -> x};
            r = let rename R@1 {a -> x,b -> y};
            joined = pipe probe first [x] l,r;
            result = let canon project joined keep {x,y};
            return result as answer(x,y);
        }
        structures { providers { adaptive_membership = ::cq_storage::AdaptiveMembership<_, _> { reads [membership], build typed, shared }; vec = ::std::vec::Vec<_> { reads [], build typed, accounting rows }; host_scan = ::core::iter::Map<_, _> { reads [], build typed }; sort = ::cq_storage::dedup::Sort { reads [distinct, comparator_rows], build inferred, shared }; }
            r = @adaptive_membership as ::cq_storage::AdaptiveMembership<_, 64> [x] membership;
            r rows = @vec as ::std::vec::Vec<_>;
            l host scan = @host_scan as ::core::iter::Map<_, _> [];
            r host scan = @host_scan as ::core::iter::Map<_, _> [];
            result rows = @sort as ::cq_storage::dedup::Sort; result canon = rows;
        }
    };
    assert!(contract(&source).is_err());
    source.structures.entries[0] = syn::parse_quote!(
        r = @hash_index as ::cq_storage::HashIndex<'_, _, _> [x];
    );
    contract(&source).unwrap();
}
#[test]
fn grouped_folds_keep_complete_valuations_and_infer_only_the_group_key() {
    let source: physical::Program = syn::parse_quote! { physical {
        relation R(k:i32,id:i32,v:i32);
        relation answer(k:i32,n:usize,total:i32,lo:Option<i32>,hi:Option<i32>);
        r = pipe rename R@0 {k -> k,id -> id,v -> v};
        valuations = let canon project r keep {k,id,v};
        grouped = pipe group valuations by [k] {
            n:usize = count();
            total:i32 = sum(v);
            lo:Option<i32> = min(v);
            hi:Option<i32> = max(v);
        };
        return grouped as answer(k,n,total,lo,hi);
    }};
    let target = compile(&source).unwrap();
    check(&source, &target).unwrap();
    let facts = contract(&target).unwrap();
    assert_eq!(facts.unique["valuations"], ["id", "k", "v"]);
    assert_eq!(facts.unique["grouped"], ["k"]);
    assert!(facts.canonical.contains_key("valuations"));
    assert!(!facts.canonical.contains_key("grouped"));
    assert_eq!(
        facts.groups["grouped"],
        syn::parse_quote!(
            @group_map as ::std::collections::HashMap<_, (usize, i32, Option<i32>, Option<i32>,), ::cq_storage::HashState>
        )
    );
    let group_entry = target
        .structures
        .entries
        .iter()
        .position(|entry| matches!(entry.structure, structures::Structure::Group { .. }))
        .unwrap();
    let mut missing = target.clone();
    missing.structures.entries.remove(group_entry);
    assert!(contract(&missing).is_err());
    let mut duplicate = target.clone();
    duplicate
        .structures
        .entries
        .push(target.structures.entries[group_entry].clone());
    assert!(contract(&duplicate).is_err());
    let mut misplaced = target.clone();
    misplaced.structures.entries[group_entry].name = syn::parse_quote!(valuations);
    assert!(contract(&misplaced).is_err());
    assert_eq!(facts.layouts["valuations"], ["id", "k", "v"]);
    assert_eq!(
        facts.physical.rows["grouped"]
            .computed
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["hi", "lo", "n", "total"]
    );
    assert!(!facts.lineage.contains_key("grouped"));
    assert_eq!(
        syn::parse2::<structures::Program>(target.to_token_stream()).unwrap(),
        target
    );

    let mut noncanonical = source;
    noncanonical.program.definitions[1].canon = None;
    assert!(compile(&noncanonical).is_err());
}

#[test]
fn global_group_is_unique_by_the_empty_key() {
    let source: physical::Program = syn::parse_quote! { physical {
        relation R(v:i32); relation answer(n:usize,total:i32,lo:Option<i32>,hi:Option<i32>);
        r = let canon rename R@0 {v -> v};
        grouped = pipe group r by [] {
            n:usize = count(); total:i32 = sum(v);
            lo:Option<i32> = min(v); hi:Option<i32> = max(v);
        };
        return grouped as answer(n,total,lo,hi);
    }};
    let target = compile(&source).unwrap();
    let facts = contract(&target).unwrap();
    assert!(facts.unique["grouped"].is_empty());
    assert!(!facts.canonical.contains_key("grouped"));
    assert!(facts.groups.is_empty());
    let mut extra = target.clone();
    extra
        .structures
        .entries
        .push(syn::parse_quote!(grouped group = @group_map as SomeMap<_,_>;));
    extra
        .structures
        .declare(&[cq_lang_structures::provider::base::declaration("group_map")])
        .unwrap();
    assert!(contract(&extra).is_err());
}

#[test]
fn literal_occurrences_select_ordinary_rows_or_shared_indices() {
    let scanned: physical::Program = syn::parse_quote! { physical {
        relation R(a:i32,b:i32);
        r = let canon rename R@0 {a -> x} constants {b = 7};
        return r as answer(x);
    }};
    let selected = compile(&scanned).unwrap();
    let facts = contract(&selected).unwrap();
    assert!(facts.retained.contains_key("r"));
    assert!(!facts.filtered.contains_key("r"));
    assert!(!facts.lineage.contains_key("r"));
    assert_eq!(
        syn::parse2::<structures::Program>(selected.to_token_stream()).unwrap(),
        selected
    );
    let probed: physical::Program = syn::parse_quote! { physical {
        relation L(a:i32); relation R(a:i32,b:i32);
        l = pipe rename L@0 {a -> x};
        r = let rename R@1 {a -> x} constants {b = 7};
        j = let canon probe first [x] l,r;
        return j as answer(x);
    }};
    let selected = compile(&probed).unwrap();
    let facts = contract(&selected).unwrap();
    assert_eq!(facts.built["r"].len(), 1);
    assert!(!facts.filtered.contains_key("r"));
    assert!(!facts.retained.contains_key("r"));
    assert!(!facts.host_read("r"));
    let mut membership = probed;
    membership.program.definitions[2] = syn::parse_quote!(j = let canon semi eager [x] l,r;);
    let selected = compile(&membership).unwrap();
    let facts = contract(&selected).unwrap();
    assert_eq!(
        facts.built["r"][0].1,
        syn::parse_quote!(@hash_set as ::cq_storage::HashSet<_>)
    );
    assert!(!facts.filtered.contains_key("r"));
    assert!(!facts.retained.contains_key("r"));
}

#[test]
fn literal_occurrences_retain_ordinary_rows_for_multiple_indices() {
    let source: physical::Program = syn::parse_quote! { physical {
        relation L(a:i32); relation R(tag:i32,a:i32,b:i32);
        l = pipe rename L@0 {a -> x};
        r = let rename R@1 {a -> x,b -> y} constants {tag = 7};
        p = pipe probe first [x] l,r;
        q = pipe project p keep {y};
        j = let canon probe first [y] q,r;
        return j as answer(x,y);
    }};
    let selected = compile(&source).unwrap();
    let facts = contract(&selected).unwrap();
    assert_eq!(facts.built["r"].len(), 2);
    assert!(facts.retained.contains_key("r"));
    assert!(!facts.filtered.contains_key("r"));
    assert!(!facts.host_read("r"));
    assert!(selected.structures.entries.iter().all(|entry| {
        !matches!(&entry.structure, structures::Structure::Built { keyed, .. }
            if entry.name == "r" && keyed.filtered.is_some())
    }));
}

#[test]
fn existential_filtered_probe_records_key_only_storage_and_checks_projection() {
    let source: physical::Program = syn::parse_quote! {physical {
        relation L(a:i64); relation R(a:i64,b:i64);
        l = pipe rename L@0 {a -> x}; r = pipe rename R@1 {a -> x,b -> y};
        selected = let range select r if (y > 1);
        joined = pipe probe first [x] l,selected;
        result = let canon project joined keep {x}; return result as answer(x);
    }};
    let target = compile(&source).unwrap();
    let facts = contract(&target).unwrap();
    assert!(
        facts
            .membership
            .contains(&("selected".into(), vec!["x".into()]))
    );
    assert_eq!(
        facts.built["selected"][0].1,
        syn::parse_quote!(@dense_membership as ::cq_storage::DenseMembership<_, 16777216>)
    );
    let mut changed = target.clone();
    let entry = changed
        .structures
        .entries
        .iter_mut()
        .find(|entry| {
            entry.name == "selected"
                && matches!(entry.structure, structures::Structure::Built { .. })
        })
        .unwrap();
    let structures::Structure::Built { keyed, .. } = &mut entry.structure else {
        unreachable!()
    };
    keyed
        .filtered
        .as_mut()
        .unwrap()
        .payload
        .columns
        .push(syn::parse_quote!(y));
    assert!(check(&source, &changed).is_err());
    let mut alternate_provider = target.clone();
    let entry = alternate_provider
        .structures
        .entries
        .iter_mut()
        .find(|entry| {
            entry.name == "selected"
                && matches!(entry.structure, structures::Structure::Built { .. })
        })
        .unwrap();
    let structures::Structure::Built { keyed, .. } = &mut entry.structure else {
        unreachable!()
    };
    keyed.provider = cq_lang_structures::provider::base::instance(
        "selection",
        syn::parse_quote!(::cq_storage::Selection<_>),
    );
    alternate_provider
        .structures
        .declare(&[cq_lang_structures::provider::base::declaration("selection")])
        .unwrap();
    // Provider capabilities are Rust obligations, not inferred from its name.
    check(&source, &alternate_provider).unwrap();
    let mut changed = target;
    let entry = changed
        .structures
        .entries
        .iter_mut()
        .find(|entry| {
            entry.name == "selected"
                && matches!(entry.structure, structures::Structure::Built { .. })
        })
        .unwrap();
    let structures::Structure::Built { keyed, .. } = &mut entry.structure else {
        unreachable!()
    };
    keyed.membership = None;
    // Without the membership marker this is an ordinary keyed provider; its
    // payload-read capability is a rustc obligation, not a provider name check.
    check(&source, &changed).unwrap();
    let mut grouped_types = source;
    for declaration in &mut grouped_types.program.declarations {
        for column in &mut declaration.columns {
            column.ty = syn::Type::Group(syn::TypeGroup {
                group_token: Default::default(),
                elem: Box::new(column.ty.clone()),
            });
        }
    }
    let grouped = compile(&grouped_types).unwrap();
    assert!(
        contract(&grouped)
            .unwrap()
            .membership
            .contains(&("selected".into(), vec!["x".into()]))
    );
}

#[test]
fn composite_filtered_keys_retain_their_original_provider() {
    let source: physical::Program = syn::parse_quote! {physical {
        relation L(a:i64,b:i64); relation R(a:i64,b:i64,c:i64);
        l = pipe rename L@0 {a -> x,b -> y};
        r = pipe rename R@1 {a -> x,b -> y,c -> z};
        selected = let range select r if (z > 1);
        joined = pipe probe first [x,y] l,selected;
        result = let canon project joined keep {x,y};
        return result as answer(x,y);
    }};
    for (reader, provider) in [
        (
            syn::parse_quote!(probe first [x,y] l,selected),
            syn::parse_quote!(@hash_index as ::cq_storage::HashIndex<'_, _, _>),
        ),
        (
            syn::parse_quote!(semi eager [x,y] l,selected),
            syn::parse_quote!(@hash_set as ::cq_storage::HashSet<_>),
        ),
    ] {
        let mut source = source.clone();
        source.program.definitions[3].operator = reader;
        let target = compile(&source).unwrap();
        let facts = contract(&target).unwrap();
        assert!(facts.filtered.contains_key("selected"));
        assert!(
            !facts
                .membership
                .contains(&("selected".into(), vec!["x".into(), "y".into()],))
        );
        assert_eq!(facts.built["selected"][0].1, provider);
        check(&source, &target).unwrap();
    }
}

#[test]
fn reconstructed_key_constants_do_not_disable_singleton_membership() {
    let source: physical::Program = syn::parse_quote! {physical {
        relation L(a:i64,b:i64); relation R(a:i64,b:i64);
        l = pipe rename L@0 {a -> x,b -> y};
        r = pipe rename R@1 {a -> x,b -> y};
        selected = let point select r if (y = 1);
        joined = pipe probe first [x,y] l,selected;
        result = let canon project joined keep {x,y};
        return result as answer(x,y);
    }};
    let target = compile(&source).unwrap();
    let facts = contract(&target).unwrap();
    assert!(
        facts
            .membership
            .contains(&("selected".into(), vec!["x".into(), "y".into()],))
    );
    assert_eq!(
        facts.built["selected"][0].1,
        syn::parse_quote!(@dense_membership as ::cq_storage::DenseMembership<_, 16777216>)
    );
    check(&source, &target).unwrap();
}

#[test]
fn overlapping_union_cannot_drop_its_final_dedup_provider() {
    let source: physical::Program = syn::parse_quote! {physical {
        relation R(a:i32); relation S(a:i32);
        r = pipe rename R@0.0.0 {a -> x}; s = pipe rename S@0.1.0 {a -> x};
        u = pipe union r,s block @0 free[] interface[x]; return canon u as answer(x);
    }};
    let target = compile(&source).unwrap();
    assert!(
        target
            .structures
            .entries
            .iter()
            .any(|entry| matches!(entry.structure, structures::Structure::HeadDedup { .. }))
    );
    let mut dropped = target.clone();
    dropped
        .structures
        .entries
        .retain(|entry| !matches!(entry.structure, structures::Structure::HeadDedup { .. }));
    assert!(check(&source, &dropped).is_err());
}

#[test]
fn head_dedup_types_exactly_the_recorded_physical_obligation() {
    // An injective head with an authored `return dedup`: the extra obligation
    // is admissible and selection types it with an answer identity index.
    let source: physical::Program = syn::parse_quote! {physical {
        relation R(a:i32);
        host_set R;
        r = pipe rename R@0 {a -> x};
        return dedup r as answer(x);
    }};
    let head = |program: &structures::Program| {
        program
            .structures
            .entries
            .iter()
            .any(|entry| matches!(entry.structure, structures::Structure::HeadDedup { .. }))
    };
    let target = compile(&source).unwrap();
    assert!(head(&target));
    check(&source, &target).unwrap();
    let mut dropped = target.clone();
    dropped
        .structures
        .entries
        .retain(|entry| !matches!(entry.structure, structures::Structure::HeadDedup { .. }));
    assert!(check(&source, &dropped).is_err());
    // Without the recorded obligation selection adds no index, and an
    // authored one types nothing.
    let mut plain = source;
    plain.program.return_dedup = None;
    let selected = compile(&plain).unwrap();
    assert!(!head(&selected));
    let mut extra = selected;
    extra.structures.entries.extend(
        target
            .structures
            .entries
            .iter()
            .filter(|entry| matches!(entry.structure, structures::Structure::HeadDedup { .. }))
            .cloned(),
    );
    assert!(contract(&extra).is_err());
}

/// STR-ReaderChoice: the host key set records token `keyset` for eager and
/// lazy readers alike; the reader's recorded Physical choice alone prints the
/// declared argument.
#[test]
fn reader_choice_prints_the_keyset_laziness() {
    for (choice, printed) in [
        (
            quote::quote!(eager),
            quote::quote!(::cq_storage::dispatch::KeySet<_, true>),
        ),
        (
            quote::quote!(lazy),
            quote::quote!(::cq_storage::dispatch::KeySet<_, false>),
        ),
    ] {
        let source: physical::Program = syn::parse2(quote::quote! {physical {
            relation R(x:i32); relation S(x:i32);
            r = pipe rename R@0 {x -> x}; s = pipe rename S@1 {x -> x};
            m = pipe semi #choice [x] r,s;
            o = let canon project m keep {x};
            return o as answer(x);
        }})
        .unwrap();
        let target = compile(&source).unwrap();
        let recorded = target
            .structures
            .entries
            .iter()
            .find_map(|entry| match &entry.structure {
                structures::Structure::Host { keyed, .. } if keyed.provider.token == "keyset" => {
                    Some(keyed.provider.clone())
                }
                _ => None,
            });
        assert_eq!(
            recorded,
            Some(syn::parse_quote!(@keyset as ::cq_storage::dispatch::KeySet<_>))
        );
        let facts = contract(&target).unwrap();
        let printed: syn::Type = syn::parse2(printed).unwrap();
        assert!(
            facts
                .fallbacks
                .values()
                .any(|provider| provider.ty == printed)
        );
        // A spelling stating the choice itself is rejected.
        let mut spelled = target;
        for entry in &mut spelled.structures.entries {
            if let structures::Structure::Host { keyed, .. } = &mut entry.structure
                && keyed.provider.token == "keyset"
            {
                keyed.provider.ty = printed.clone();
            }
        }
        assert!(contract(&spelled).is_err());
    }
}

#[test]
fn independent_union_indices_share_canonical_rows() {
    let source: physical::Program = syn::parse_quote! {physical {
        relation L(a:i32); relation A(a:i32,b:i32); relation B(a:i32,b:i32);
        l = pipe rename L@0 {a -> x}; a = pipe rename A@1.0.0 {a -> x,b -> y}; b = pipe rename B@1.1.0 {a -> x,b -> y};
        d = let canon union a,b block @1 free[x] interface[y];
        j = pipe probe first [x] l,d; q = pipe project j keep {y}; out = let canon semi eager [y] q,d; return out as answer(y);
    }};
    let target = compile(&source).unwrap();
    let facts = contract(&target).unwrap();
    assert_eq!(facts.built["d"].len(), 2);
    assert_eq!(
        facts.retained["d"],
        syn::parse_quote!(@sort as ::cq_storage::dedup::Sort)
    );
    assert!(facts.filtered.contains_key("d"));
    let mut missing = target.clone();
    let position = missing.structures.entries.iter().position(|entry|entry.name=="d" && matches!(&entry.structure,structures::Structure::Built {keyed,..} if keyed.level_names()==vec![vec!["y".to_owned()]])).unwrap();
    missing.structures.entries.remove(position);
    assert!(check(&source, &missing).is_err());
}

#[test]
fn union_curried_indices_and_an_explicit_scan_keep_both_decisions() {
    let source: physical::Program = syn::parse_quote! {physical {
        relation L(a:i32,b:i32); relation A(a:i32,b:i32,c:i32); relation B(a:i32,b:i32,c:i32);
        l = pipe rename L@0 {a -> x,b -> y}; a = pipe rename A@1.0.0 {a -> x,b -> y,c -> z}; b = pipe rename B@1.1.0 {a -> x,b -> y,c -> z};
        d = let canon union a,b block @1 free[x,y] interface[z];
        p = pipe project l keep {x}; checked = pipe semi eager [x] p,d;
        c = pipe probe first [x] checked,l; j = pipe probe first [x,y] c,d;
        aux = let project d keep {x,y}; out = let canon semi eager [x,y] j,aux; result = let canon project out keep {x,z}; return result as answer(x,z);
    }};
    let target = compile(&source).unwrap();
    let facts = contract(&target).unwrap();
    assert!(facts.retained.contains_key("d"));
    assert!(facts.filtered.contains_key("d"));
    assert_eq!(facts.built["d"][0].0.len(), 2);
}

#[test]
fn proven_disjoint_union_rejects_cross_arm_canon_but_retains_final_canon() {
    let source: physical::Program = syn::parse_quote! {physical {
        relation R(a:i32); relation S(a:i32);
        rr = pipe rename R@0.0.0 {a -> x}; ss = pipe rename S@0.1.0 {a -> x};
        r = pipe point select rr if (x = 1); s = pipe point select ss if (x = 2);
        u = pipe union r,s block @0 free[] interface[x]; return canon u as answer(x);
    }};
    let target = compile(&source).unwrap();
    check(&source, &target).unwrap();
    let mut kept = source.clone();
    kept.program
        .definitions
        .iter_mut()
        .find(|d| d.name == "u")
        .unwrap()
        .canon = Some(Default::default());
    assert!(physical::contract::check(&kept).is_err());
}
#[test]
fn scenarios_discharge_every_physical_read_and_reject_changed_targets() {
    for case in scenarios::SCENARIOS {
        let source = syn::parse_str(case.physical).unwrap();
        let target = compile(&source).unwrap();
        check(&source, &target).unwrap();
        let mut changed = target;
        changed.physical.program.head.relation = syn::parse_quote!(mutated);
        assert!(check(&source, &changed).is_err());
    }
}

fn chain() -> physical::Program {
    syn::parse_quote! { physical {
        relation L(a:i32); relation R(a:i32,b:i32); relation S(b:i32);
        l = let rename L@0 {a -> a}; r = let rename R@1 {a -> a,b -> b}; s = pipe rename S@2 {b -> b};
        j = pipe probe first [a] l,r; x = pipe semi eager [a] j,l; p = let canon semi eager [b] x,s; return p as answer(a,b);
    } }
}

#[test]
fn concrete_types_and_preservation_are_checked_independently() {
    let source = chain();
    let target = compile(&source).unwrap();
    check(&source, &target).unwrap();
    let mut reordered = target.clone();
    reordered.structures.entries.reverse();
    check(&source, &reordered).unwrap();
    for replacement in [
        syn::parse_quote!(l = @hash_index as ::cq_storage::HashIndex<'_, _, _> [a];),
        syn::parse_quote!(l = @custom as Custom [a];),
    ] {
        let mut q = target.clone();
        q.structures.entries[0] = replacement;
        check(&source, &q).unwrap();
    }
    let mut q = target.clone();
    q.structures.entries.remove(1);
    assert!(check(&source, &q).is_err());
    let mut q = target.clone();
    q.structures
        .entries
        .push(syn::parse_quote!(s = @std_hash_set as ::std::collections::HashSet<_> [b];));
    assert!(check(&source, &q).is_err());
    let mut q = target;
    q.physical.program.declarations[0].value = Some(syn::parse_quote!(= (rows())));
    assert!(check(&source, &q).is_err());
}

#[test]
fn host_reads_record_fallback_types_for_every_read_kind() {
    for op in ["probe first [a] l,r", "semi eager [a] l,r", "merge[a] l,r"] {
        let p: physical::Program = syn::parse_str(&format!("physical {{ relation L(a:i32); relation R(a:i32); l = pipe rename L@0 {{a -> a}}; r = pipe rename R@1 {{a -> a}}; j = let canon {op}; return j as answer(a); }}")).unwrap();
        let target = compile(&p).unwrap();
        assert!(
            target
                .structures
                .entries
                .iter()
                .all(|e| !matches!(e.structure, structures::Structure::Built { .. }))
        );
        check(&p, &target).unwrap();
    }
    let p: physical::Program = syn::parse_quote!(physical { relation R(a:i32); r = pipe rename R@0 {a -> x}; w = let canon point select r if (x = 2); return w as answer(x); });
    let target = compile(&p).unwrap();
    assert!(
        target
            .structures
            .entries
            .iter()
            .all(|e| !matches!(e.structure, structures::Structure::Built { .. }))
    );
}

#[test]
fn const_supplies_on_host_and_materialized_inputs_never_build() {
    for mode in ["pipe", "let"] {
        let p: physical::Program = syn::parse_str(&format!(
            "physical {{ relation R(a:i32); r = {mode} rename R@0 {{a -> x}}; w = let canon point select r if (x = 2); return w as answer(x); }}"
        )).unwrap();
        let target = compile(&p).unwrap();
        assert!(
            target
                .structures
                .entries
                .iter()
                .all(|e| !matches!(e.structure, structures::Structure::Built { .. }))
        );
        check(&p, &target).unwrap();
        let mut unnecessary = target;
        unnecessary
            .structures
            .entries
            .push(syn::parse_quote!(r = @hash_index as ::cq_storage::HashIndex<'_, _, _> [x];));
        assert!(check(&p, &unnecessary).is_err());
    }
}

#[test]
fn ordered_read_provider_capabilities_are_rustc_obligations() {
    let p: physical::Program = syn::parse_quote!(physical {
        relation L(a:i32); relation R(a:i32);
        l = pipe rename L@0 {a -> x}; r = let rename R@1 {a -> x};
        w = let canon merge[x] l,r; return w as answer(x);
    });
    let target = compile(&p).unwrap();
    assert_eq!(
        built_entries(&target),
        syn::parse_quote!(structures { providers { btree_map_grouped = ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> { reads [exact, membership, ordered], supports [keys], build typed, shared }; } r = @btree_map_grouped as ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> [x] supports [keys]; })
    );
    let mut q = target.clone();
    q.structures.entries[0] =
        syn::parse_quote!(r = @hash_index as ::cq_storage::HashIndex<'_, _, _> [x];);
    check(&p, &q).unwrap();
    // The recorded key remains a structural obligation regardless of provider.
    let mut changed_key = target;
    changed_key.structures.entries[0] = syn::parse_quote!(r = @custom as Custom [];);
    assert!(check(&p, &changed_key).is_err());
}

#[test]
fn compatible_membership_chains_use_recorded_trie_levels() {
    let p: physical::Program = syn::parse_quote!(physical {
        relation L(a:i32,b:i32); relation R(a:i32,b:i32);
        l = pipe rename L@0 {a -> a,b -> b}; r = let rename R@1 {a -> a,b -> b};
        p = pipe project l keep {a}; x = pipe semi eager [a] p,r;
        j = let canon probe first [a,b] r,r;
        k = let canon semi eager [a] j,x; return k as answer(a,b);
    });
    // x is a keyed intermediate and must be retained.
    let mut p = p;
    p.program.definitions[3].mode = syn::parse_quote!(let);
    let target = compile(&p).unwrap();
    check(&p, &target).unwrap();
    let entry = target
        .structures
        .entries
        .iter()
        .position(|e| e.name == "r")
        .unwrap();
    assert_eq!(
        target.structures.entries[entry],
        syn::parse_quote!(r = @trie(@hash_index as ::cq_storage::HashIndex<'_, _, _>) as ::cq_storage::Trie<'_, _, ::cq_storage::HashIndex<'_, _, _>> [a][b];)
    );
    let mut q = target;
    q.structures.entries[entry] =
        syn::parse_quote!(r = @hash_index as ::cq_storage::HashIndex<'_, _, _> [a,b];);
    assert!(check(&p, &q).is_err());
}

#[test]
fn smallest_shared_full_key_adapters_retain_one_seek_representation() {
    let source: physical::Program = syn::parse_quote! { physical {
        relation L(a:i64,b:i64); relation R(b:i64,c:i64); relation S(a:i64,c:i64);
        l = pipe rename L@0 {a -> a,b -> b};
        r = pipe rename R@1 {b -> b,c -> c};
        s = pipe rename S@2 {a -> a,c -> c};
        candidate = pipe probe smallest [b] l,r;
        result = let canon probe first [a,c] candidate,s;
        return result as answer(a,b,c);
    }};
    let target = compile(&source).unwrap();
    let facts = contract(&target).unwrap();
    assert!(
        !facts
            .membership
            .contains(&("s".into(), vec!["a".into(), "c".into()]))
    );
    let adapters: Vec<_> = facts
        .fallbacks
        .iter()
        .filter(|((name, _, key), _)| name == "s" && key == &["a", "c"])
        .map(|(_, ty)| ty)
        .collect();
    assert!(adapters.len() >= 2);
    assert!(
        adapters
            .iter()
            .all(|ty| **ty == syn::parse_quote!(@hash_index as ::cq_storage::HashIndex<'_, _, _>))
    );
}

#[test]
fn smallest_candidate_let_levels_keep_independent_mutation_checks() {
    // Author the physical premise directly: adjacent reads introduce then
    // test one column. The fine-split source planner may choose other steps.
    let source: physical::Program = syn::parse_quote! { physical {
        relation L(a: i32, b: i32);
        relation A(a: i32, c: i32); relation B(b: i32, c: i32);
        l = pipe rename L@0 { a -> a, b -> b };
        a = let rename A@1 { a -> a, c -> c };
        b = let rename B@2 { b -> b, c -> c };
        p = pipe probe smallest [a] l,a;
        result = let canon semi eager [b,c] p,b;
        return result as answer(a,b,c);
    }};
    let target = compile(&source).unwrap();
    check(&source, &target).unwrap();
    let entry = target
        .structures
        .entries
        .iter()
        .position(|e| {
            matches!(&e.structure, structures::Structure::Built { keyed, .. } if keyed.levels.len() == 2)
        })
        .unwrap();
    let mut changed = target.clone();
    let structures::Structure::Built { keyed, .. } =
        &mut changed.structures.entries[entry].structure
    else {
        panic!()
    };
    keyed.levels.reverse();
    assert!(check(&source, &changed).is_err());
    let mut changed = target;
    let structures::Structure::Built { keyed, .. } =
        &mut changed.structures.entries[entry].structure
    else {
        panic!()
    };
    keyed.provider = cq_lang_structures::provider::base::instance(
        "std_hash_set",
        syn::parse_quote!(::std::collections::HashSet<_>),
    );
    changed
        .structures
        .declare(&[cq_lang_structures::provider::base::declaration(
            "std_hash_set",
        )])
        .unwrap();
    // The two recorded levels are authoritative; Rust checks the provider's
    // implementation of the required candidate access.
    check(&source, &changed).unwrap();
}

#[test]
fn mixed_smallest_selects_each_retained_projection_and_checks_mutations() {
    for (retained, host, prefix) in [("r", "s", "b"), ("s", "r", "a")] {
        let r = if retained == "r" {
            "r0 = pipe rename R@0 {b -> b,c -> c}; r = let project r0 keep {b,c};"
        } else {
            "r = pipe rename R@0 {b -> b,c -> c};"
        };
        let s = if retained == "s" {
            "s0 = pipe rename S@0 {a -> a,c -> c}; s = let project s0 keep {a,c};"
        } else {
            "s = pipe rename S@0 {a -> a,c -> c};"
        };
        let source: physical::Program = syn::parse_str(&format!(
            "physical {{
                relation L(a:i32,b:i32); relation R(b:i32,c:i32); relation S(a:i32,c:i32);
                l = pipe rename L@0 {{a -> a,b -> b}}; {r} {s}
                candidate = pipe probe smallest [b] l,r;
                checked = let canon semi eager [a,c] candidate,s;
                return checked as answer(a,b,c);
            }}"
        ))
        .unwrap();
        physical::contract::check(&source).unwrap();
        let target = compile(&source).unwrap();
        check(&source, &target).unwrap();
        assert_eq!(target.physical, source);
        assert_eq!(
            built_entries(&target),
            syn::parse_str::<structures::Structures>(&format!(
                "structures {{ providers {{
                    trie = ::cq_storage::Trie<'_, _, _> {{ reads [exact, membership], build typed, nested 2 }};
                    hash_set = ::cq_storage::HashSet<_> {{ reads [membership], build typed, accounting keys, shared }};
                }}
                {retained} = @trie(@hash_set as ::cq_storage::HashSet<_>)
                    as ::cq_storage::Trie<'_, _, ::cq_storage::HashSet<_>> [{prefix}][c]; }}"
            ))
            .unwrap()
        );
        assert_eq!(
            syn::parse2::<structures::Program>(target.to_token_stream()).unwrap(),
            target
        );

        let entry = target
            .structures
            .entries
            .iter()
            .position(|e| matches!(e.structure, structures::Structure::Built { .. }))
            .unwrap();
        let mut missing = target.clone();
        missing.structures.entries.remove(entry);
        assert!(contract(&missing).is_err());
        assert!(check(&source, &missing).is_err());
        for replacement in [
            format!("{retained} = @hash_index as ::cq_storage::HashIndex<'_, _, _> [{prefix},c];"),
            format!(
                "{retained} = @trie(@std_hash_set as ::std::collections::HashSet<_>) as ::cq_storage::Trie<'_, _, ::std::collections::HashSet<_>> [c][{prefix}];"
            ),
        ] {
            let mut changed = target.clone();
            changed.structures.entries[entry] = syn::parse_str(&replacement).unwrap();
            assert!(contract(&changed).is_err(), "{replacement}");
            assert!(check(&source, &changed).is_err(), "{replacement}");
        }
        let mut alternate_provider = target.clone();
        alternate_provider.structures.entries[entry] = syn::parse_str(&format!(
            "{retained} = @std_hash_set as ::std::collections::HashSet<_> [{prefix}][c];"
        ))
        .unwrap();
        // Provider names do not prove or disprove candidate access.
        check(&source, &alternate_provider).unwrap();
        for reader in [
            structures::ReaderRef::Candidates,
            structures::ReaderRef::Membership,
        ] {
            let mut missing = target.clone();
            missing.structures.entries.retain(|e| {
                !(e.name == host
                    && matches!(&e.structure, structures::Structure::Host { reader: r, .. } if r == &reader))
            });
            assert!(contract(&missing).is_err(), "{host}/{reader:?}");
            assert!(check(&source, &missing).is_err(), "{host}/{reader:?}");
        }
    }
}

#[test]
fn candidate_levels_coexist_with_payload_membership_and_ordered_reads() {
    let source: physical::Program = syn::parse_quote!(physical {
        relation L(a:i32,b:i32); relation R(b:i32,c:i32); relation S(a:i32,c:i32);
        l0 = pipe rename L@0 {a -> a,b -> b};
        r0 = pipe rename R@1 {b -> b,c -> c}; r = let project r0 keep {b,c};
        s = pipe rename S@2 {a -> a,c -> c};
        payload = pipe probe first [b] l0,r;
        l1 = pipe project payload keep {a,b};
        members = let semi eager [c] s,r;
        l2 = pipe semi eager [a] l1,members;
        ordered = let merge[b,c] r,r;
        l = pipe semi eager [b] l2,ordered;
        candidate = pipe probe smallest [b] l,r;
        checked = let canon semi eager [a,c] candidate,s;
        return checked as answer(a,b,c);
    });
    let target = compile(&source).unwrap();
    let mut reordered = target.clone();
    reordered.structures.entries.reverse();
    for target in [&target, &reordered] {
        check(&source, target).unwrap();
        let analysis = contract(target).unwrap();
        assert!(analysis.candidate_on("r", &["b".into()], "c").is_some());
        assert_eq!(
            analysis.built_on("r", &["b".into(), "c".into()]).unwrap().1,
            1
        );
    }
    // These layouts are necessary for coverage. Removing only the ordered
    // implementation is instead a Rust OrderedRead obligation: an opaque
    // candidate provider could lawfully implement both interfaces.
    for levels in ["b/c", "b", "c"] {
        let levels: Vec<Vec<String>> = levels
            .split('/')
            .map(|column| vec![column.to_owned()])
            .collect();
        let missing = target
            .structures
            .entries
            .iter()
            .position(|entry| {
                entry.name == "r"
                    && matches!(&entry.structure,
                structures::Structure::Built { keyed, .. } if keyed.level_names() == levels)
            })
            .unwrap();
        let mut changed = target.clone();
        changed.structures.entries.remove(missing);
        assert!(
            check(&source, &changed).is_err(),
            "missing read layout {levels:?}"
        );
    }
    // Without the dedicated ordered provider, the complete candidate key
    // still records coverage. Rust must establish Ordered access for its type.
    let mut shared_provider = target.clone();
    shared_provider.structures.entries.remove(2);
    check(&source, &shared_provider).unwrap();
    // Removing both complete-key records really does lose merge coverage.
    shared_provider.structures.entries.remove(0);
    assert!(check(&source, &shared_provider).is_err());
}

#[test]
fn nonemptiness_scans_rows_beside_a_nonempty_keyed_membership_index() {
    let p: physical::Program = syn::parse_quote! { physical {
        relation L(a: i32); relation R(a: i32);
        l = pipe rename L@0 { a -> x }; r = let rename R@1 { a -> x };
        unit = pipe unit;
        nonempty = pipe semi eager [] unit,r;
        members = pipe probe first [] nonempty,l;
        result = let canon semi eager [x] members,r;
        return result as answer(x);
    }};
    let target = compile(&p).unwrap();
    let facts = contract(&target).unwrap();
    assert!(facts.retained.contains_key("r"));
    assert_eq!(facts.built["r"].len(), 1);
    assert_eq!(facts.built["r"][0].0, vec![vec!["x".to_owned()]]);
    check(&p, &target).unwrap();
}

fn built_entries(target: &structures::Program) -> structures::Structures {
    let mut entries = target.structures.clone();
    entries
        .entries
        .retain(|e| matches!(e.structure, structures::Structure::Built { .. }));
    entries.declare(&[]).unwrap();
    entries
}

fn filtered_source() -> physical::Program {
    syn::parse_quote!(physical {
        relation L(k:i32); relation R(tag:StringCode,k:i32,value:i32,unused:i32);
        l = pipe rename L@0 {k -> k};
        r = pipe rename R@1 {tag -> tag,k -> k,value -> value,unused -> unused};
        selected = let point select r if (tag = "a");
        joined = pipe probe first [k] l,selected;
        keys = pipe project joined keep {k};
        checked = pipe semi eager [k] keys,selected;
        again = pipe probe first [k] checked,selected;
        out = let canon project again keep {k,value};
        return out as answer(k,value);
    })
}

#[test]
fn filtered_readers_share_one_projected_index_and_check_column_coverage() {
    let source = filtered_source();
    let target = compile(&source).unwrap();
    let analysis = contract(&target).unwrap();
    assert!(!analysis.retained.contains_key("selected"));
    assert_eq!(analysis.built["selected"].len(), 1);
    let entry = target
        .structures
        .entries
        .iter()
        .position(|e| e.name == "selected")
        .unwrap();
    let structures::Structure::Built { keyed, .. } = &target.structures.entries[entry].structure
    else {
        panic!("projected read must have a payload provider");
    };
    assert_eq!(
        keyed.filtered,
        Some(syn::parse_quote!(payload [k,value] constants {tag = "a"}))
    );
    assert_eq!(
        syn::parse2::<structures::Program>(target.to_token_stream()).unwrap(),
        target
    );
    for replacement in [
        syn::parse_quote!(selected = @hash_index as ::cq_storage::HashIndex<'_, _, _> [k] payload [k] constants {tag = "a"};),
        syn::parse_quote!(selected = @hash_index as ::cq_storage::HashIndex<'_, _, _> [value] payload [k,value] constants {tag = "a"};),
        syn::parse_quote!(selected = @hash_index as ::cq_storage::HashIndex<'_, _, _> [k] payload [k,value] constants {tag = "b"};),
        syn::parse_quote!(selected = @hash_index as ::cq_storage::HashIndex<'_, _, _> [k] payload [k,value,tag] constants {tag = "a"};),
    ] {
        let mut bad = target.clone();
        bad.structures.entries[entry] = replacement;
        assert!(check(&source, &bad).is_err());
        assert!(structures::check_rewrite(&target, &bad).is_err());
    }
    let mut bad = target.clone();
    bad.structures
        .entries
        .push(syn::parse_quote!(selected rows = @vec as ::std::vec::Vec<_>;));
    assert!(check(&source, &bad).is_err());
    let mut changed_occurrence = target.clone();
    let physical::Operator::Rename(rename) =
        &mut changed_occurrence.physical.program.definitions[1].operator
    else {
        unreachable!()
    };
    rename.occurrence = syn::parse_quote!(2);
    // The filtered layout still covers every read; correspondence must also
    // preserve the identity of the source occurrence supplying those rows.
    contract(&changed_occurrence).unwrap();
    assert!(check(&source, &changed_occurrence).is_err());
    assert!(structures::check_rewrite(&target, &changed_occurrence).is_err());
    let mut reordered = target.clone();
    reordered.physical.program.definitions.swap(0, 1);
    // Structures is representation-only, including definition order.
    assert!(check(&source, &reordered).is_err());
    assert!(structures::check_rewrite(&target, &reordered).is_err());
    let mut wrong_graph = target;
    wrong_graph.physical.program.definitions[2].operator =
        syn::parse_quote!(point select r if (tag = "b"));
    assert!(structures::check_rewrite(&reordered, &wrong_graph).is_err());
}

#[test]
fn scanned_selections_keep_rows_and_reject_a_missing_rows_decision() {
    let mut source = filtered_source();
    // The selection supplies outer rows too, so it needs a reusable row scan.
    source.program.definitions[3].operator = syn::parse_quote!(probe first [k] selected,l);
    let target = compile(&source).unwrap();
    let analysis = contract(&target).unwrap();
    assert!(analysis.retained.contains_key("selected"));
    assert!(!analysis.filtered.contains_key("selected"));
    let mut bad = target;
    bad.structures.entries.retain(|e| {
        e.name != "selected" || !matches!(e.structure, structures::Structure::Rows { .. })
    });
    assert!(check(&source, &bad).is_err());
}

#[test]
fn range_filters_project_only_columns_demanded_downstream() {
    let mut source = filtered_source();
    source.program.definitions[2].operator = syn::parse_quote!(range select r if (value > 0));
    source.program.definitions[7].operator = syn::parse_quote!(project again keep {k});
    source.program.head = syn::parse_quote!(answer(k));
    let target = compile(&source).unwrap();
    let analysis = contract(&target).unwrap();
    assert!(!analysis.retained.contains_key("selected"));
    assert_eq!(
        analysis.filtered["selected"],
        syn::parse_quote!(payload [k] constants {})
    );
    assert_eq!(analysis.layouts["joined"], ["k"]);
}

#[test]
fn filtered_payload_covers_rust_uses_and_keeps_opaque_environments() {
    for (guard, expected) in [
        ("(*unused > 0)", vec!["k", "unused", "value"]),
        ("(accept!(value))", vec!["k", "unused", "value"]),
    ] {
        let mut source = filtered_source();
        source.program.definitions.insert(
            4,
            syn::parse_str(&format!("guarded = pipe guard joined if {guard};")).unwrap(),
        );
        source.program.definitions[5].operator = syn::parse_quote!(project guarded keep {k});
        let target = compile(&source).unwrap();
        let analysis = contract(&target).unwrap();
        let payload: Vec<_> = analysis.filtered["selected"]
            .payload
            .columns
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(payload, expected);
    }
}

#[test]
fn a_later_merge_does_not_remove_an_earlier_scan_requirement() {
    let source: physical::Program = syn::parse_quote!(physical {
        relation R(a:i32);
        r0 = pipe rename R@0 {a -> a}; r = let point select r0 if (a = 1);
        rows = let canon project r keep {a};
        joined = pipe merge[a] r,rows;
        out = let canon project joined keep {a}; return out as answer(a);
    });
    let target = compile(&source).unwrap();
    let analysis = contract(&target).unwrap();
    assert!(analysis.physical.scanned.contains("r"));
    assert!(analysis.retained.contains_key("r"));
    let mut bad = target;
    bad.structures
        .entries
        .retain(|e| e.name != "r" || !matches!(e.structure, structures::Structure::Rows { .. }));
    assert!(check(&source, &bad).is_err());
}

#[test]
fn selection_records_context_before_choosing_dedup_types() {
    let source: physical::Program = syn::parse_quote! { physical {
        relation R(a:i32);
        r = let canon rename R@0 {a -> x};
        return dedup r as answer(*x % 2);
    }};
    let selected = cq_structure_selection::compile_with(
        &source,
        quote::quote!(consumer = canonical, host_set R discharged,),
    )
    .unwrap();
    let facts = contract(&selected).unwrap();
    assert!(facts.physical.consumer_canonical);
    assert!(facts.physical.discharged_host_sets.contains("R"));
    assert!(facts.canonical.is_empty());
    assert!(facts.head_dedup.is_empty());
    assert!(selected.physical.program.definitions[0].canon.is_none());
    check(&selected.physical, &selected).unwrap();
    assert!(check(&source, &selected).is_err());
    let context = cq_structure_selection::context(quote::quote!(
        host_set R discharged, positional(R), consumer = canonical,
    ))
    .unwrap();
    assert!(context.consumer_canonical);
    assert!(context.discharged_host_sets.contains("R"));
    let ordinary = compile(&source).unwrap();
    let empty = cq_structure_selection::compile_with(&source, Default::default()).unwrap();
    assert_eq!(
        ordinary.to_token_stream().to_string(),
        empty.to_token_stream().to_string()
    );
}

#[test]
fn redundant_context_packets_preserve_explicit_canon() {
    let source: physical::Program = syn::parse_quote! { physical {
        relation R(a:i32);
        host_set R discharged;
        consumer canonical;
        r = let canon rename R@0 {a -> x};
        return r as answer(*x % 2);
    }};
    let ordinary = compile(&source).unwrap();
    for arguments in [
        quote::quote!(consumer = canonical),
        quote::quote!(host_set R discharged),
        quote::quote!(host_set R discharged, consumer = canonical),
    ] {
        let selected = cq_structure_selection::compile_with(&source, arguments).unwrap();
        assert_eq!(selected.physical, source);
        assert_eq!(
            selected.to_token_stream().to_string(),
            ordinary.to_token_stream().to_string()
        );
        let facts = contract(&selected).unwrap();
        assert!(facts.canonical.contains_key("r"));
        assert!(facts.head_dedup.is_empty());
    }
}

#[test]
fn named_host_consumers_do_not_alias_reader_roles() {
    for name in ["scan", "candidates", "membership", "result"] {
        let result = syn::Ident::new(name, proc_macro2::Span::call_site());
        let source = syn::parse_quote! { physical {
            relation L(a:i32); relation R(a:i32,b:i32);
            l = pipe rename L@0 {a -> x};
            r = pipe rename R@1 {a -> x,b -> y};
            #result = let canon probe first [x] l,r;
            return #result as answer(x,y);
        }};
        let selected = compile(&source).unwrap();
        let facts = contract(&selected).unwrap();
        assert!(
            facts
                .fallbacks
                .contains_key(&("r".into(), structures::ReaderRef::Scan, vec![]))
        );
        assert!(facts.fallbacks.contains_key(&(
            "r".into(),
            structures::ReaderRef::Result(name.into()),
            vec!["x".into()]
        )));
        let printed = selected.to_token_stream();
        assert!(
            printed
                .to_string()
                .contains(&format!("host result ({name})"))
        );
        let reparsed = syn::parse2::<structures::Program>(printed).unwrap();
        assert_eq!(selected, reparsed);
        check(&source, &reparsed).unwrap();
    }
    assert!(
        syn::parse_str::<structures::Entry>("r host arbitrary = @reader as Reader [x];").is_err()
    );
    assert!(
        syn::parse_str::<structures::Entry>(
            "r host result(scan, candidates) = @reader as Reader [x];"
        )
        .is_err()
    );
}
