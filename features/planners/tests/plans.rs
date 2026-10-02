use cq_planners::{Planner, check, check_valuation, compile, compile_valuation, cq, ra, vo};
use cq_testkit::{PassCase, SCENARIOS, cq_semantics, databases, semantics, valuation_semantics};
use quote::{ToTokens, quote};

#[test]
fn every_scenario_through_every_planner() {
    let mut combinations = 0;
    for scenario in SCENARIOS {
        let source = scenario.parse_cq().unwrap();
        for planner in Planner::ALL {
            let harness = PassCase {
                source: source.clone(),
                expected_next: None,
                compile: Box::new(|source: &cq::Program| compile(source, planner)),
                check: Box::new(check),
                mutations: vec![Box::new(|target: &ra::Program| {
                    let mut changed = target.clone();
                    changed.plan.output.head.relation = syn::parse_quote!(wrong_answer);
                    changed
                })],
                answers: Some(Box::new(|target: &ra::Program, database| {
                    semantics::evaluate_ra(target, database).output
                })),
            };
            let target = harness
                .run(scenario.case())
                .unwrap_or_else(|error| panic!("{}/{planner:?}: {error:?}", scenario.name));
            let valuation = compile_valuation(&source, planner).unwrap();
            check_valuation(&source, &valuation, planner).unwrap();
            let normal = &valuation.cq;
            for seed in 0..4 {
                let database = databases::binary_database(normal, seed);
                let expected = cq_semantics::evaluate_cq(normal, &database);
                assert_eq!(
                    valuation_semantics::evaluate(&valuation, &database),
                    expected,
                    "{} / {planner:?} / database {seed}",
                    scenario.name
                );
                assert_eq!(semantics::evaluate_ra(&target, &database).output, expected);
            }
            combinations += 1;
        }
    }
    assert_eq!(combinations, 270);
}

#[test]
fn pinned_vo_and_ra_decisions_for_each_planner() {
    for (fixture, scenario_name) in [
        ("triangle", "triangle"),
        ("chain", "oriented_chain"),
        ("fine_triangle", "fine_triangle"),
    ] {
        let source = SCENARIOS
            .iter()
            .find(|scenario| scenario.name == scenario_name)
            .unwrap()
            .parse_cq()
            .unwrap();
        for planner in Planner::ALL {
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
            let stem = format!("{}_{fixture}", planner.name());
            let expected_vo = std::fs::read_to_string(root.join(format!("{stem}.vo"))).unwrap();
            let expected_ra = std::fs::read_to_string(root.join(format!("{stem}.ra"))).unwrap();
            let valuation = compile_valuation(&source, planner).unwrap();
            let algebra = compile(&source, planner).unwrap();
            assert_eq!(
                format!("{}\n", valuation.to_token_stream()),
                expected_vo,
                "{stem}"
            );
            assert_eq!(
                format!("{}\n", algebra.to_token_stream()),
                expected_ra,
                "{stem}"
            );
            check_valuation(&source, &syn::parse_str(&expected_vo).unwrap(), planner).unwrap();
            check(&source, &syn::parse_str(&expected_ra).unwrap()).unwrap();
        }
    }
}

#[test]
fn argument_grammar_is_small_and_strict() {
    assert_eq!(
        syn::parse_str::<Planner>("").unwrap(),
        Planner::WrittenCoarse
    );
    for planner in Planner::ALL {
        assert_eq!(
            syn::parse2::<Planner>(planner.to_token_stream()).unwrap(),
            planner
        );
    }
    for arguments in [
        "written",
        "written coarse",
        "written, wide",
        "prefix, fine",
        "prefix auto, fine",
        "elimination, fine, extra",
        "hypergraph, fine",
        "hypergraph(extra)",
        "unknown",
        "written, coarse;",
    ] {
        assert!(syn::parse_str::<Planner>(arguments).is_err(), "{arguments}");
    }
}

#[test]
fn two_hundred_partial_fuzz_seeds_through_every_planner_check() {
    for ordinal in 0..200 {
        let seed = ordinal * 2;
        let case = cq_testkit::fuzz::generate(seed);
        assert!(case.has_partial_terms());
        let source: cq::Program = syn::parse_str(&case.cq()).unwrap();
        for planner in Planner::ALL {
            let target = compile(&source, planner)
                .unwrap_or_else(|error| panic!("seed {seed}, {planner:?}: {error}"));
            check(&source, &target).unwrap();
        }
    }
}

#[test]
fn hypergraph_reduces_trees_and_keeps_rust_opaque() {
    let source: cq::Program = syn::parse_quote! {
        struct Tree;
        relation R(x:i32,y:i32); relation S(y:i32,z:i32); relation T(z:i32,w:i32);
        answer(x,w) :- R(x,y), S(y,z), T(z,w), if x > 1, if partial(x), let q:i32 = opaque(w);
    };
    let valuation = compile_valuation(&source, Planner::Hypergraph).unwrap();
    assert_eq!(
        valuation
            .valuation
            .steps
            .iter()
            .filter(|s| matches!(s, vo::Step::Reduce(..)))
            .count(),
        4
    );
    let target = compile(&source, Planner::Hypergraph).unwrap();
    assert!(
        target
            .plan
            .definitions
            .iter()
            .any(|d| matches!(d.operator, ra::Operator::Select(_)))
    );
    let chain: Vec<_> = target
        .plan
        .definitions
        .iter()
        .filter_map(|d| match &d.operator {
            ra::Operator::Guard(g) => Some(g.condition.to_token_stream().to_string()),
            ra::Operator::Bind(b) => Some(b.expression.to_token_stream().to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(chain, ["partial (x)", "opaque (w)"]);

    let cyclic = SCENARIOS
        .iter()
        .find(|s| s.name == "triangle")
        .unwrap()
        .parse_cq()
        .unwrap();
    let cyclic = compile_valuation(&cyclic, Planner::Hypergraph).unwrap();
    assert!(
        !cyclic
            .valuation
            .steps
            .iter()
            .any(|s| matches!(s, vo::Step::Reduce(..)))
    );

    let mut missing_edge = valuation.clone();
    let edge = missing_edge
        .valuation
        .steps
        .iter()
        .position(|s| matches!(s, vo::Step::Reduce(..)))
        .unwrap();
    missing_edge.valuation.steps.remove(edge);
    assert!(check_valuation(&source, &missing_edge, Planner::Hypergraph).is_err());
}

#[test]
fn checks_reject_changed_chain_and_before_but_admit_free_rankings() {
    let source: cq::Program = syn::parse_quote! {
        struct Q; relation R(x:i32); relation S(w:i32);
        answer(y) :- R(x), S(w), if first(x), let y:i32 = partial(x), if last(y,w);
    };
    for planner in Planner::ALL {
        let mut changed = compile(&source, planner).unwrap();
        let guard = changed
            .plan
            .definitions
            .iter_mut()
            .find_map(|d| match &mut d.operator {
                ra::Operator::Guard(g) => Some(g),
                _ => None,
            })
            .unwrap();
        guard.condition = syn::parse_quote!(false);
        assert!(check(&source, &changed).is_err());

        let mut changed = compile_valuation(&source, planner).unwrap();
        let guard = changed
            .valuation
            .steps
            .iter()
            .position(|s| matches!(s, vo::Step::Guard(..)))
            .unwrap();
        let binder = changed
            .valuation
            .steps
            .iter()
            .position(|s| matches!(s, vo::Step::Let(..)))
            .unwrap();
        changed.valuation.steps.swap(guard, binder);
        assert!(check_valuation(&source, &changed, planner).is_err());
    }
    let normal = cq::normal::normalize(&source).unwrap();
    let early: vo::Program = syn::parse2(quote! {
        #normal valuation { R@0(x); if first(x); S@1(w); let y:i32 = partial(x); if last(y,w); }
    })
    .unwrap();
    assert!(
        check_valuation(&source, &early, Planner::PrefixConnectedCoarse)
            .unwrap_err()
            .to_string()
            .contains("Before")
    );

    let simple: cq::Program = syn::parse_quote! {
        struct Q; relation R(x:i32,y:i32); relation S(y:i32,z:i32);
        answer(x,z) :- R(x,y),S(y,z);
    };
    let normal = cq::normal::normalize(&simple).unwrap();
    let alternative: vo::Program = syn::parse2(quote! {
        #normal valuation { S@1(y,z); R@0(x,y); }
    })
    .unwrap();
    let target = cq_elaborate::compile(&alternative).unwrap();
    check(&simple, &target).unwrap();
    for planner in [
        Planner::PrefixConnectedCoarse,
        Planner::PrefixGreedyCoarse,
        Planner::EliminationCoarse,
    ] {
        check_valuation(&simple, &alternative, planner).unwrap();
    }
    assert!(check_valuation(&simple, &alternative, Planner::WrittenCoarse).is_err());
}
