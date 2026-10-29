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
fn large_seeds_preserve_determinism_and_rust_oracle_lane() {
    for seed in [u32::MAX as u64 + 1, 1 << 63, u64::MAX - 1, u64::MAX] {
        let case = generate(seed);
        assert_eq!(case, generate(seed));
        assert_eq!(case.requires_rust_oracle(), seed & 1 == 0);
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
fn every_generator_and_opaque_rust_operation_routes_away_from_sql() {
    use cq_testkit::fuzz::{Expr, Generator, RustOp};
    for generator in [
        Generator::Range,
        Generator::InclusiveRange,
        Generator::Duplicates,
        Generator::Empty,
        Generator::Mapped,
    ] {
        for operation in [
            None,
            Some(RustOp::Divide),
            Some(RustOp::Remainder),
            Some(RustOp::Add),
            Some(RustOp::Subtract),
            Some(RustOp::Multiply),
            Some(RustOp::Unwrap),
        ] {
            let mut case = generator_case(generator);
            if let Some(operator) = operation {
                case.query.bindings[0].expression = Expr::Rust(operator, Box::new(Expr::Var(0)));
            }
            assert!(case.requires_rust_oracle());
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
    assert!(smaller.requires_rust_oracle());
}

#[test]
fn literal_atoms_keep_source_positions_and_resolve_local_dependencies() {
    use cq_testkit::fuzz::{Atom, Block, BlockKind, Case, Expr, Query, Relation, Step, Term};
    let mut case = Case {
        seed: 0,
        query: Query {
            schema: vec![Relation {
                name: "r0".into(),
                arity: 3,
            }],
            atoms: vec![Atom {
                relation: 0,
                terms: vec![
                    Term::Expression(Expr::Negate(Box::new(Expr::Var(0)))),
                    Term::Var(0),
                    Term::Constant(5),
                ],
            }],
            bindings: Vec::new(),
            guards: Vec::new(),
            constraints: Vec::new(),
            blocks: Vec::new(),
            head: vec![Expr::Var(0)],
            order: vec![Step::Atom(0)],
        },
        data: vec![vec![vec![-2, 2, 5], vec![-3, 3, 6]]],
    };
    for source in [case.cq(), case.written_cq()] {
        let program: cq_testkit::cq::Program = syn::parse_str(&source).unwrap();
        assert_eq!(
            program.query.body.len(),
            1,
            "literals must not become guards"
        );
        let cq_testkit::cq::BodyItem::Positive { atom } = &program.query.body[0] else {
            panic!("expected original atom");
        };
        assert_eq!(atom.args.len(), 3);
        assert!(matches!(atom.args[2], syn::Expr::Lit(_)));
    }
    assert!(case.sql().contains("a0.c2 = 5"));
    assert!(case.sql().contains("a0.c0 = (-(a0.c1))"));
    case.query.blocks.push(Block {
        kind: BlockKind::Union,
        branches: vec![vec![Step::Atom(0)], vec![Step::Atom(0)]],
        interface: vec![0],
    });
    case.query.order = vec![Step::Block(0)];
    assert!(case.sql().contains("a.c2 = 5"));
    assert!(case.sql().contains("a.c0 = (-(a.c1))"));
}

#[test]
fn rust_generator_samples_literal_excluded_local_zero_rows() {
    use cq_testkit::fuzz::{Expr, RustOp, Term};
    let mut found = false;
    for seed in 0..2048 {
        let case = generate(seed);
        for atom in &case.query.atoms {
            if matches!(
                atom.terms.as_slice(),
                [
                    Term::Expression(Expr::Rust(RustOp::Divide, _)),
                    Term::Var(_),
                    Term::Constant(5)
                ]
            ) && !case.data[atom.relation].is_empty()
            {
                assert!(case.data[atom.relation].contains(&vec![0, 0, 6]));
                found = true;
            }
        }
    }
    assert!(
        found,
        "sample an occurrence literal alongside local total division"
    );
}

#[test]
fn relational_source_uses_where_and_rust_boolean_guards_stay_opaque() {
    use cq_testkit::fuzz::{Comparison, Expr, Generator, Guard, RelTerm, Relational, Step};
    let mut case = generator_case(Generator::Range);
    case.query.bindings.clear();
    case.query.head = vec![Expr::Var(0)];
    case.query.constraints = vec![Relational::Or(vec![
        vec![
            Relational::Between(RelTerm::Var(0), RelTerm::Literal(-2), RelTerm::Parameter(3)),
            Relational::Compare(Comparison::NotEqual, RelTerm::Var(0), RelTerm::Literal(0)),
        ],
        vec![Relational::In(RelTerm::Var(0), vec![5, 7])],
    ])];
    case.query.guards = vec![Guard::Or(
        Box::new(Guard::Compare(
            Comparison::Equal,
            Expr::Var(0),
            Expr::Constant(1),
        )),
        Box::new(Guard::Compare(
            Comparison::Greater,
            Expr::OpaqueCall(Box::new(Expr::Var(0))),
            Expr::Constant(2),
        )),
    )];
    case.query.order = vec![
        Step::Atom(0),
        Step::Constraint(0),
        Step::Guard(0),
        Step::Atom(1),
    ];
    let source = case.cq();
    assert!(
        source.contains("where (v0 in -2..=parameter_positive_3 && v0 != 0) || (v0 in [5, 7])")
    );
    assert!(
        source.contains("if ((v0.to_owned() == 1) || (std::convert::identity(v0.to_owned()) > 2))")
    );
    let program: cq_testkit::cq::Program = syn::parse_str(&source).unwrap();
    cq_testkit::cq::contract::check(&program).unwrap();
    assert_eq!(program.query.body.len(), 4);
    assert!(matches!(
        &program.query.body[1],
        cq_testkit::cq::BodyItem::Constraint(_)
    ));
    let cq_testkit::cq::BodyItem::Filter(filter) = &program.query.body[2] else {
        panic!("Rust guard must remain one opaque filter");
    };
    let expected: syn::Expr =
        syn::parse_str("((v0.to_owned() == 1) || (std::convert::identity(v0.to_owned()) > 2))")
            .unwrap();
    assert_eq!(filter.condition, expected);
    assert!(case.sql().contains("BETWEEN -2 AND 3"));
    assert!(case.sql().contains("IN (5, 7)"));
}
