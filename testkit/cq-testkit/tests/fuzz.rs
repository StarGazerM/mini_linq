use cq_testkit::fuzz::{Coverage, FEATURE_TAGS, generate};
use std::collections::BTreeSet;

#[test]
fn public_generator_and_printers_are_seeded_and_cover_all_tags_in_2048_seeds() {
    let mut coverage = Coverage::default();
    for seed in 0..2048 {
        let first = generate(seed);
        let second = generate(seed);
        assert_eq!(first, second);
        assert_eq!(first.cq(), second.cq());
        assert_eq!(first.written_cq(), second.written_cq());
        assert_eq!(first.sql(), second.sql());
        coverage.observe(&first);
    }
    assert_eq!(coverage.count("cases"), 2048);
    let advertised: BTreeSet<_> = FEATURE_TAGS.iter().copied().collect();
    let observed: BTreeSet<_> = coverage.counts().keys().map(String::as_str).collect();
    assert_eq!(advertised, observed, "tag inventory must be complete");
    for feature in FEATURE_TAGS {
        assert!(coverage.count(feature) > 30, "{feature}: {coverage}");
    }
    assert_eq!(coverage.count("never-observed"), 0);
}

#[test]
fn large_seeds_preserve_determinism_and_partial_lane() {
    for seed in [u32::MAX as u64 + 1, 1 << 63, u64::MAX - 1, u64::MAX] {
        let case = generate(seed);
        assert_eq!(case, generate(seed));
        assert_eq!(case.has_partial_terms(), seed & 1 == 0);
    }
}

fn generator_case(generator: cq_testkit::fuzz::Generator) -> cq_testkit::fuzz::Case {
    use cq_testkit::fuzz::{Atom, Binding, Case, Expr, Query, Relation, Step, Term};
    Case {
        seed: 0,
        query: Query {
            schema: vec![
                Relation {
                    name: "r0".into(),
                    arity: 1,
                },
                Relation {
                    name: "r1".into(),
                    arity: 1,
                },
            ],
            atoms: vec![
                Atom {
                    relation: 0,
                    terms: vec![Term::Var(0)],
                },
                Atom {
                    relation: 1,
                    terms: vec![Term::Var(2)],
                },
            ],
            bindings: vec![Binding {
                variable: 1,
                expression: Expr::Var(0),
                generator: Some(generator),
            }],
            guards: Vec::new(),
            constraints: Vec::new(),
            blocks: Vec::new(),
            head: vec![Expr::Var(1)],
            order: vec![Step::Atom(0), Step::Binding(0), Step::Atom(1)],
        },
        data: vec![vec![vec![2]], vec![vec![3]]],
    }
}

#[test]
fn every_generator_and_partial_expression_routes_away_from_sql() {
    use cq_testkit::fuzz::{Expr, Generator, PartialOp};
    for generator in [
        Generator::Range,
        Generator::InclusiveRange,
        Generator::Duplicates,
        Generator::Empty,
        Generator::IteratorPanic,
    ] {
        for partial in [
            None,
            Some(PartialOp::Divide),
            Some(PartialOp::Remainder),
            Some(PartialOp::Add),
            Some(PartialOp::Subtract),
            Some(PartialOp::Multiply),
            Some(PartialOp::Unwrap),
        ] {
            let mut case = generator_case(generator);
            if let Some(operator) = partial {
                case.query.bindings[0].expression = Expr::Partial(operator, Box::new(Expr::Var(0)));
            }
            assert!(case.has_partial_terms());
            assert!(case.sql().starts_with("-- "));
            assert!(case.sql().contains("compiled written-order"));
            for source in [case.cq(), case.written_cq()] {
                let program: cq_testkit::cq::Program = syn::parse_str(&source).unwrap();
                cq_testkit::cq::contract::check(&program).unwrap();
                assert!(matches!(
                    &program.query.body[1],
                    cq_testkit::cq::BodyItem::For(_)
                ));
            }
        }
    }
}

#[test]
fn generator_minimization_keeps_captures_and_nested_binding_scope() {
    use cq_testkit::fuzz::{Binding, Expr, Generator, Step};
    let mut case = generator_case(Generator::Range);
    case.query.bindings.push(Binding {
        variable: 3,
        expression: Expr::Var(1),
        generator: Some(Generator::Duplicates),
    });
    case.query.order.push(Step::Binding(1));
    case.query.head = vec![Expr::Var(3)];
    assert!(
        case.without_atom(0).is_none(),
        "cannot remove the captured variable"
    );
    let smaller = case.without_atom(1).expect("unrelated atom can be removed");
    assert_eq!(smaller.query.bindings, case.query.bindings);
    assert_eq!(
        smaller.query.order,
        vec![Step::Atom(0), Step::Binding(0), Step::Binding(1)]
    );
    let program: cq_testkit::cq::Program = syn::parse_str(&smaller.cq()).unwrap();
    cq_testkit::cq::contract::check(&program).unwrap();
    assert!(smaller.has_partial_terms());
}
