use cq_lang_ix as ix;
use cq_lang_physical as physical;
use cq_physical_lowering::{check, compile, erase};
use cq_testkit::{cases, corpus as scenarios, rewrites as authored, semantics as oracle, values};
#[test]
fn group_survives_ix_erasure_and_canonicalizes_all_input_valuations() {
    let (source, target) = lower(
        "relational {
            relation R(k:i32,id:i32,v:i32);
            relation answer(k:i32,n:usize,total:i32,lo:Option<i32>,hi:Option<i32>);
            r = rename R@0 {k -> k,id -> id,v -> v};
            valuations = project r keep {k,id,v};
            grouped = group valuations by [k] {
                n:usize = count(); total:i32 = sum(v);
                lo:Option<i32> = min(v); hi:Option<i32> = max(v);
            };
            output grouped as answer(k,n,total,lo,hi);
        } indexes {}",
    );
    assert_eq!(erase(&target), source.ra);
    let indexed = cq_index_policy::compile(&source.ra).unwrap();
    assert_eq!(indexed, source);
    let structures = cq_structure_selection::compile(&target).unwrap();
    let selected = cq_lang_structures::contract::check(&structures).unwrap();
    assert_eq!(selected.unique["grouped"], ["k"]);
    assert_eq!(selected.unique["valuations"], ["id", "k", "v"]);
    assert!(target.program.definitions[1].canon.is_some());
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
    let mut broken = target;
    broken.program.definitions[1].canon = None;
    assert!(check(&source, &broken).is_err());
}

#[test]
fn global_group_preserves_empty_key_and_inherited_input_uniqueness() {
    let source: ix::Program = syn::parse_quote! {
        relational {
            relation R(v:i32); relation answer(n:usize);
            r = rename R@0 {v -> v};
            grouped = group r by [] { n:usize = count(); };
            output grouped as answer(n);
        } indexes {}
    };
    let target =
        cq_physical_lowering::compile_with(&source, quote::quote!(host_set R discharged)).unwrap();
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
fn literal_occurrences_lower_to_retained_sources_without_select_or_guard() {
    let (source, target) = lower(
        "relational { relation R(a:i32,b:i32);
        r = rename R@0 {a -> x} constants {b = 7}; output r as answer(x);
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
        r = rename R@0 {} constants {a = 7}; output r as answer();
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
        "relational { relation R(a:i32,b:i32); r = rename R@0 {a -> x,b -> y};
        s = select r if (x = 2); output s as answer(x,y);
        } indexes {i = @r[0] const;}",
    );
    assert!(matches!(
        target.program.definitions[1].operator,
        physical::Operator::Point(..)
    ));
    let mut changed = target.clone();
    changed.program.definitions[1] = syn::parse_quote!(s = pipe canon select r if (x = 2););
    check(&source, &changed).unwrap();
    let mut missing = source.clone();
    missing.indexes.clear();
    assert!(
        check(&missing, &target)
            .unwrap_err()
            .to_string()
            .contains("demand")
    );
    let mut retained = target;
    retained.program.definitions[0].mode = syn::parse_quote!(let);
    check(&source, &retained).unwrap();
}

#[test]
fn relational_column_equality_is_a_select_and_rust_is_a_guard() {
    let (_, target) = lower(
        "relational {relation R(a:i32,b:i32); r = rename R@0 {a -> x,b -> y};
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
        l = rename L@0 {a -> a}; r = rename R@1 {a -> a,b -> b}; j = natural_join l with r;
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
}

#[test]
fn chain_reordering_fails_erasure_even_if_local_program_is_scoped() {
    let (source, target) = lower(
        "relational {relation R(a:i32); r = rename R@0 {a -> x};
        g = guard r if (*x != 0); b = bind g with y:i32 = (10 / *x);
        output b as answer(x,y); } indexes {}",
    );
    let mut changed = target;
    changed.program.definitions[1] = syn::parse_quote!(b = pipe bind r with y:i32 = (10 / *x););
    changed.program.definitions[2] = syn::parse_quote!(g = pipe canon guard b if (*x != 0););
    changed.program.returned = syn::parse_quote!(g);
    physical::contract::check(&changed).unwrap();
    assert!(check(&source, &changed).is_err());
}

#[test]
fn consumer_canonical_relaxes_only_the_return_boundary() {
    let source: ix::Program = syn::parse_quote! {
        relational {
            relation R(a:i32);
            r = rename R@0 {a -> x};
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
    let mut invalid = target;
    invalid.program.return_canon = Some(Default::default());
    assert!(check(&source, &invalid).is_err());
    assert_eq!(
        compile(&source).unwrap(),
        cq_physical_lowering::compile_with(&source, Default::default(),).unwrap()
    );
}

#[test]
fn discharged_host_sets_seed_uniqueness_without_a_consumer_fact() {
    let source: ix::Program = syn::parse_quote! {
        relational {
            relation R(a:i32,b:i32);
            r = rename R@0 {a -> x,b -> y};
            p = project r keep {x,y};
            output p as answer(x,y);
        } indexes {}
    };
    let target =
        cq_physical_lowering::compile_with(&source, quote::quote!(host_set R discharged)).unwrap();
    assert!(
        target
            .program
            .definitions
            .iter()
            .all(|definition| definition.canon.is_none())
    );
    assert!(target.program.host_sets[0].discharged.is_some());
    check(&source, &target).unwrap();
    let mut missing = target;
    missing.program.host_sets.clear();
    assert!(check(&source, &missing).is_err());
    assert!(
        cq_physical_lowering::compile_with(&source, quote::quote!(host_set Missing discharged),)
            .is_err()
    );
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
