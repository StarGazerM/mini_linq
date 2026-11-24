use cq_lang_ix as ix;
use cq_lang_physical as physical;
use cq_physical_lowering::{check, compile, erase};
use cq_testkit::{cases, corpus as scenarios, rewrites as authored, semantics as oracle, values};
#[test]
fn group_survives_ix_erasure_and_preserves_unique_input_valuations() {
    let (source, target) = lower(
        "relational {
            relation R(k:i32,id:i32,v:i32);
            relation answer(k:i32,n:usize,total:i32,lo:Option<i32>,hi:Option<i32>);
            r = rename R {k -> k,id -> id,v -> v};
            valuations = project r keep {k,id,v};
            grouped = group valuations by [k] {
                n:usize = count(); total:i32 = sum(v);
                lo:Option<i32> = min(v); hi:Option<i32> = max(v);
            };
            output grouped as answer(k,n,total,lo,hi);
        } indexes {}",
    );
    let expected: physical::Program = syn::parse_quote! { physical {
        relation R(k:i32,id:i32,v:i32);
        relation answer(k:i32,n:usize,total:i32,lo:Option<i32>,hi:Option<i32>);
        r = pipe rename R {k -> k,id -> id,v -> v};
        valuations = pipe project r keep {k,id,v};
        grouped = pipe group valuations by [k] {
            n:usize = count(); total:i32 = sum(v);
            lo:Option<i32> = min(v); hi:Option<i32> = max(v);
        };
        return grouped as answer(k,n,total,lo,hi);
    }};
    assert_eq!(target, expected);
    assert_eq!(erase(&target), source.ra);
    let indexed = cq_index_policy::compile(&source.ra).unwrap();
    assert_eq!(indexed, source);
    let structures = cq_structure_selection::compile(&target).unwrap();
    let selected = cq_lang_structures::contract::check(&target, &structures).unwrap();
    assert_eq!(selected.unique["grouped"], ["k"]);
    assert_eq!(selected.unique["valuations"], ["id", "k", "v"]);
    assert!(target.program.definitions[1].canon.is_none());
    assert!(target.program.definitions[2].canon.is_none());
    assert!(matches!(
        target.program.definitions[2].operator,
        physical::Operator::Group(_)
    ));
    let facts = physical::contract::check(&target).unwrap();
    assert_eq!(facts.headings["valuations"], ["id", "k", "v"]);
    assert_eq!(facts.headings["grouped"], ["hi", "k", "lo", "n", "total"]);
    assert_eq!(facts.rows["grouped"].types["n"], syn::parse_quote!(usize));
    assert_eq!(
        facts.rows["grouped"].types["lo"],
        syn::parse_quote!(Option<i32>)
    );
    let mut redundant = target;
    redundant.program.definitions[1].mode = syn::parse_quote!(let);
    redundant.program.definitions[1].canon = Some(Default::default());
    check(&source, &redundant).unwrap();
}

#[test]
fn needed_group_boundary_supplies_later_consumers_in_the_same_analysis() {
    let relational = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); relation answer(a:i32,n:usize);
        r = rename R {a -> a,b -> b};
        p = project r keep {a};
        g = group p by [] { n:usize = count(); };
        joined = natural_join p with g;
        output joined as answer(a,n);
    }};
    let source = cq_index_policy::compile(&relational).unwrap();
    let target = compile(&source).unwrap();
    let expected: physical::Program = syn::parse_quote! { physical {
        relation R(a:i32,b:i32); relation answer(a:i32,n:usize);
        r = pipe rename R {a -> a,b -> b};
        p = let canon project r keep {a};
        g = let group p by [] { n:usize = count(); };
        joined = pipe probe first [] p,g;
        return joined as answer(a,n);
    }};
    assert_eq!(target, expected);
    check(&source, &target).unwrap();
}

#[test]
fn global_group_preserves_empty_key_and_inherited_input_uniqueness() {
    let source: ix::Program = syn::parse_quote! {
        relational {
            relation R(v:i32); relation answer(n:usize);
            r = rename R {v -> v};
            grouped = group r by [] { n:usize = count(); };
            output grouped as answer(n);
        } indexes {}
    };
    let target = compile(&source).unwrap();
    assert!(
        target
            .program
            .definitions
            .iter()
            .all(|definition| definition.canon.is_none())
    );
    assert_eq!(erase(&target), source.ra);
    let physical::Operator::Group(group) = &target.program.definitions[1].operator else {
        panic!("group was not propagated");
    };
    assert!(group.keys.is_empty());
}

#[test]
fn canonical_placement_preserves_operator_keys_and_generator_multiplicity() {
    for (source, expected) in [
        (
            "relational {
                u = unit; b = bind u with x:i32 = (1);
                g = guard b if (*x != 0); p = project g keep {x};
                output p as answer(x);
            } indexes {}",
            "physical {
                u = pipe unit; b = pipe bind u with x:i32 = (1);
                g = pipe guard b if (*x != 0); p = pipe project g keep {x};
                return p as answer(x);
            }",
        ),
        (
            "relational {
                u = unit; b = bind u with for x:i32 = ([1,1]);
                p = project b keep {x}; output p as answer(x);
            } indexes {}",
            "physical {
                u = pipe unit; b = pipe bind u with for x:i32 = ([1,1]);
                p = let canon project b keep {x}; return p as answer(x);
            }",
        ),
        (
            "relational {
                relation R(x:i32,y:i32); r = rename R {x -> x,y -> y};
                p = project r keep {x}; output p as answer(x);
            } indexes {}",
            "physical {
                relation R(x:i32,y:i32);
                r = pipe rename R {x -> x,y -> y};
                p = let canon project r keep {x}; return p as answer(x);
            }",
        ),
    ] {
        let source: ix::Program = syn::parse_str(source).unwrap();
        let expected: physical::Program = syn::parse_str(expected).unwrap();
        let target = compile(&source).unwrap();
        assert_eq!(target, expected);
        check(&source, &target).unwrap();
    }
}

#[test]
fn missing_binding_union_canon_requires_an_alternative_boundary() {
    let (source, target) = lower(
        "relational {
            relation R(x:i32); relation S(x:i32);
            r = rename R {x -> x}; s = rename S {x -> x};
            u = union r,s; output u as answer(x);
        } indexes {}",
    );
    let expected: physical::Program = syn::parse_quote! { physical {
        relation R(x:i32); relation S(x:i32);
        r = pipe rename R {x -> x}; s = pipe rename S {x -> x};
        u = let canon union r,s; return u as answer(x);
    }};
    assert_eq!(target, expected);
    let mut missing = target.clone();
    missing.program.definitions[2].canon = None;
    assert!(physical::contract::check(&missing).is_err());
    assert!(check(&source, &missing).is_err());

    missing.program.return_canon = Some(Default::default());
    check(&source, &missing).unwrap();
    let mut redundant = target;
    redundant.program.return_canon = Some(Default::default());
    check(&source, &redundant).unwrap();
}

#[test]
fn consumer_canonical_cannot_replace_a_group_input_boundary() {
    let source: ix::Program = syn::parse_quote! { relational {
        relation R(x:i32); relation S(x:i32); relation answer(n:usize);
        r = rename R {x -> x}; s = rename S {x -> x};
        u = union r,s; g = group u by [] { n:usize = count(); };
        output g as answer(n);
    } indexes {} };
    let target =
        cq_physical_lowering::compile_with(&source, quote::quote!(consumer = canonical)).unwrap();
    let expected: physical::Program = syn::parse_quote! { physical {
        relation R(x:i32); relation S(x:i32); relation answer(n:usize);
        consumer canonical;
        r = pipe rename R {x -> x}; s = pipe rename S {x -> x};
        u = let canon union r,s; g = pipe group u by [] { n:usize = count(); };
        return g as answer(n);
    }};
    assert_eq!(target, expected);
    let mut missing = target;
    missing.program.definitions[2].canon = None;
    assert!(physical::contract::check(&missing).is_err());
    assert!(check(&source, &missing).is_err());
}

#[test]
fn disjoint_union_inherits_input_keys_and_accepts_redundant_canon() {
    let ra: ix::ra::Program = syn::parse_quote! { relational {
        relation R(x:i32,y:i32); relation S(x:i32,y:i32);
        r = rename R {x -> x,y -> y}; s = rename S {x -> x,y -> y};
        a = anti [x] s,r;
        u = union r,a block free[x] interface[y] fallback;
        output u as answer(x,y);
    }};
    let source = cq_index_policy::compile(&ra).unwrap();
    let target = compile(&source).unwrap();
    let expected: physical::Program = syn::parse_quote! { physical {
        relation R(x:i32,y:i32); relation S(x:i32,y:i32);
        r = pipe rename R {x -> x,y -> y}; s = pipe rename S {x -> x,y -> y};
        a = pipe anti eager [x] s,r;
        u = pipe union r,a block free[x] interface[y] fallback;
        return u as answer(x,y);
    }};
    assert_eq!(target, expected);
    let mut returned = target.clone();
    returned.program.return_canon = Some(Default::default());
    check(&source, &returned).unwrap();
    let mut redundant = target;
    redundant.program.definitions[3].mode = syn::parse_quote!(let);
    redundant.program.definitions[3].canon = Some(Default::default());
    check(&source, &redundant).unwrap();
    redundant.program.return_canon = Some(Default::default());
    check(&source, &redundant).unwrap();
}


#[test]
fn literal_occurrences_lower_to_retained_sources_without_select_or_guard() {
    let (source, target) = lower(
        "relational { relation R(a:i32,b:i32);
        r = rename R {a -> x} constants {b = 7}; output r as answer(x);
        } indexes {}",
    );
    assert_eq!(target.program.definitions.len(), 1);
    assert!(matches!(
        target.program.definitions[0].mode,
        physical::Mode::Let(_)
    ));
    assert!(matches!(&target.program.definitions[0].operator,
        physical::Operator::Rename(rename) if rename.constants.is_some()));
    assert_eq!(erase(&target), source.ra);
    let (_, target) = lower(
        "relational { relation R(a:i32);
        r = rename R {} constants {a = 7}; output r as answer();
        } indexes {}",
    );
    assert!(matches!(
        target.program.definitions[0].mode,
        physical::Mode::Exists(_)
    ));
}

#[test]
fn scenarios_preserve_relational_answers_under_probe_and_merge() {
    for case in scenarios::SCENARIOS {
        let source = case.parse_ix().unwrap();
        for merge in [false, true] {
            let mut target = compile(&source).unwrap();
            if merge {
                target = authored::merged(&target).unwrap();
            }
            check(&source, &target).unwrap();
            if !merge {
                assert_eq!(
                    target,
                    syn::parse_str::<physical::Program>(case.physical).unwrap(),
                    "{}",
                    case.name
                );
            }
            assert_eq!(erase(&target), source.ra);
            let mut changed = target.clone();
            changed.program.head.relation = syn::parse_quote!(wrong);
            assert!(check(&source, &changed).is_err());
            if let Some(data) = cases::CASES.iter().find(|c| c.name == case.name) {
                let database = data
                    .inputs
                    .iter()
                    .map(|(n, rows)| ((*n).into(), rows.iter().map(|r| r.to_vec()).collect()))
                    .collect();
                let expected = data
                    .expected
                    .iter()
                    .map(|r| r.to_vec())
                    .collect::<values::Rows>();
                assert_eq!(
                    oracle::evaluate_ra(&erase(&target), &database).output,
                    expected,
                    "{}",
                    case.name
                );
            }
        }
    }
}
#[test]
fn demands_are_a_pass_obligation() {
    let case = scenarios::SCENARIOS
        .iter()
        .find(|c| c.name == "triangle")
        .unwrap();
    let source = case.parse_ix().unwrap();
    let target = compile(&source).unwrap();
    let mut missing = source.clone();
    missing.indexes.clear();
    ix::contract::check(&missing).unwrap();
    physical::contract::check(&target).unwrap();
    assert!(check(&missing, &target).is_err());
    assert!(compile(&missing).is_err());
}

fn lower(source: &str) -> (ix::Program, physical::Program) {
    let source = syn::parse_str(source).unwrap();
    let target = compile(&source).unwrap();
    check(&source, &target).unwrap();
    (source, target)
}

#[test]
fn const_demand_and_point_modifier_are_independent_obligations() {
    let (source, target) = lower(
        "relational { relation R(a:i32,b:i32); r = rename R {a -> x,b -> y};
        s = select r if (x = 2); output s as answer(x,y);
        } indexes {i = @r[0] const;}",
    );
    assert!(matches!(
        target.program.definitions[1].operator,
        physical::Operator::Point(..)
    ));
    let mut changed = target.clone();
    changed.program.definitions[1] = syn::parse_quote!(s = let canon select r if (x = 2););
    check(&source, &changed).unwrap();
    let mut missing = source.clone();
    missing.indexes.clear();
    assert!(check(&missing, &target).is_err());
    let mut retained = target;
    retained.program.definitions[0].mode = syn::parse_quote!(let);
    check(&source, &retained).unwrap();
}

#[test]
fn relational_column_equality_is_a_select_and_rust_is_a_guard() {
    let (_, target) = lower(
        "relational {relation R(a:i32,b:i32); r = rename R {a -> x,b -> y};
        s = select r if (x = y); g = guard s if (*x > 2); output g as answer(x,y);
        } indexes {}",
    );
    assert!(matches!(
        target.program.definitions[1].operator,
        physical::Operator::Select(_)
    ));
    assert!(matches!(
        target.program.definitions[2].operator,
        physical::Operator::Guard(_)
    ));
}

#[test]
fn admissible_join_and_canonicalization_rewrites_are_accepted() {
    let source = "relational { relation L(a:i32); relation R(a:i32,b:i32);
        l = rename L {a -> a}; r = rename R {a -> a,b -> b}; j = natural_join l with r;
        p = project j keep {a}; e = bind p with n:i32 = (1); output e as answer(a,n);
    } indexes { i = @r[0] stream; }";
    let (source, target) = lower(source);
    let merged = authored::merged(&target).unwrap();
    check(&source, &merged).unwrap();
    let canon = authored::canonical_projects(&target).unwrap();
    check(&source, &canon).unwrap();
    let mut changed = target.clone();
    changed.program.definitions[0].canon = Some(Default::default());
    check(&source, &changed).unwrap();
    let mut changed = target;
    changed.program.definitions[3].canon = None;
    changed.program.definitions[4].canon = None;
    assert!(check(&source, &changed).is_err());
    assert!(physical::contract::check(&changed).is_err());
}

#[test]
fn chain_reordering_fails_erasure_even_if_local_program_is_scoped() {
    let (source, target) = lower(
        "relational {relation R(a:i32); r = rename R {a -> x};
        g = guard r if (*x != 0); b = bind g with y:i32 = (10 / *x);
        output b as answer(x,y); } indexes {}",
    );
    let mut changed = target;
    changed.program.definitions[1] = syn::parse_quote!(b = pipe bind r with y:i32 = (10 / *x););
    changed.program.definitions[2] = syn::parse_quote!(g = let canon guard b if (*x != 0););
    changed.program.returned = syn::parse_quote!(g);
    physical::contract::check(&changed).unwrap();
    assert!(check(&source, &changed).is_err());
}

#[test]
fn consumer_canonical_relaxes_only_the_return_boundary() {
    let source: ix::Program = syn::parse_quote! {
        relational {
            relation R(a:i32);
            r = rename R {a -> x};
            b = bind r with y:i32 = (*x % 2);
            p = project b keep {y};
            output p as answer(y);
        } indexes {}
    };
    let ordinary = compile(&source).unwrap();
    assert!(ordinary.program.definitions[2].canon.is_some());
    let target =
        cq_physical_lowering::compile_with(&source, quote::quote!(consumer = canonical)).unwrap();
    assert!(target.program.consumer_canonical.is_some());
    assert!(target.program.definitions[2].canon.is_none());
    assert!(target.program.return_canon.is_none());
    check(&source, &target).unwrap();
    let mut missing = target.clone();
    missing.program.consumer_canonical = None;
    assert!(check(&source, &missing).is_err());
    assert!(physical::contract::check(&missing).is_err());
    let mut redundant = target;
    redundant.program.return_canon = Some(Default::default());
    check(&source, &redundant).unwrap();
    assert_eq!(
        compile(&source).unwrap(),
        cq_physical_lowering::compile_with(&source, Default::default(),).unwrap()
    );
}

#[test]
fn input_sets_seed_uniqueness_without_compiler_facts() {
    let source: ix::Program = syn::parse_quote! {
        relational {
            relation R(a:i32,b:i32);
            r = rename R {a -> x,b -> y};
            p = project r keep {x,y};
            output p as answer(x,y);
        } indexes {}
    };
    let target = compile(&source).unwrap();
    assert!(
        target
            .program
            .definitions
            .iter()
            .all(|definition| definition.canon.is_none())
    );
    check(&source, &target).unwrap();
    let facts = physical::contract::check(&target).unwrap();
    let keys = physical::uniqueness(&target, &facts.headings, &facts.disjoint, |d, _| d.canon.is_some());
    assert_eq!(keys["r"], ["x", "y"]);
    assert_eq!(keys["p"], ["x", "y"]);
}

#[test]
fn consumer_fact_preserves_internal_union_canonicalization() {
    let case = cq_testkit::blocks::SCENARIOS
        .iter()
        .find(|case| case.name == "block_union")
        .unwrap();
    let source = case.parse_ix().unwrap();
    let ordinary = compile(&source).unwrap();
    let target =
        cq_physical_lowering::compile_with(&source, quote::quote!(consumer = canonical)).unwrap();
    for (before, after) in ordinary
        .program
        .definitions
        .iter()
        .zip(&target.program.definitions)
    {
        if before.name != ordinary.program.returned {
            assert_eq!(before.canon, after.canon);
        }
    }
    assert!(target.program.definitions.iter().any(|definition| {
        matches!(definition.operator, physical::Operator::Union(_)) && definition.canon.is_some()
    }));
    check(&source, &target).unwrap();
}

#[test]
fn recorded_requirements_distinguish_reads_of_the_same_result() {
    let ra: ix::ra::Program = syn::parse_quote! { relational {
        relation R(a:i32,b:i32); relation L(a:i32); relation M(b:i32);
        r = rename R {a -> x,b -> y};
        l = rename L {a -> x};
        m = rename M {b -> y};
        point = select r if (x = 2);
        window = select r if (x >= 1);
        equal = select r if (x = y);
        on_x = natural_join l with r;
        on_y = semijoin m with r;
        both = natural_join point with window;
        same = natural_join both with equal;
        keyed = natural_join on_x with on_y;
        result = natural_join same with keyed;
        output result as answer(x,y);
    }};
    let mut source = cq_index_policy::compile(&ra).unwrap();
    // Lowering consumes the recorded entries, not the producer's list positions.
    source.indexes = source.indexes.into_iter().rev().collect();
    source.indexes.push_punct(Default::default());
    let info = ix::ra::contract::check(&ra).unwrap();
    let requirements = ix::read_requirements(&source, &info).unwrap();
    for (reader, supply, columns) in [
        ("point", ix::SupplyKind::Const, vec![0]),
        ("window", ix::SupplyKind::Range, vec![0]),
        ("on_x", ix::SupplyKind::Stream, vec![0]),
        ("on_y", ix::SupplyKind::Stream, vec![1]),
    ] {
        let recorded = requirements[reader];
        assert!(
            source
                .indexes
                .iter()
                .any(|index| std::ptr::eq(index, recorded))
        );
        assert_eq!(
            ix::signature(&recorded.requirement),
            ("r".into(), columns, supply)
        );
    }
    assert!(!requirements.contains_key("equal"));
    let target = compile(&source).unwrap();
    for definition in &target.program.definitions {
        match ix::symbol_name(&definition.name).as_str() {
            "point" => assert!(matches!(definition.operator, physical::Operator::Point(..))),
            "window" => assert!(matches!(definition.operator, physical::Operator::Range(..))),
            "equal" => assert!(matches!(
                definition.operator,
                physical::Operator::Select(..)
            )),
            "on_x" | "on_y" => {
                let columns = match &definition.operator {
                    physical::Operator::Probe(_, keyed) | physical::Operator::Semi(_, keyed) => {
                        keyed
                            .columns
                            .iter()
                            .map(ix::symbol_name)
                            .collect::<Vec<_>>()
                    }
                    _ => panic!("expected a recorded keyed read"),
                };
                assert_eq!(columns, [if definition.name == "on_x" { "x" } else { "y" }]);
            }
            _ => {}
        }
    }
    check(&source, &target).unwrap();
}
