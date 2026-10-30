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
fn hand_lookup_expand_and_fine_orders_preserve_default_answers() {
    let seed = [(1,), (2,), (3,)];
    let r = [(1, 10, 100), (1, 11, 101), (2, 20, 200), (2, 20, 200)];
    let s = [(1, 10, 1000), (1, 12, 1200), (2, 20, 2000)];
    let default: std::collections::BTreeSet<_> = mini_linq::mini_linq! {
        struct Q;
        relation seed(a:i32); relation r(a:i32,x:i32,y:i32); relation s(a:i32,x:i32,z:i32);
        answer(a,x,y,z) :- seed(a), r(a,x,y), s(a,x,z);
    }
    .map(|(a, x, y, z)| (*a, *x, *y, *z))
    .collect();
    let lookup: std::collections::BTreeSet<_> = mini_linq::compile! {
        pipeline { plan = mini_linq::passes::plan(no_reduce, order(seed, r@1(a), s@2(a), r@1, s@2)); }
        struct Q;
        relation seed(a:i32); relation r(a:i32,x:i32,y:i32); relation s(a:i32,x:i32,z:i32);
        answer(a,x,y,z) :- seed(a), r(a,x,y), s(a,x,z);
    }.map(|(a,x,y,z)| (*a,*x,*y,*z)).collect();
    let fine: std::collections::BTreeSet<_> = mini_linq::compile! {
        pipeline { plan = mini_linq::passes::plan(no_reduce, order(seed, r@1(a,x), s@2(a,x), r@1, s@2)); }
        struct Q;
        relation seed(a:i32); relation r(a:i32,x:i32,y:i32); relation s(a:i32,x:i32,z:i32);
        answer(a,x,y,z) :- seed(a), r(a,x,y), s(a,x,z);
    }.map(|(a,x,y,z)| (*a,*x,*y,*z)).collect();
    assert_eq!(
        default,
        std::collections::BTreeSet::from([(1, 10, 100, 1000), (2, 20, 200, 2000)])
    );
    assert_eq!(lookup, default);
    assert_eq!(fine, default);
    // An advised schedule may root at a non-written occurrence; the written
    // outer `seed` is then a single bound read and `s` a lookup before its expansion.
    let rooted: std::collections::BTreeSet<_> = mini_linq::compile! {
        pipeline { plan = mini_linq::passes::plan(no_reduce, order(r@1, seed@0, s@2(a, x), s@2)); }
        struct Q;
        relation seed(a:i32); relation r(a:i32,x:i32,y:i32); relation s(a:i32,x:i32,z:i32);
        answer(a,x,y,z) :- seed(a), r(a,x,y), s(a,x,z);
    }
    .map(|(a, x, y, z)| (*a, *x, *y, *z))
    .collect();
    assert_eq!(rooted, default);
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
        pipeline { mini_linq::passes::sequence, mini_linq::passes::sequential, mini_linq::passes::translate_pull };
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
        pipeline { mini_linq::passes::sequence, mini_linq::passes::sequential, mini_linq::passes::translate_pull };
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
        answer(z,a) :- rows(z,a), where z == 1 && a == 2;
    }
    .collect();
    assert_eq!(answer, [(&1, &2)]);
    let grouped = std::collections::BTreeMap::from([((1, 2), vec![(), ()]), ((3, 4), vec![()])]);
    let answer: Vec<_> = mini_linq::mini_linq! {
        struct Q; relation grouped(first:i32, second:i32);
        answer(z,a) :- grouped(z,a), where z == 1 && a == 2;
    }
    .collect();
    assert_eq!(answer, [(&1, &2)]);
}

#[test]
fn a_shared_host_point_read_cannot_supply_the_stream_fallback_type() {
    let rows = [(1,), (1,), (2,)];
    let answer: Vec<_> = mini_linq::passes::select! {
        pipeline { mini_linq::passes::sequence, mini_linq::passes::sequential, mini_linq::passes::translate_pull };
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
fn ordered_host_fallback_capabilities_are_rustc_obligations() {
    let source: cq_lang_physical::Program = syn::parse_quote!(physical {
        relation L(x:i32); relation R(x:i32);
        l = pipe rename L@0 {x -> x}; r = pipe rename R@1 {x -> x};
        out = pipe canon merge[x] l,r; return out as answer(x);
    });
    let structured = cq_structure_selection::compile(&source).unwrap();
    let mut alternate_provider = structured.clone();
    let entry = alternate_provider.structures.entries.iter_mut().find(|e| matches!(&e.structure, cq_lang_structures::Structure::Host { reader, .. } if reader.result().is_some_and(|name| name == "out"))).unwrap();
    if let cq_lang_structures::Structure::Host { keyed, .. } = &mut entry.structure {
        keyed.ty = syn::parse_quote!(::std::collections::HashSet<_>);
    }
    // The record preserves the ordered read and key. Rust, not syn::Type
    // inspection, checks whether the chosen provider implements that access.
    cq_lang_structures::check_rewrite(&structured, &alternate_provider).unwrap();
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

/// Advice replaces the Masks reduction `S ⋉ T` by `T ⋉ S` as recorded RA
/// semijoins, and both plans return the written answers.
#[test]
fn reduce_and_skip_advice_rewrite_reductions_and_preserve_answers() {
    let default = mini_linq::passes::plan! {
        pipeline { capture_plan };
        struct Q;
        relation R(a:i32,b:i32); relation S(b:i32,c:i32); relation T(c:i32,d:i32);
        answer(a,c) :- R(a,b), S(b,c), T(c,7);
    };
    let advised = mini_linq::passes::plan! {
        arguments { skip(S with T), reduce(T with S) };
        pipeline { capture_plan };
        struct Q;
        relation R(a:i32,b:i32); relation S(b:i32,c:i32); relation T(c:i32,d:i32);
        answer(a,c) :- R(a,b), S(b,c), T(c,7);
    };
    let semijoins = |plan: &str| {
        let plan: cq_lang_ra::Program = syn::parse_str(plan).unwrap();
        plan.plan
            .definitions
            .iter()
            .filter(|d| matches!(d.operator, cq_lang_ra::Operator::Semijoin(..)))
            .count()
    };
    assert_eq!(semijoins(default), 1);
    assert_eq!(semijoins(advised), 1);
    assert_ne!(default, advised);
    let r = [(1, 10), (2, 20), (3, 30)];
    let s = [(10, 100), (20, 200), (30, 300)];
    let t = [(100, 7), (200, 8), (300, 7)];
    let written: std::collections::BTreeSet<_> = mini_linq::mini_linq! {
        struct Q; relation r(a:i32,b:i32); relation s(b:i32,c:i32); relation t(c:i32,d:i32);
        answer(a,c) :- r(a,b), s(b,c), t(c,7);
    }
    .map(|(a, c)| (*a, *c))
    .collect();
    let advised: std::collections::BTreeSet<_> = mini_linq::compile! {
        pipeline { plan = mini_linq::passes::plan(skip(s with t), reduce(t with s)); }
        struct Q; relation r(a:i32,b:i32); relation s(b:i32,c:i32); relation t(c:i32,d:i32);
        answer(a,c) :- r(a,b), s(b,c), t(c,7);
    }
    .map(|(a, c)| (*a, *c))
    .collect();
    assert_eq!(
        written,
        std::collections::BTreeSet::from([(1, 100), (3, 300)])
    );
    assert_eq!(advised, written);
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
                heuristic: cq_order::Heuristic::Written,
                split: cq_order::Split::Coarse,
            }).unwrap();
            let options = cq_reduce::Options {
                semijoins: cq_reduce::Reductions::Masks,
            };
            let reduced = cq_reduce::compile(&ordered, &options).unwrap();
            cq_reduce::check(&ordered, &reduced, &options).unwrap();
            assert!(has_reduction(&reduced.valuation.steps));
            let expected = cq_elaborate::compile(&reduced).unwrap();
            cq_elaborate::check(&reduced, &expected).unwrap();
            assert_eq!(default, expected);
            assert_eq!(opted_out, cq_elaborate::compile(&ordered).unwrap());
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
fn opaque_string_filters_and_derived_country_domains_execute_exactly() {
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
    let custom: std::collections::BTreeSet<_> = mini_linq::mini_linq! {
        struct Q; relation parts(key:i32,name:String); relation lines(part:i32);
        answer(key) :- lines(key), parts(key,name), if green(name);
    }
    .map(|(key,)| *key)
    .collect();
    assert_eq!(custom, actual);
    let countries = [
        (1, "FRANCE".to_owned()),
        (2, "GERMANY".to_owned()),
        (3, "CANADA".to_owned()),
    ];
    let links = [(1, 2), (2, 1), (1, 1), (3, 2), (99, 1)];
    let pairs: std::collections::BTreeSet<_> = mini_linq::mini_linq! {
        struct Q; relation countries(key:i32,name:String); relation links(a:i32,b:i32);
        answer(a,b) :- links(a,b), countries(a,left), countries(b,right),
            where (left == "FRANCE" && right == "GERMANY") || (left == "GERMANY" && right == "FRANCE");
    }
    .map(|(a, b)| (*a, *b))
    .collect();
    assert_eq!(pairs, [(1, 2), (2, 1)].into());
}

#[test]
fn opaque_total_string_guard_handles_dangling_parts() {
    let parts = [(1, "g".to_owned()), (2, String::new())];
    let lines = [(1,)];
    let actual: Vec<_> = mini_linq::mini_linq! {
        struct Q; relation parts(key:i32,name:String); relation lines(part:i32);
        answer(key) :- lines(key), parts(key,name), if name.as_bytes().first() == Some(&b'g');
    }
    .map(|(key,)| *key)
    .collect();
    assert_eq!(actual, [1]);
}

#[test]
fn opaque_guard_preserves_macro_forwarded_type_and_name_hygiene() {
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
        answer(total) :- rows(x,y), let total:i32 = *x + *y, where total > 0;
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
        answer(total) :- rows(total,_), if total.is_positive();
    }
    .map(|(total,)| *total)
    .collect();
    assert_eq!(actual, [2]);
}

#[test]
fn cq_negation_excludes_matching_keys_with_constants_and_wildcards() {
    let rows = [(1,), (2,), (3,), (4,), (4,)];
    let blocked = [(1, 7, 10), (1, 7, 11), (2, 8, 10), (3, 7, 12)];
    let actual: std::collections::BTreeSet<_> = mini_linq::mini_linq! {
        struct Q;
        relation rows(key:i32);
        relation blocked(key:i32,kind:i32,payload:i32);
        answer(key) :- rows(key), !blocked(key,7,_);
    }
    .map(|(key,)| *key)
    .collect();
    assert_eq!(actual, [2, 4].into());
}

#[test]
fn cq_negation_respects_repeated_variables() {
    let rows = [(1,), (2,), (3,)];
    let blocked = [(1, 2), (2, 2), (3, 1)];
    let actual: std::collections::BTreeSet<_> = mini_linq::mini_linq! {
        struct Q;
        relation rows(key:i32);
        relation blocked(left:i32,right:i32);
        answer(key) :- rows(key), !blocked(key,key);
    }
    .map(|(key,)| *key)
    .collect();
    assert_eq!(actual, [1, 3].into());
}

#[test]
fn cq_negation_normalizes_raw_relations_and_repeated_variables() {
    let rows = [(1,), (2,), (3,), (4,), (4,)];
    let blocked = [(1, 2, 7, 9), (2, 2, 7, 8), (3, 3, 8, 9)];
    let expected: std::collections::BTreeSet<_> = rows
        .iter()
        .filter(|(x,)| {
            !blocked
                .iter()
                .any(|(a, b, kind, _)| a == x && b == x && *kind == 7)
        })
        .copied()
        .collect();
    macro_rules! query {
        ($relation:ident, $first:ident, $second:ident) => {
            mini_linq::compile! {
                pipeline {}
                struct Raw;
                relation r#A(x:i32) = (rows);
                relation r#S(a:i32,b:i32,kind:i32,payload:i32) = (blocked);
                relation r#answer(x:i32);
                answer(*x) :- A(x), !$relation($first,$second,7,_);
            }
            .collect::<std::collections::BTreeSet<_>>()
        };
    }
    assert_eq!(query!(S, r#x, x), expected);
    assert_eq!(query!(r#S, x, r#x), expected);
}

#[test]
fn cq_negation_of_an_empty_relation_retains_every_key() {
    let rows = [(1,), (2,), (2,)];
    let blocked: [(i32,); 0] = [];
    let actual: std::collections::BTreeSet<_> = mini_linq::mini_linq! {
        struct Q; relation rows(key:i32); relation blocked(key:i32);
        answer(key) :- rows(key), !blocked(key);
    }
    .map(|(key,)| *key)
    .collect();
    assert_eq!(actual, [1, 2].into());
}

#[test]
fn cq_all_wildcard_negation_tests_relation_emptiness_without_binding() {
    let rows = [(1,), (2,)];
    for (blocked, expected) in [
        (
            Vec::<(i32, i32)>::new(),
            std::collections::BTreeSet::from([1, 2]),
        ),
        (vec![(9, 10)], std::collections::BTreeSet::new()),
    ] {
        let actual: std::collections::BTreeSet<_> = mini_linq::mini_linq! {
            struct Q; relation rows(key:i32); relation blocked(left:i32,right:i32);
            answer(key) :- rows(key), !blocked(_,_);
        }
        .map(|(key,)| *key)
        .collect();
        assert_eq!(actual, expected);
        let existence: Vec<_> = mini_linq::mini_linq! {
            struct Q; relation blocked(left:i32,right:i32);
            relation answer(unit:());
            answer(()) :- !blocked(_,_);
        }
        .collect();
        assert_eq!(
            existence,
            if blocked.is_empty() {
                vec![((),)]
            } else {
                vec![]
            }
        );
    }
}

#[test]
fn cq_constant_only_negation_tests_matching_rows_not_global_emptiness() {
    let blocked = [(1, 7), (2, 8)];
    let absent: Vec<_> = mini_linq::mini_linq! {
        struct Q; relation blocked(key:i32,kind:i32);
        relation answer(unit:());
        answer(()) :- !blocked(1,8);
    }
    .collect();
    assert_eq!(absent, [((),)]);
    let present: Vec<_> = mini_linq::mini_linq! {
        struct Q; relation blocked(key:i32,kind:i32);
        relation answer(unit:());
        answer(()) :- !blocked(1,7);
    }
    .collect();
    assert!(present.is_empty());
}

#[test]
fn named_sequence_planner_swaps_checked_policy_and_preserves_target_delegation() {
    macro_rules! planned {
        ($($planner:ident)::+ $(($($args:tt)*))?; $($output:tt)*) => {
            mini_linq::compile! {
                pipeline {
                    plan = $($planner)::+ $(($($args)*))?;
                    print = capture_plan;
                }
                physical {
                    relation rows(x:i32);
                    r = pipe canon rename rows@0 {x -> x};
                    return r as answer(x);
                }
                structures {
                    r host scan = ::core::iter::Map<_, _> [];
                    r canon = ::std::collections::HashSet<_>;
                }
                sequence { result = store rows r; $($output)* }
            }
        };
    }
    let default: cq_lang_sequence::Program =
        syn::parse_str(planned!(mini_linq::passes::plan;)).unwrap();
    let custom: cq_lang_sequence::Program =
        syn::parse_str(planned!(core_pass_fixtures::plan_reserve(expected);)).unwrap();
    assert_eq!(
        default.sequence.output,
        Some(cq_lang_sequence::OutputProtocol::Count)
    );
    assert_eq!(
        custom.sequence.output,
        Some(cq_lang_sequence::OutputProtocol::Reserve)
    );
    cq_lang_sequence::check_rewrite(&default, &custom).unwrap();
    cq_lang_sequence::check_grading(&custom, &cq_sequence_pull::machine()).unwrap();
    let authored: cq_lang_sequence::Program =
        syn::parse_str(planned!(core_pass_fixtures::plan_reserve(expected); output count;))
            .unwrap();
    assert_eq!(
        authored.sequence.output,
        Some(cq_lang_sequence::OutputProtocol::Count)
    );
    let delegated: cq_lang_sequence::Program =
        syn::parse_str(planned!(mini_linq::passes::plan(mini_linq::passes::plan_sequential);))
            .unwrap();
    assert_eq!(delegated, default);
    let preserved: cq_lang_sequence::Program =
        syn::parse_str(planned!(mini_linq::passes::plan_sequential(preserve_order);)).unwrap();
    assert_eq!(preserved, default);
    let automatic: cq_lang_sequence::Program =
        syn::parse_str(planned!(mini_linq::passes::plan(lookup_first);)).unwrap();
    cq_lang_sequence::check_rewrite(&default, &automatic).unwrap();
    cq_lang_sequence::check_grading(&automatic, &cq_sequence_pull::machine()).unwrap();

    let rows = [(1,), (1,), (2,)];
    let custom: Vec<_> = mini_linq::compile! {
        pipeline { plan = core_pass_fixtures::plan_reserve(expected); }
        physical {
            relation rows(x:i32);
            r = pipe canon rename rows@0 {x -> x};
            return r as answer(x);
        }
        structures {
            r host scan = ::core::iter::Map<_, _> [];
            r canon = ::std::collections::HashSet<_>;
        }
        sequence { result = store canon rows r; }
    }
    .collect();
    let default: Vec<_> = mini_linq::mini_linq! {
        struct Q; relation rows(x:i32); answer(x) :- rows(x);
    }
    .collect();
    assert_eq!(custom, default);
}

#[test]
fn facade_observes_every_default_boundary_without_executing_the_query() {
    macro_rules! observe {
        ($boundary:ident) => {
            mini_linq::compile! {
                observe { boundary = $boundary; stage = ::core::stringify; }
                pipeline {}
                struct Observed;
                relation R(x: i32); relation S(x: i32);
                answer(x) :- R(x), S(x);
            }
        };
    }
    let source: cq_lang_cq::Program = syn::parse_str(observe!(cq)).unwrap();
    let vo: cq_lang_vo::Program = syn::parse_str(observe!(vo)).unwrap();
    assert_eq!(
        vo,
        cq_planning::prefix_valuation(&source, &Default::default()).unwrap()
    );
    let ra: cq_lang_ra::Program = syn::parse_str(observe!(relational_plan)).unwrap();
    assert_eq!(
        ra,
        cq_planning::compile_prefix(&source, &Default::default()).unwrap()
    );
    syn::parse_str::<cq_lang_ix::Program>(observe!(index_requirements)).unwrap();
    syn::parse_str::<cq_lang_physical::Program>(observe!(physical)).unwrap();
    syn::parse_str::<cq_lang_structures::Program>(observe!(structures)).unwrap();
    syn::parse_str::<cq_lang_structures::Program>(observe!(represent)).unwrap();
    syn::parse_str::<cq_lang_structures::Program>(observe!(stage)).unwrap();
    syn::parse_str::<cq_lang_sequence::Program>(observe!(sequence)).unwrap();
    let decomposed: cq_lang_sequence::Program = syn::parse_str(observe!(decompose)).unwrap();
    let graded: cq_lang_sequence::Program = syn::parse_str(observe!(grade)).unwrap();
    assert_eq!(decomposed, graded);
    cq_lang_sequence::check_grading(&graded, &cq_sequence_pull::machine()).unwrap();
    let rust = observe!(final_rust);
    syn::parse_str::<syn::Expr>(rust).unwrap();
    assert!(rust.contains("profile_select"));
    assert!(!rust.contains("translate_pull"));
}

#[test]
fn observation_keeps_authored_planner_arguments() {
    let observed = mini_linq::compile! {
        observe { boundary = relational_plan; stage = ::core::stringify; }
        pipeline { plan = ::mini_linq::passes::plan(order(S, R), no_reduce); }
        struct Observed;
        relation R(x: i32); relation S(x: i32);
        answer(x) :- R(x), S(x);
    };
    let source: cq_lang_cq::Program = syn::parse_quote! {
        struct Observed;
        relation R(x: i32); relation S(x: i32);
        answer(x) :- R(x), S(x);
    };
    let options = syn::parse_quote!(order(S, R), no_reduce);
    let expected = cq_planning::compile_prefix(&source, &options).unwrap();
    assert_eq!(
        syn::parse_str::<cq_lang_ra::Program>(observed).unwrap(),
        expected
    );
}

#[test]
fn observed_cq_preserves_embedded_rust_literals() {
    let observed = mini_linq::compile! {
        observe { boundary = cq; stage = ::core::stringify; }
        pipeline {}
        struct Expressions;
        relation R(value: i32);
        relation answer(value: String);
        answer(format!(" :: ( & * ! > {x}")) :-
            R(x), let label = " :: ( & * ! >".to_owned(), if *x > 0 && !label.is_empty();
    };
    let expected: cq_lang_cq::Program = syn::parse_quote! {
        struct Expressions;
        relation R(value: i32);
        relation answer(value: String);
        answer(format!(" :: ( & * ! > {x}")) :-
            R(x), let label = " :: ( & * ! >".to_owned(), if *x > 0 && !label.is_empty();
    };
    assert_eq!(
        syn::parse_str::<cq_lang_cq::Program>(observed).unwrap(),
        expected
    );
}
