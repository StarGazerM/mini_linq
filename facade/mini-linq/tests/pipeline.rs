//! Public pipeline boundaries, argument forwarding and language rewrites.
#[test]
fn defaults_and_surface_expansion_before_the_cq_contract_agree() {
    let rows = [(1,), (1,), (2,)];
    let default: Vec<_> =
        mini_linq::mini_linq! { struct Q; relation rows(x:i32); answer(x) :- rows(x); }.collect();
    let surfaced: Vec<_> = mini_linq::compile! {
        pipeline { surface = [core_pass_fixtures::expand_identity(expected)]; }
        struct Q; relation rows(x:i32); answer(x) :- identity!{rows(x)};
    }
    .collect();
    assert_eq!(default, surfaced);
}

#[test]
fn exists_retains_nonemptiness_and_consumes_its_entire_source() {
    let seen = std::cell::Cell::new(0);
    let retained: cq_storage::Exists = [(), (), ()]
        .into_iter()
        .inspect(|_| seen.set(seen.get() + 1))
        .collect();
    assert_eq!(seen.get(), 3);
    assert_eq!(retained.iter().count(), 1);
    let empty: cq_storage::Exists = std::iter::empty().collect();
    assert_eq!(empty.iter().count(), 0);
    let rows = [(1,), (2,), (2,)];
    let answer: Vec<_> = mini_linq::passes::select! {
        pipeline { mini_linq::passes::print_pull };
        physical { relation rows(x:i32);
            r = pipe rename rows {x -> x}; p = exists project r keep {};
            out = pipe canon semi eager [] p,p; return out as answer();
        }
    }
    .collect();
    assert_eq!(answer, [()]);
}

#[test]
fn physical_and_structures_correspondence_reject_changes_in_meaning() {
    let source: cq_lang_physical::Program = syn::parse_quote!(physical {
        relation R(x:i32); r = pipe canon rename R {x -> x}; return r as answer(x);
    });
    let mut changed = source.clone();
    changed.program.head.relation = syn::parse_quote!(wrong);
    assert!(cq_lang_physical::check_rewrite(&source, &changed).is_err());
    let structured = cq_structure_selection::compile(&source).unwrap();
    let mut changed = structured.clone();
    changed.physical = changed.physical.clone();
    changed.physical.program.head.relation = syn::parse_quote!(wrong);
    assert!(cq_lang_structures::check_rewrite(&structured, &changed).is_err());
    let mut changed = structured.clone();
    changed
        .structures
        .entries
        .retain(|e| !matches!(e.structure, cq_lang_structures::Structure::Dedup { .. }));
    assert!(cq_lang_structures::check_rewrite(&structured, &changed).is_err());
}

#[test]
fn ra_rewrites_use_the_existing_tableau_check() {
    let source: cq_lang_ra::Program = syn::parse_quote!(relational {
        relation R(x:i32); relation S(y:i32);
        r = rename R {x -> x}; s = rename S {y -> y}; j = natural_join r with s;
        output j as answer(x,y);
    });
    let mut target = source.clone();
    target.plan.definitions[2].operator = syn::parse_quote!(natural_join s with r);
    cq_lang_ra::check_rewrite(&source, &target).unwrap();
    target.plan.definitions[1].operator = syn::parse_quote!(rename R {x -> y});
    assert!(cq_lang_ra::check_rewrite(&source, &target).is_err());
}

#[test]
fn lazy_semijoin_state_stops_at_a_match_and_keeps_prior_keys() {
    let visits = std::cell::Cell::new(0);
    let state: cq_storage::dispatch::KeySet<_, false> = cq_storage::Build::build(
        [(1, ()), (2, ()), (3, ())]
            .into_iter()
            .inspect(|_| visits.set(visits.get() + 1)),
    );
    assert!(cq_storage::ExactMembership::contains(&state, &2));
    assert_eq!(visits.get(), 2);
    assert!(cq_storage::ExactMembership::contains(&state, &1));
    assert_eq!(visits.get(), 2);
    assert!(!cq_storage::ExactMembership::contains(&state, &9));
    assert_eq!(visits.get(), 3);
    assert!(cq_storage::ExactMembership::contains(&state, &3));
    let left = [(1,), (2,)];
    let right = [(2,), (3,)];
    let answer: Vec<_> = mini_linq::passes::select! {
        pipeline { mini_linq::passes::print_pull };
        physical { relation left(x:i32); relation right(x:i32);
            l = pipe rename left {x -> x}; r = pipe rename right {x -> x};
            out = pipe canon semi lazy [x] l,r; return out as answer(x);
        }
    }
    .collect();
    assert_eq!(answer, [(&2,)]);
}

#[test]
fn point_fallback_types_use_logical_keys_while_native_reads_keep_declaration_order() {
    let rows = [(1, 2), (3, 4), (1, 2)];
    let answer: Vec<_> = mini_linq::mini_linq! {
        struct Q; relation rows(first:i32, second:i32);
        answer(z,a) :- rows(z,a), if *z == 1 && *a == 2;
    }
    .collect();
    assert_eq!(answer, [(&1, &2)]);
    let grouped = std::collections::BTreeMap::from([((1, 2), vec![(), ()]), ((3, 4), vec![()])]);
    let answer: Vec<_> = mini_linq::mini_linq! {
        struct Q; relation grouped(first:i32, second:i32);
        answer(z,a) :- grouped(z,a), if *z == 1 && *a == 2;
    }
    .collect();
    assert_eq!(answer, [(&1, &2)]);
}

#[test]
fn a_shared_host_point_read_cannot_supply_the_stream_fallback_type() {
    let rows = [(1,), (1,), (2,)];
    let answer: Vec<_> = mini_linq::passes::select! {
        pipeline { mini_linq::passes::print_pull };
        physical { relation rows(x:i32);
            r = pipe rename rows {x -> x};
            a_point = pipe point select r if (x = 1);
            z_probe = pipe canon probe first [x] a_point,r;
            return z_probe as answer(x);
        }
    }
    .collect();
    assert_eq!(answer, [(&1,)]);
}

#[test]
fn ordered_host_fallback_records_must_be_ordered_admissible() {
    let source: cq_lang_physical::Program = syn::parse_quote!(physical {
        relation L(x:i32); relation R(x:i32);
        l = pipe rename L {x -> x}; r = pipe rename R {x -> x};
        out = pipe canon merge[x] l,r; return out as answer(x);
    });
    let structured = cq_structure_selection::compile(&source).unwrap();
    let mut invalid = structured.clone();
    let entry = invalid.structures.entries.iter_mut().find(|e| matches!(&e.structure, cq_lang_structures::Structure::Host { reader, .. } if reader == "out")).unwrap();
    if let cq_lang_structures::Structure::Host { keyed, .. } = &mut entry.structure {
        keyed.ty = syn::parse_quote!(::std::collections::HashSet<_>);
    }
    assert!(cq_lang_structures::check_rewrite(&structured, &invalid).is_err());
}
