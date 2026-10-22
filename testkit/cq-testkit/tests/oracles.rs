use cq_testkit::{CASES, SCENARIOS, cq_semantics, databases, semantics, valuation_semantics};

#[test]
fn cq_evaluator_reproduces_every_authored_case() {
    for case in CASES {
        assert_eq!(
            cq_semantics::evaluate_cq(&case.parse_cq().unwrap(), &case.database()),
            case.expected_rows(),
            "{}",
            case.name,
        );
    }
}

#[test]
fn ra_evaluator_reproduces_every_scenario_including_alternative_plans() {
    for scenario in SCENARIOS {
        let case = scenario.case().unwrap();
        assert_eq!(
            semantics::evaluate_ra(&scenario.parse_ra().unwrap(), &case.database()).output,
            case.expected_rows(),
            "{}",
            scenario.name,
        );
    }
}

#[test]
fn valuation_evaluator_reproduces_every_scenario_including_reductions() {
    for scenario in SCENARIOS {
        let case = scenario.case().unwrap();
        assert_eq!(
            valuation_semantics::evaluate(&scenario.parse_vo().unwrap(), &case.database()),
            case.expected_rows(),
            "{}",
            scenario.name,
        );
    }
}

#[test]
fn binary_database_has_deterministic_empty_full_and_sampled_seeds() {
    for scenario in SCENARIOS {
        let source = scenario.parse_cq().unwrap();
        let empty = databases::binary_database(&source, 0);
        assert!(empty.values().all(|rows| rows.is_empty()));
        let full = databases::binary_database(&source, 1);
        for declaration in source.inputs() {
            let rows = &full[&cq_testkit::cq::symbol_name(&declaration.name)];
            assert_eq!(rows.len(), 1 << declaration.columns.len());
        }
        assert_eq!(
            databases::binary_database(&source, 37),
            databases::binary_database(&source, 37),
        );
    }
}

#[test]
fn authored_block_programs_check_at_every_stage_and_match_all_oracles() {
    for scenario in cq_testkit::blocks::SCENARIOS {
        let cq = scenario.parse_cq().unwrap();
        cq_testkit::cq::contract::check(&cq).unwrap();
        let vo = scenario.parse_vo().unwrap();
        cq_testkit::vo::contract::check(&vo).unwrap();
        let ra = scenario.parse_ra().unwrap();
        cq_testkit::ra::contract::check(&ra).unwrap();
        let ix = scenario.parse_ix().unwrap();
        cq_testkit::ix::contract::check(&ix).unwrap();
        let physical = scenario.parse_physical().unwrap();
        cq_testkit::physical::contract::check(&physical).unwrap();
        let structures = scenario.parse_structures().unwrap();
        cq_testkit::structures::contract::check(&structures).unwrap();
        let case = scenario.case().unwrap();
        let database = case.database();
        assert_eq!(
            cq_testkit::cq_semantics::evaluate_cq(&cq, &database),
            case.expected_rows()
        );
        assert_eq!(
            cq_testkit::valuation_semantics::evaluate(&vo, &database),
            case.expected_rows()
        );
        assert_eq!(
            cq_testkit::semantics::evaluate_ra(&ra, &database).output,
            case.expected_rows()
        );
    }
}

#[test]
#[should_panic(expected = "generators use compiled written-order Rust")]
fn integer_oracle_does_not_interpret_rust_generators() {
    let program = syn::parse_str(
        "struct Q; relation Seed(x: i32); relation answer(x: i32); answer(x) :- Seed(x), for y in 0i32..2;"
    ).unwrap();
    let database = cq_testkit::Database::from([("Seed".into(), Default::default())]);
    cq_semantics::evaluate_cq(&program, &database);
}

#[test]
fn cq_literals_filter_original_positions_without_binding_columns() {
    use cq_testkit::{Database, Rows};
    let database = Database::from([(
        "R".into(),
        Rows::from([
            vec![5, 1, -2],
            vec![5, 2, -2],
            vec![6, 3, -2],
            vec![5, 4, -1],
        ]),
    )]);
    let program = syn::parse_str(
        "struct Q; relation R(tag:i32,payload:i32,code:i32);
         answer(x) :- R(5,x,-2);",
    )
    .unwrap();
    let relation = cq_semantics::valuations(&program, &database);
    assert_eq!(relation.heading, ["x".to_owned()].into());
    assert_eq!(
        cq_semantics::evaluate_cq(&program, &database),
        Rows::from([vec![1], vec![2]])
    );

    for (atom, expected) in [
        ("R(5,1,-2)", Rows::from([vec![]])),
        ("R(5,1,-1)", Rows::new()),
    ] {
        let program = syn::parse_str(&format!(
            "struct Q; relation R(tag:i32,payload:i32,code:i32);
             answer() :- {atom};"
        ))
        .unwrap();
        assert!(
            cq_semantics::valuations(&program, &database)
                .heading
                .is_empty()
        );
        assert_eq!(cq_semantics::evaluate_cq(&program, &database), expected);
    }
}

#[test]
fn ra_occurrence_constants_use_source_names_and_drop_literal_headings() {
    use cq_testkit::{Database, Rows};
    let program = syn::parse_quote! {
        relational {
            relation R(tag:i32,payload:i32,code:i32);
            r = rename R@0 {payload -> tag} constants {tag = 5, code = -2};
            witness = rename R@1 {} constants {tag = 5, payload = 1, code = -2};
            result = natural_join r with witness;
            output result as answer(tag);
        }
    };
    let database = Database::from([(
        "R".into(),
        Rows::from([
            vec![5, 1, -2],
            vec![5, 2, -2],
            vec![6, 3, -2],
            vec![5, 4, -1],
        ]),
    )]);
    let evaluation = semantics::evaluate_ra(&program, &database);
    assert_eq!(evaluation.output, Rows::from([vec![1], vec![2]]));
    assert_eq!(
        evaluation.definitions["r"].heading,
        ["tag".to_owned()].into()
    );
    assert!(evaluation.definitions["witness"].heading.is_empty());
    assert_eq!(evaluation.definitions["witness"].rows.len(), 1);

    let missing = Database::from([("R".into(), Rows::from([vec![5, 2, -2]]))]);
    let evaluation = semantics::evaluate_ra(&program, &missing);
    assert!(evaluation.definitions["witness"].rows.is_empty());
    assert!(evaluation.output.is_empty());
}

#[test]
fn vo_literals_filter_before_fine_projections_and_reductions() {
    use cq_testkit::{Database, Rows};
    let program = syn::parse_quote! {
        struct Q;
        relation R(tag:i32,payload:i32,code:i32);
        relation S(tag:i32,payload:i32);
        answer(x) :- R(5,x,-2), S(7,x), R(5,1,-2);
        valuation {
            reduce R@0 with S@1(x);
            R@0(_);
            R@0(x);
            S@1(x);
            R@2();
        }
    };
    let database = Database::from([
        (
            "R".into(),
            Rows::from([
                vec![5, 1, -2],
                vec![5, 2, -2],
                vec![6, 3, -2],
                vec![5, 4, -1],
            ]),
        ),
        (
            "S".into(),
            Rows::from([vec![7, 2], vec![8, 1], vec![7, 3], vec![7, 4]]),
        ),
    ]);
    assert_eq!(
        valuation_semantics::evaluate(&program, &database),
        Rows::from([vec![2]])
    );
    let mut missing = database;
    missing.get_mut("R").unwrap().remove(&vec![5, 1, -2]);
    assert!(valuation_semantics::evaluate(&program, &missing).is_empty());
}
