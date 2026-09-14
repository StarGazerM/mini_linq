//! Stage I's independent semantic reference gate.
//! Hand-authored RA is compared with valuations and fixed expected answers.
//! The companion stage1_production suite checks compiled Rust execution.

#[path = "stage1_reference/cases.rs"]
mod cases;
#[path = "stage1_reference/contracts.rs"]
mod contracts;
#[path = "stage1_reference/oracle.rs"]
mod oracle;

#[path = "stage1_reference/verifier.rs"]
mod planned_ra;

use cq_ir::{cq, relational_plan};
use oracle::{Database, Rows, evaluate_cq, evaluate_ra, valuations};
use quote::ToTokens;

fn parse(text: &str) -> relational_plan::Module {
    let module = syn::parse_str(text).unwrap();
    relational_plan::contract::check(&module).unwrap();
    module
}

fn database(case: &cases::Case) -> Database {
    case.inputs
        .iter()
        .map(|(name, rows)| {
            (
                (*name).to_owned(),
                rows.iter().map(|row| row.to_vec()).collect(),
            )
        })
        .collect()
}

fn case(name: &str) -> &'static cases::Case {
    cases::CASES.iter().find(|case| case.name == name).unwrap()
}

fn assert_known(name: &str) {
    let fixture = case(name);
    let module = parse(fixture.plan);
    contracts::assert_verified_reference(name, &module);
    let reparsed = parse(&module.to_token_stream().to_string());
    assert_eq!(module, reparsed);
    let db = database(fixture);
    let expected = fixture
        .expected
        .iter()
        .map(|row| row.to_vec())
        .collect::<Rows>();
    assert_eq!(
        evaluate_cq(&module.program, &db),
        expected,
        "{name}: CQ vs hand-written answers"
    );
    assert_eq!(
        evaluate_ra(&module, &db).output,
        expected,
        "{name}: RA vs hand-written answers"
    );
}

macro_rules! known_cases {
    ($($name:ident),* $(,)?) => { $(
        #[test]
        fn $name() { assert_known(stringify!($name)); }
    )* };
}

known_cases!(
    scan,
    projection,
    self_chain,
    return_trip,
    intersection,
    unary_guard,
    chain_projection,
    claw,
    fact_star,
    triangle,
    cycle4,
    cartesian,
    existential,
    ternary_guard,
    composite,
    bushy,
    ya_delivery,
    oriented_chain,
    spade,
    barbell,
    loomis_whitney_4,
    ya_deep,
    ya_composite,
    ya_disconnected,
    cartesian_first,
);

fn binary_database(program: &cq::Program, seed: u64) -> Database {
    let mut random = seed;
    program
        .inputs()
        .map(|input| {
            let rows = (0_usize..(1 << input.columns.len()))
                .filter_map(|bits| {
                    random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
                    let include = match seed {
                        0 => false,
                        1 => true,
                        _ => (random >> 61) <= seed % 8,
                    };
                    include.then(|| {
                        (0..input.columns.len())
                            .map(|column| ((bits >> column) & 1) as i32)
                            .collect()
                    })
                })
                .collect();
            (cq_ir::symbol_name(&input.name), rows)
        })
        .collect()
}

#[test]
fn all_known_plans_and_r1_logical_baselines_match_valuations_on_generated_inputs() {
    for fixture in cases::CASES {
        let module = parse(fixture.plan);
        contracts::assert_verified_reference(fixture.name, &module);
        let source = cq::Module {
            program: module.program.clone(),
        };
        let r1 = cq_compiler::compile_cq(&source).unwrap();
        for seed in 0..64 {
            let db = binary_database(&module.program, seed);
            let expected = evaluate_cq(&module.program, &db);
            assert_eq!(
                evaluate_ra(&module, &db).output,
                expected,
                "{}: known RA, seed {seed}",
                fixture.name
            );
            assert_eq!(
                evaluate_ra(&r1, &db).output,
                expected,
                "{}: R1 logical baseline, seed {seed}",
                fixture.name
            );
        }
    }
}

#[test]
fn emptying_each_used_input_annihilates_the_query_in_every_pattern() {
    for fixture in cases::CASES {
        let module = parse(fixture.plan);
        let original = database(fixture);
        for name in original.keys() {
            let mut db = original.clone();
            db.get_mut(name).unwrap().clear();
            assert!(
                evaluate_ra(&module, &db).output.is_empty(),
                "{}: empty {name}",
                fixture.name
            );
            assert!(
                evaluate_cq(&module.program, &db).is_empty(),
                "{}: empty {name}",
                fixture.name
            );
        }
    }
}

#[test]
fn all_reduction_schedules_give_each_occurrence_complete_support() {
    let schedules: &[(&str, &[&str])] = &[
        ("ya_delivery", &["d1", "s2", "n1"]),
        ("ya_deep", &["r1", "s2", "t2", "u1"]),
        ("ya_composite", &["r1", "s2", "t1"]),
        ("ya_disconnected", &["r1", "s1"]),
    ];
    for (name, reduced_names) in schedules {
        let fixture = case(name);
        let module = parse(fixture.plan);
        let mut inputs = vec![database(fixture)];
        inputs.extend((0..64).map(|seed| binary_database(&module.program, seed)));
        for db in inputs {
            let full = valuations(&module.program, &db);
            let actual = evaluate_ra(&module, &db);
            for name in *reduced_names {
                let reduced = &actual.definitions[*name];
                assert_eq!(
                    *reduced,
                    full.project(reduced.heading.clone()),
                    "{}: reduced {name}",
                    fixture.name
                );
            }
        }
    }
}

fn assert_wrong_plan_detected(name: &str, text: String, db: Database) {
    let original = parse(case(name).plan);
    let wrong = parse(&text); // It must pass local headings/references first.
    contracts::assert_rejected_reference(name, &wrong);
    assert_eq!(wrong.program, original.program);
    assert_ne!(
        evaluate_ra(&wrong, &db).output,
        evaluate_cq(&original.program, &db),
        "the semantic oracle must detect the incorrect {name} plan"
    );
}

#[test]
fn oracle_detects_forgotten_self_join_argument_order() {
    let fixture = case("return_trip");
    assert_wrong_plan_detected(
        fixture.name,
        fixture.plan.replace(
            "inward = rename Road {src -> b, dst -> a}",
            "inward = rename Road {src -> a, dst -> b}",
        ),
        database(fixture),
    );
}

#[test]
fn oracle_detects_dropping_one_component_of_a_join_key() {
    let fixture = case("composite");
    assert_wrong_plan_detected(
        fixture.name,
        fixture.plan.replace(
            "s = rename S {first -> b, second -> a, value -> y}",
            "s = rename S {first -> other, second -> a, value -> y}",
        ),
        database(fixture),
    );
}

#[test]
fn oracle_detects_premature_projection_even_when_local_ra_is_valid() {
    let fixture = case("chain_projection");
    assert_wrong_plan_detected(
        fixture.name,
        fixture.plan.replace(
            "rs = natural_join r with s;",
            "early = project r keep {a}; rs = natural_join early with s;",
        ),
        database(fixture),
    );
}

#[test]
fn oracle_detects_erasing_a_disconnected_existence_test() {
    let fixture = case("existential");
    let mut db = database(fixture);
    db.get_mut("S").unwrap().clear();
    assert_wrong_plan_detected(
        fixture.name,
        fixture
            .plan
            .replace("s = rename S {first -> y, second -> z};", "")
            .replace("exists = project s keep {};", "exists = unit;"),
        db,
    );
}

#[test]
fn oracle_detects_replacing_a_ternary_constraint_by_pairwise_support() {
    let fixture = case("ternary_guard");
    assert_wrong_plan_detected(
        fixture.name,
        fixture
            .plan
            .replace(
                "stock = rename Stock {depot -> d, item -> i, day -> t};",
                "",
            )
            .replace(
                "result = natural_join pairs with stock;",
                "result = natural_join pairs with pairs;",
            ),
        database(fixture),
    );
}

#[test]
fn full_support_check_detects_a_stale_parent_that_final_answers_cannot_detect() {
    let fixture = case("ya_delivery");
    let wrong = parse(&fixture.plan.replace(
        "down_d = natural_join d0 with s2;",
        "down_d = natural_join d0 with s0;",
    ));
    let db = database(fixture);
    let full = valuations(&wrong.program, &db);
    let result = evaluate_ra(&wrong, &db);
    assert_eq!(result.output, evaluate_cq(&wrong.program, &db));
    let reduced = &result.definitions["d1"];
    assert_ne!(*reduced, full.project(reduced.heading.clone()));
}

#[test]
fn graph_oracle_checks_all_known_alpha_acyclicity_classifications() {
    for fixture in cases::CASES {
        let module = parse(fixture.plan);
        let expected = !matches!(
            fixture.name,
            "triangle" | "cycle4" | "spade" | "barbell" | "loomis_whitney_4"
        );
        assert_eq!(
            oracle::has_join_tree(&module.program),
            expected,
            "{}",
            fixture.name
        );
    }
}

#[test]
fn duplicate_occurrences_retain_separate_nodes_and_tuple_tests() {
    let fixture = case("intersection");
    let module = parse(
        &fixture
            .plan
            .replace("S(x, y);", "R(x, y);")
            .replace("s = rename S", "s = rename R"),
    );
    assert_eq!(module.program.query.body.len(), 2);
    assert!(oracle::has_join_tree(&module.program));
    let db = database(fixture);
    assert_eq!(evaluate_cq(&module.program, &db), db["R"]);
    assert_eq!(evaluate_ra(&module, &db).output, db["R"]);
}

#[test]
fn cyclic_pairwise_support_does_not_imply_a_nonempty_complete_join() {
    let module = parse(case("triangle").plan);
    let rows = Rows::from([vec![0, 1], vec![1, 0]]);
    let db = Database::from([
        ("R".into(), rows.clone()),
        ("S".into(), rows.clone()),
        ("T".into(), rows),
    ]);
    let actual = evaluate_ra(&module, &db);
    assert!(actual.output.is_empty());
    assert!(evaluate_cq(&module.program, &db).is_empty());
    for left in ["r", "s", "t"] {
        for right in ["r", "s", "t"] {
            let l = &actual.definitions[left];
            let r = &actual.definitions[right];
            assert_eq!(l.join(r).project(l.heading.clone()), *l);
        }
    }
}

#[test]
fn deep_reduction_detects_stale_children_and_stale_ancestors() {
    let fixture = case("ya_deep");
    let db = database(fixture);
    for (before, after, inspected) in [
        (
            "up_s = natural_join s0 with t1;",
            "up_s = natural_join s0 with t0;",
            "r1",
        ),
        (
            "down_t = natural_join t1 with s2;",
            "down_t = natural_join t1 with s1;",
            "t2",
        ),
    ] {
        let wrong = parse(&fixture.plan.replace(before, after));
        let actual = evaluate_ra(&wrong, &db);
        let full = valuations(&wrong.program, &db);
        assert_eq!(actual.output, evaluate_cq(&wrong.program, &db));
        let reduced = &actual.definitions[inspected];
        assert_ne!(
            *reduced,
            full.project(reduced.heading.clone()),
            "{inspected}"
        );
    }
}

#[test]
fn connected_query_can_have_a_valid_cartesian_first_plan_with_lower_row_volume() {
    let fixture = case("cartesian_first");
    let db = database(fixture);
    let product = evaluate_ra(&parse(fixture.plan), &db);
    let connected = evaluate_ra(&parse(case("chain_projection").plan), &db);
    assert_eq!(product.output, connected.output);
    let product_volume =
        product.definitions["product"].rows.len() + product.definitions["checked"].rows.len();
    let connected_volume =
        connected.definitions["rs"].rows.len() + connected.definitions["rst"].rows.len();
    assert_eq!((product_volume, connected_volume), (2, 4));
}

#[test]
fn alpha_acyclicity_need_not_survive_removing_an_atom() {
    let mut program = parse(case("ternary_guard").plan).program;
    assert!(oracle::has_join_tree(&program));
    program.query.body = program
        .query
        .body
        .into_iter()
        .filter(|item| {
            let atom = item.positive_atom().expect("positive R1/reference fixture");
            cq_ir::symbol_name(&atom.relation) != "Stock"
        })
        .collect();
    cq::contract::check(&cq::Module {
        program: program.clone(),
    })
    .unwrap();
    assert!(!oracle::has_join_tree(&program));
}

#[test]
fn body_acyclicity_and_free_connexity_are_different_checks() {
    for (name, expected) in [("self_chain", false), ("projection", true)] {
        let program = parse(case(name).plan).program;
        assert!(oracle::has_join_tree(&program));
        let mut headings = program
            .query
            .body
            .iter()
            .map(|item| {
                let atom = item.positive_atom().expect("positive R1/reference fixture");
                atom.variables().map(cq_ir::symbol_name).collect()
            })
            .collect::<Vec<_>>();
        headings.push(
            program
                .query
                .head
                .variables()
                .map(cq_ir::symbol_name)
                .collect(),
        );
        assert_eq!(oracle::has_join_tree_for(&headings), expected, "{name}");
    }
}
