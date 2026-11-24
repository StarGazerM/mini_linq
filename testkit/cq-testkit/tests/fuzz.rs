//! Generator, printer and G3 reference tests.
//!
//! Default tests use fixed hand-built cases. Generated sweeps are opt-in:
//! `cargo test -p cq-testkit -- --ignored` (DuckDB agreement: add `--features duckdb`).

use cq_testkit::fuzz::{
    Atom, Binding, Block, BlockKind, Case, Comparison, Coverage, Expr, FEATURE_TAGS, Fold, FoldArg,
    FoldKind, Generator, Guard, Profile, Query, RelTerm, Relation, Relational, RouteCoverage,
    RustOp, Step, Term, generate, generate_with,
};
use cq_testkit::values::{ColumnType, TypedRows, Value};
use std::collections::BTreeSet;

fn i(v: i32) -> Value {
    Value::I32(v)
}

fn rows(data: &[&[i32]]) -> Vec<Vec<Value>> {
    data.iter()
        .map(|row| row.iter().copied().map(i).collect())
        .collect()
}

fn answer(data: &[&[Value]]) -> TypedRows {
    data.iter().map(|row| row.to_vec()).collect()
}

fn query(schema: Vec<Relation>, atoms: Vec<Atom>, head: Vec<Expr>) -> Query {
    Query {
        schema,
        atoms,
        negations: Vec::new(),
        bindings: Vec::new(),
        guards: Vec::new(),
        constraints: Vec::new(),
        blocks: Vec::new(),
        head,
        folds: Vec::new(),
        order: Vec::new(),
        raw_names: false,
    }
}

fn atom(relation: usize, terms: Vec<Term>) -> Atom {
    Atom { relation, terms }
}

fn check(case: &Case) {
    let program: cq_testkit::cq::Program =
        syn::parse_str(&case.cq()).unwrap_or_else(|error| panic!("{error}\n{}", case.cq()));
    cq_testkit::cq::contract::check(&program)
        .unwrap_or_else(|error| panic!("{error}\n{}", case.cq()));
}

/// R(x, y), S(y): a two-relation join used by several cases.
fn join_case(r: &[&[i32]], s: &[&[i32]]) -> Case {
    Case {
        seed: 0,
        query: query(
            vec![Relation::i32("r0", 2), Relation::i32("r1", 1)],
            vec![
                atom(0, vec![Term::Var(0), Term::Var(1)]),
                atom(1, vec![Term::Var(1)]),
            ],
            vec![Expr::Var(0)],
        ),
        data: vec![rows(r), rows(s)],
    }
}

#[test]
fn reference_joins_under_set_semantics_with_duplicates() {
    let case = join_case(&[&[1, 2], &[1, 2], &[3, 4], &[5, 9]], &[&[2], &[4], &[4]]);
    assert_eq!(case.reference(), answer(&[&[i(1)], &[i(3)]]));
    check(&case);
}

#[test]
fn reference_negation_is_an_anti_join_with_wildcards_and_constants() {
    let mut case = join_case(&[&[1, 2], &[3, 4], &[5, 4]], &[&[2], &[4]]);
    // !r0(x, 4) removes x = 3 and x = 5; then !r1(_) on a nonempty r1 removes all.
    case.query.negations = vec![atom(0, vec![Term::Var(0), Term::Constant(i(4))])];
    assert_eq!(case.reference(), answer(&[&[i(1)]]));
    check(&case);
    case.query.negations.push(atom(1, vec![Term::Wildcard]));
    assert!(case.reference().is_empty());
}

#[test]
fn reference_folds_consume_distinct_valuations_per_group() {
    let mut case = Case {
        seed: 0,
        query: query(
            vec![Relation::i32("r0", 3)],
            vec![atom(0, vec![Term::Var(0), Term::Var(1), Term::Wildcard])],
            vec![Expr::Var(0)],
        ),
        // (1,5,_) twice with different wildcards is one valuation.
        data: vec![rows(&[&[1, 5, 0], &[1, 5, 1], &[1, -2, 0], &[2, 7, 0]])],
    };
    case.query.folds = vec![
        Fold {
            kind: FoldKind::Count,
            argument: None,
        },
        Fold {
            kind: FoldKind::Sum,
            argument: Some(FoldArg::Var(1)),
        },
        Fold {
            kind: FoldKind::Min,
            argument: Some(FoldArg::Var(1)),
        },
        Fold {
            kind: FoldKind::Max,
            argument: Some(FoldArg::Var(1)),
        },
        Fold {
            kind: FoldKind::Sum,
            argument: Some(FoldArg::Parameter(i(3))),
        },
    ];
    let u = Value::Usize;
    assert_eq!(
        case.reference(),
        answer(&[
            &[i(1), u(2), i(3), i(-2), i(5), i(6)],
            &[i(2), u(1), i(7), i(7), i(7), i(3)],
        ])
    );
    let types = case.output_types();
    assert_eq!(types[1].column, ColumnType::Usize);
    assert!(types[3].optional && types[4].optional && !types[2].optional);
    assert!(case.parameters().values().any(|v| *v == i(3)));
    check(&case);
}

#[test]
fn reference_union_and_fallback_blocks_project_branch_locals() {
    // r0(x), { r1(x, y) } else { r2(x, y) } -> fallback picks the first nonempty
    // branch per x; union takes both.
    let mut case = Case {
        seed: 0,
        query: query(
            vec![
                Relation::i32("r0", 1),
                Relation::i32("r1", 3),
                Relation::i32("r2", 3),
            ],
            vec![
                atom(0, vec![Term::Var(0)]),
                atom(1, vec![Term::Var(0), Term::Var(1), Term::Var(2)]),
                atom(2, vec![Term::Var(0), Term::Var(1), Term::Var(3)]),
            ],
            vec![Expr::Var(0), Expr::Var(1)],
        ),
        data: vec![
            rows(&[&[1], &[2]]),
            rows(&[&[1, 10, 0]]),
            rows(&[&[1, 11, 0], &[2, 12, 0]]),
        ],
    };
    case.query.blocks = vec![Block {
        kind: BlockKind::Fallback,
        branches: vec![vec![Step::Atom(1)], vec![Step::Atom(2)]],
        interface: vec![1],
    }];
    case.query.order = vec![Step::Atom(0), Step::Block(0)];
    assert_eq!(case.reference(), answer(&[&[i(1), i(10)], &[i(2), i(12)]]));
    check(&case);
    case.query.blocks[0].kind = BlockKind::Union;
    assert_eq!(
        case.reference(),
        answer(&[&[i(1), i(10)], &[i(1), i(11)], &[i(2), i(12)]])
    );
}

#[test]
fn reference_evaluates_generators_and_opaque_rust_like_the_printed_rust() {
    let mut case = generator_case(Generator::Duplicates);
    assert_eq!(case.reference(), answer(&[&[i(2)], &[i(0)]]));
    case.query.bindings[0].generator = Some(Generator::InclusiveRange);
    case.query.bindings[0].expression = Expr::Rust(RustOp::Divide, Box::new(Expr::Var(0)));
    // 10 / ((2 % 3) + 4) = 1; 0..=1.
    assert_eq!(case.reference(), answer(&[&[i(0)], &[i(1)]]));
    case.query.bindings[0].generator = Some(Generator::Empty);
    assert!(case.reference().is_empty());
}

#[test]
fn reference_keeps_i64_values_that_agree_in_the_low_32_bits_apart() {
    let wide = |v: i64| Value::I64(v);
    let case = Case {
        seed: 0,
        query: query(
            vec![
                Relation {
                    name: "r0".into(),
                    types: vec![ColumnType::I64, ColumnType::I64],
                    key: vec![1, 0],
                },
                Relation {
                    name: "r1".into(),
                    types: vec![ColumnType::I64],
                    key: Vec::new(),
                },
            ],
            vec![
                atom(0, vec![Term::Var(0), Term::Var(1)]),
                atom(1, vec![Term::Var(1)]),
            ],
            vec![Expr::Var(0)],
        ),
        data: vec![
            vec![
                vec![wide(1), wide(1 << 32)],
                vec![wide(2), wide(0)],
                vec![wide(i64::MIN), wide(i64::MAX)],
            ],
            vec![vec![wide(0)], vec![wide(i64::MAX)]],
        ],
    };
    assert_eq!(case.reference(), answer(&[&[wide(2)], &[wide(i64::MIN)]]));
    assert!(case.cq().contains("c0: i64"));
    check(&case);
    let features = case.features();
    for tag in [
        "low32-collision",
        "extreme-values",
        "multi-column-key",
        "unsorted-key",
    ] {
        assert!(features.contains(tag), "{tag}");
    }
    // Storing columns in key order keeps the answer.
    let reordered = case.reorder_columns(0, &[1, 0]);
    assert_eq!(reordered.query.schema[0].key, vec![0, 1]);
    assert_eq!(reordered.reference(), case.reference());
}

#[test]
fn reference_contradictory_and_point_ranges() {
    let mut case = join_case(&[&[1, 2], &[3, 2], &[2, 2]], &[&[2]]);
    case.query.constraints = vec![
        Relational::Compare(Comparison::Less, RelTerm::Var(0), RelTerm::Literal(i(2))),
        Relational::Compare(Comparison::Equal, RelTerm::Var(0), RelTerm::Literal(i(3))),
    ];
    assert!(case.reference().is_empty());
    assert!(case.features().contains("contradictory-constraints"));
    check(&case);
    case.query.constraints = vec![Relational::Between(
        RelTerm::Var(0),
        RelTerm::Literal(i(3)),
        RelTerm::Parameter(i(3)),
    )];
    assert_eq!(case.reference(), answer(&[&[i(3)]]));
    assert!(case.features().contains("one-point-range"));
    case.query.constraints = vec![Relational::Or(vec![
        vec![Relational::In(RelTerm::Var(0), vec![i(1), i(9)])],
        vec![Relational::Compare(
            Comparison::Greater,
            RelTerm::Var(0),
            RelTerm::Var(1),
        )],
    ])];
    assert_eq!(case.reference(), answer(&[&[i(1)], &[i(3)]]));
}

#[test]
fn closed_captures_unit_heads_and_raw_identifiers_print_and_evaluate() {
    let mut case = join_case(&[&[1, 2], &[3, 2]], &[&[2]]);
    case.query.raw_names = true;
    case.query.schema[0].name = "r#match".into();
    case.query.bindings = vec![Binding {
        variable: 3,
        expression: Expr::Binary(
            cq_testkit::fuzz::Arithmetic::Add,
            Box::new(Expr::Parameter(2)),
            Box::new(Expr::Constant(1)),
        ),
        generator: None,
    }];
    case.query.guards = vec![Guard::Compare(
        Comparison::Less,
        Expr::Parameter(-1),
        Expr::Var(1),
    )];
    case.query.head = vec![Expr::Var(1), Expr::Var(3), Expr::Parameter(-1)];
    assert_eq!(case.reference(), answer(&[&[i(2), i(3), i(-1)]]));
    assert!(case.cq().contains("r#type"));
    check(&case);
    let features = case.features();
    for tag in ["closed-let", "head-constant", "raw-identifier"] {
        assert!(features.contains(tag), "{tag}");
    }
    case.data[1].clear();
    assert!(case.reference().is_empty());
}

#[test]
#[ignore = "generated sweep: cargo test -p cq-testkit -- --ignored"]
fn profiles_restrict_generated_features() {
    for seed in 0..64 {
        let case = generate_with(seed, &Profile::gpu());
        assert!(!case.requires_rust_oracle(), "seed {seed}");
        assert!(case.query.blocks.is_empty(), "seed {seed}");
        let quick = generate_with(seed, &Profile::quick());
        assert!(
            quick.data.iter().all(|rows| rows.len() <= 64),
            "seed {seed}"
        );
        for profile in [Profile::quick(), Profile::full(), Profile::gpu()] {
            let case = generate_with(seed, &profile);
            assert!(
                case.query
                    .schema
                    .iter()
                    .flat_map(|r| &r.types)
                    .all(|t| *t != ColumnType::Str)
            );
        }
    }
}

#[test]
fn route_coverage_records_behavior() {
    let case = join_case(&[&[1, 2]], &[&[2]]);
    let mut coverage = RouteCoverage::default();
    coverage.record("default", &case, true);
    coverage.record("par", &case, false);
    assert_eq!(coverage.count("atoms=2", "default"), (1, 0));
    assert_eq!(coverage.count("atoms=2", "par"), (0, 1));
    assert_eq!(coverage.gaps(&["negation"]).len(), 2);
    assert!(coverage.gaps(&["atoms=2"]).is_empty());
}

fn generator_case(generator: Generator) -> Case {
    let mut query = query(
        vec![Relation::i32("r0", 1), Relation::i32("r1", 1)],
        vec![atom(0, vec![Term::Var(0)]), atom(1, vec![Term::Var(2)])],
        vec![Expr::Var(1)],
    );
    query.bindings = vec![Binding {
        variable: 1,
        expression: Expr::Var(0),
        generator: Some(generator),
    }];
    query.order = vec![Step::Atom(0), Step::Binding(0), Step::Atom(1)];
    Case {
        seed: 0,
        query,
        data: vec![rows(&[&[2]]), rows(&[&[3]])],
    }
}

#[test]
fn every_generator_and_opaque_rust_operation_routes_away_from_sql() {
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
            let program: cq_testkit::cq::Program = syn::parse_str(&case.cq()).unwrap();
            cq_testkit::cq::contract::check(&program).unwrap();
            assert!(matches!(
                &program.query.body[1],
                cq_testkit::cq::BodyItem::For(_)
            ));
            case.reference();
        }
    }
}

#[test]
fn generator_minimization_keeps_captures_and_nested_binding_scope() {
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
    check(&smaller);
    assert!(smaller.requires_rust_oracle());
}

#[test]
fn literal_atoms_keep_source_positions_and_resolve_local_dependencies() {
    let case = Case {
        seed: 0,
        query: query(
            vec![Relation::i32("r0", 3)],
            vec![atom(
                0,
                vec![
                    Term::Expression(Expr::Negate(Box::new(Expr::Var(0)))),
                    Term::Var(0),
                    Term::Constant(i(5)),
                ],
            )],
            vec![Expr::Var(0)],
        ),
        data: vec![rows(&[&[-2, 2, 5], &[-3, 3, 6], &[3, 3, 5]])],
    };
    let program: cq_testkit::cq::Program = syn::parse_str(&case.cq()).unwrap();
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
    assert_eq!(case.reference(), answer(&[&[i(2)]]));
}

#[test]
fn relational_constraints_stay_relational_and_rust_guards_stay_opaque() {
    let mut case = generator_case(Generator::Range);
    case.query.bindings.clear();
    case.query.head = vec![Expr::Var(0)];
    case.query.constraints = vec![Relational::Or(vec![
        vec![
            Relational::Between(
                RelTerm::Var(0),
                RelTerm::Literal(i(-2)),
                RelTerm::Parameter(i(3)),
            ),
            Relational::Compare(
                Comparison::NotEqual,
                RelTerm::Var(0),
                RelTerm::Literal(i(0)),
            ),
        ],
        vec![Relational::In(RelTerm::Var(0), vec![i(5), i(7)])],
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
    let program: cq_testkit::cq::Program = syn::parse_str(&case.cq()).unwrap();
    cq_testkit::cq::contract::check(&program).unwrap();
    assert_eq!(program.query.body.len(), 4);
    assert!(matches!(
        &program.query.body[1],
        cq_testkit::cq::BodyItem::Constraint(_)
    ));
    assert!(matches!(
        &program.query.body[2],
        cq_testkit::cq::BodyItem::Filter(_)
    ));
    // x = 2: in [-2, 3], != 0, and opaque 2 > 2 fails but 2 == 1 fails: empty.
    assert!(case.reference().is_empty());
    case.data[0] = rows(&[&[1], &[3], &[7]]);
    assert_eq!(case.reference(), answer(&[&[i(1)], &[i(3)], &[i(7)]]));
}

// ----- Generated sweeps (opt-in) -----

#[test]
#[ignore = "generated sweep: cargo test -p cq-testkit -- --ignored"]
fn sweep_generator_is_seeded_and_covers_every_tag() {
    let mut coverage = Coverage::default();
    for seed in 0..2048 {
        let first = generate(seed);
        assert_eq!(first, generate(seed));
        assert_eq!(first.cq(), generate(seed).cq());
        assert_eq!(first.sql(), generate(seed).sql());
        coverage.observe(&first);
    }
    assert_eq!(coverage.count("cases"), 2048);
    let advertised: BTreeSet<_> = FEATURE_TAGS.iter().copied().collect();
    let observed: BTreeSet<_> = coverage.counts().keys().map(String::as_str).collect();
    assert_eq!(
        advertised
            .symmetric_difference(&observed)
            .collect::<Vec<_>>(),
        Vec::<&&str>::new(),
        "tag inventory must be complete"
    );
    for feature in FEATURE_TAGS {
        assert!(coverage.count(feature) >= 10, "{feature}: {coverage}");
    }
}

#[test]
#[ignore = "generated sweep: cargo test -p cq-testkit -- --ignored"]
fn sweep_seventy_percent_of_reference_answers_are_nonempty() {
    for profile in [Profile::quick(), Profile::full(), Profile::gpu()] {
        // Per lane: (nonempty, cases).
        let mut lanes = std::collections::BTreeMap::<&str, (usize, usize)>::new();
        for seed in 0..1000 {
            let case = generate_with(seed, &profile);
            let lane = if !case.query.blocks.is_empty() {
                "block"
            } else if case.requires_rust_oracle() {
                "rust"
            } else {
                "relational"
            };
            let entry = lanes.entry(lane).or_default();
            entry.0 += usize::from(!case.reference().is_empty());
            entry.1 += 1;
        }
        let nonempty = lanes.values().map(|(n, _)| n).sum::<usize>();
        eprintln!("{profile:?}: {nonempty}/1000 nonempty; per lane {lanes:?}");
        assert!(nonempty >= 700, "{profile:?}: {nonempty}/1000 nonempty");
    }
}

#[test]
#[ignore = "generated sweep: cargo test -p cq-testkit -- --ignored"]
fn sweep_large_seeds_preserve_determinism_and_rust_oracle_lane() {
    for seed in [u32::MAX as u64 + 1, 1 << 63, u64::MAX - 1, u64::MAX] {
        let case = generate(seed);
        assert_eq!(case, generate(seed));
        assert_eq!(case.requires_rust_oracle(), seed & 1 == 0);
    }
}
