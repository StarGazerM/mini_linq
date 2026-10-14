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
            r = pipe rename rows@0 {x -> x}; p = exists project r keep {};
            out = pipe canon semi eager [] p,p; return out as answer();
        }
    }
    .collect();
    assert_eq!(answer, [()]);
}

#[test]
fn physical_and_structures_correspondence_reject_changes_in_meaning() {
    let source: cq_lang_physical::Program = syn::parse_quote!(physical {
        relation R(x:i32); r = pipe canon rename R@0 {x -> x}; return r as answer(x);
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
        r = rename R@0 {x -> x}; s = rename S@1 {y -> y}; j = natural_join r with s;
        output j as answer(x,y);
    });
    let mut target = source.clone();
    target.plan.definitions[2].operator = syn::parse_quote!(natural_join s with r);
    cq_lang_ra::check_rewrite(&source, &target).unwrap();
    target.plan.definitions[1].operator = syn::parse_quote!(rename R@0 {x -> y});
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
            l = pipe rename left@0 {x -> x}; r = pipe rename right@1 {x -> x};
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
            r = pipe rename rows@0 {x -> x};
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
        l = pipe rename L@0 {x -> x}; r = pipe rename R@1 {x -> x};
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

macro_rules! capture_plan {
    (pipeline {}; $($plan:tt)*) => {
        stringify!($($plan)*)
    };
}

#[test]
fn default_omits_unfiltered_reductions_and_opt_out_preserves_relational_plan() {
    use quote::ToTokens;

    let source: cq_lang_cq::Program = syn::parse_quote! {
        struct Q;
        relation R(a:i32,b:i32); relation S(b:i32,c:i32); relation T(c:i32,d:i32);
        answer(a,b,c,d) :- R(a,b), S(b,c), T(c,d);
    };
    let default = mini_linq::passes::plan! {
        pipeline { capture_plan };
        struct Q;
        relation R(a:i32,b:i32); relation S(b:i32,c:i32); relation T(c:i32,d:i32);
        answer(a,b,c,d) :- R(a,b), S(b,c), T(c,d);
    };
    let opted_out = mini_linq::passes::plan! {
        arguments { no_reduce };
        pipeline { capture_plan };
        struct Q;
        relation R(a:i32,b:i32); relation S(b:i32,c:i32); relation T(c:i32,d:i32);
        answer(a,b,c,d) :- R(a,b), S(b,c), T(c,d);
    };
    let normal = cq_lang_cq::normal::normalize(&source).unwrap();
    let ordered = cq_order::compile(
        &normal,
        &cq_order::Options {
            heuristic: cq_order::Heuristic::Filtered,
            split: cq_order::Split::Coarse,
        },
    )
    .unwrap();
    let prior = cq_elaborate::compile(&ordered).unwrap();
    let reduced = cq_reduce::compile(
        &ordered,
        &cq_reduce::Options {
            semijoins: cq_reduce::Reductions::Hypergraph,
        },
    )
    .unwrap();
    let expected = cq_elaborate::compile(&reduced).unwrap();
    cq_elaborate::check(&reduced, &expected).unwrap();
    let expected = cq_elaborate::simplify_reductions(&expected).unwrap();
    let default: cq_lang_ra::Program = syn::parse_str(default).unwrap();
    let opted_out: cq_lang_ra::Program = syn::parse_str(opted_out).unwrap();
    assert_eq!(
        opted_out, prior,
        "no_reduce must retain the prior emitted IR"
    );
    assert_eq!(
        default, expected,
        "the default must run Hypergraph reduction"
    );
    assert_eq!(
        default, opted_out,
        "unfiltered acyclic inputs need no optional reductions"
    );
    assert!(!default.to_token_stream().to_string().contains("semijoin"));
}

#[test]
fn default_reduction_and_opt_out_agree_on_host_and_fallback_inputs() {
    let left = [(1, 10), (1, 10), (2, 20), (3, 30)];
    let rows = [(10, 100), (10, 100), (30, 300), (40, 400)];
    let host = std::collections::HashMap::from([
        ((10,), vec![(100,), (100,)]),
        ((30,), vec![(300,)]),
        ((40,), vec![(400,)]),
    ]);
    macro_rules! check {
        ($right:ident) => {{
            let default: std::collections::BTreeSet<_> = mini_linq::mini_linq! {
                struct Q; relation left(a:i32,b:i32); relation $right(b:i32,c:i32);
                answer(a,c) :- left(a,b), $right(b,c);
            }
            .map(|(a, c)| (*a, *c))
            .collect();
            let opted_out: std::collections::BTreeSet<_> = mini_linq::compile! {
                pipeline { plan = mini_linq::passes::plan(no_reduce); }
                struct Q; relation left(a:i32,b:i32); relation $right(b:i32,c:i32);
                answer(a,c) :- left(a,b), $right(b,c);
            }
            .map(|(a, c)| (*a, *c))
            .collect();
            assert_eq!(
                default,
                std::collections::BTreeSet::from([(1, 100), (3, 300)])
            );
            assert_eq!(default, opted_out);
        }};
    }
    check!(rows);
    check!(host);
}

#[test]
fn default_reduces_cyclic_block_scopes_and_preserves_answers() {
    fn has_reduction(steps: &[cq_lang_vo::Step]) -> bool {
        steps.iter().any(|step| match step {
            cq_lang_vo::Step::Reduce(..) => true,
            cq_lang_vo::Step::Block(block, _) => block
                .branches
                .iter()
                .any(|branch| has_reduction(&branch.steps)),
            _ => false,
        })
    }
    macro_rules! assert_reduced {
        ($($query:tt)*) => {{
            let default = mini_linq::passes::plan! {
                pipeline { capture_plan };
                $($query)*
            };
            let opted_out = mini_linq::passes::plan! {
                arguments { no_reduce };
                pipeline { capture_plan };
                $($query)*
            };
            let default: cq_lang_ra::Program = syn::parse_str(default).unwrap();
            let opted_out: cq_lang_ra::Program = syn::parse_str(opted_out).unwrap();
            let source: cq_lang_cq::Program = syn::parse_quote! { $($query)* };
            let normal = cq_lang_cq::normal::normalize(&source).unwrap();
            let ordered = cq_order::compile(&normal, &cq_order::Options {
                heuristic: cq_order::Heuristic::Filtered,
                split: cq_order::Split::Coarse,
            }).unwrap();
            let options = cq_reduce::Options {

                semijoins: cq_reduce::Reductions::Hypergraph,
            };
            let reduced = cq_reduce::compile(&ordered, &options).unwrap();
            cq_reduce::check(&ordered, &reduced, &options).unwrap();
            assert!(has_reduction(&reduced.valuation.steps));
            let expected = cq_elaborate::compile(&reduced).unwrap();
            cq_elaborate::check(&reduced, &expected).unwrap();
            let expected = cq_elaborate::simplify_reductions(&expected).unwrap();
            assert_eq!(default, cq_physical_lowering::orient(&expected).unwrap());
            assert_eq!(opted_out, cq_physical_lowering::orient(&cq_elaborate::compile(&ordered).unwrap()).unwrap());
            cq_lang_ra::check_rewrite(&opted_out, &default).unwrap();
        }};
    }
    assert_reduced! {
        struct Cycle;
        relation R(a:i32,b:i32); relation S(b:i32,c:i32);
        relation T(a:i32,c:i32); relation U(a:i32); relation V(a:i32);
        answer(a,b,c) :- R(a,b), S(b,c), T(a,c), {U(a),V(a)} | {U(a)};
    }
    assert_reduced! {
        struct Branch;
        relation R(a:i32,b:i32); relation S(b:i32,c:i32);
        relation T(a:i32,c:i32); relation U(a:i32); relation V(a:i32);
        answer(a) :- {
            R(a,b), S(b,c), T(a,c), {U(a),V(a)} | {U(a)}
        } | {U(a)};
    }
}

#[test]
fn total_string_filters_and_derived_country_domains_execute_exactly() {
    let parts = [
        (1, "green brass".to_owned()),
        (2, "blue".to_owned()),
        (3, "green dangling".to_owned()),
    ];
    let lines = [(1,), (1,), (2,)];
    let actual: std::collections::BTreeSet<_> = mini_linq::mini_linq! {
        struct Q; relation parts(key:i32,name:String); relation lines(part:i32);
        answer(key) :- lines(key), parts(key,name), if name.contains("green");
    }
    .map(|(key,)| *key)
    .collect();
    assert_eq!(actual, [1].into());
    fn green(name: &str) -> bool {
        name.contains("green")
    }
    let asserted: std::collections::BTreeSet<_> = mini_linq::mini_linq! {
        struct Q; relation parts(key:i32,name:String); relation lines(part:i32);
        answer(key) :- lines(key), parts(key,name), #[total] if green(name);
    }
    .map(|(key,)| *key)
    .collect();
    assert_eq!(asserted, actual);
    let countries = [
        (1, "FRANCE".to_owned()),
        (2, "GERMANY".to_owned()),
        (3, "CANADA".to_owned()),
    ];
    let links = [(1, 2), (2, 1), (1, 1), (3, 2), (99, 1)];
    let pairs: std::collections::BTreeSet<_> = mini_linq::mini_linq! {
        struct Q; relation countries(key:i32,name:String); relation links(a:i32,b:i32);
        answer(a,b) :- links(a,b), countries(a,left), countries(b,right),
            if (left == "FRANCE" && right == "GERMANY") || (left == "GERMANY" && right == "FRANCE");
    }
    .map(|(a, b)| (*a, *b))
    .collect();
    assert_eq!(pairs, [(1, 2), (2, 1)].into());
}

#[test]
fn unrecognized_partial_string_guard_does_not_run_on_dangling_parts() {
    let parts = [(1, "g".to_owned()), (2, String::new())];
    let lines = [(1,)];
    let actual: Vec<_> = mini_linq::mini_linq! {
        struct Q; relation parts(key:i32,name:String); relation lines(part:i32);
        answer(key) :- lines(key), parts(key,name), if name.as_bytes()[0] == b'g';
    }
    .map(|(key,)| *key)
    .collect();
    assert_eq!(actual, [1]);
}

#[test]
fn total_allowlist_preserves_macro_forwarded_type_and_name_hygiene() {
    macro_rules! filtered {
        ($ty:ty; $rows:ident; $name:ident) => {
            mini_linq::mini_linq! {
                struct Q; relation $rows(name:$ty);
                answer($name) :- $rows($name), if $name.contains("green");
            }
        };
    }
    let rows = [("green".to_owned(),), ("blue".to_owned(),)];
    let answer: Vec<_> = filtered!(String; rows; name).collect();
    assert_eq!(answer.len(), 1);
    assert_eq!(answer[0].0, "green");
}

#[test]
fn total_identifier_remains_available_in_let_bindings_and_guards() {
    let rows = [(2, 3), (-2, 1)];
    let actual: Vec<_> = mini_linq::mini_linq! {
        struct Q; relation rows(x:i32,y:i32);
        answer(total) :- rows(x,y), let total:i32 = *x + *y, if *total > 0;
    }
    .map(|(total,)| total)
    .collect();
    assert_eq!(actual, [5]);
    let total = true;
    let y = true;
    let actual: Vec<_> = mini_linq::mini_linq! {
        struct Q; relation rows(x:i32,y:i32);
        answer(x) :- rows(x,_), if total && y;
    }
    .map(|(x,)| *x)
    .collect();
    assert_eq!(actual, [2, -2]);
    let actual: Vec<_> = mini_linq::mini_linq! {
        struct Q; relation rows(x:i32,y:i32);
        answer(total) :- rows(total,_), #[total] if total.is_positive();
    }
    .map(|(total,)| *total)
    .collect();
    assert_eq!(actual, [2]);
}
