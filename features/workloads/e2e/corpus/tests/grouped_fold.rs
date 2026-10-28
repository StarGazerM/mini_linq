use cq_testkit::conformance::assert_answers_equal;
use cq_workloads::{
    GenerationConfig, Scale, Scenario, Suite,
    dataset::{Dataset, RelationData},
    schema::Inputs,
};

type Execute = fn(&mut Inputs) -> Vec<String>;

fn inputs(name: &str, arity: usize, rows: &[Vec<i32>]) -> Inputs {
    let columns = (0..arity)
        .map(|i| format!("c{i}: i32"))
        .collect::<Vec<_>>()
        .join(",");
    let schema = syn::parse_str(&format!("relation {name}({columns});")).unwrap();
    Inputs::from_dataset(
        Dataset::new_schema(
            &[schema],
            Suite::Classic,
            "grouped-fold",
            GenerationConfig {
                scenario: if rows.is_empty() {
                    Scenario::EmptyInput
                } else {
                    Scenario::Coverage
                },
                scale: Scale::Tiny,
                seed: 0,
            },
            vec![RelationData::new(name, arity, rows.to_vec()).unwrap()],
        )
        .unwrap(),
    )
}

fn check(
    targets: (Execute, Execute, bool),
    name: &str,
    arity: usize,
    rows: &[Vec<i32>],
    expected: &[&str],
) {
    let (sequential, tasks, canonical) = targets;
    let expected = expected
        .iter()
        .map(|row| (*row).to_owned())
        .collect::<Vec<_>>();
    let reference = sequential(&mut inputs(name, arity, rows));
    let actual = tasks(&mut inputs(name, arity, rows));
    assert_answers_equal(
        "authored aggregate answer",
        expected,
        "Sequential",
        reference.clone(),
        true,
    );
    assert_answers_equal("Sequential", reference, "Tasks", actual, canonical);
}

#[test]
fn grouped_count_sum_keep_equal_values_from_distinct_matches() {
    let targets = mini_linq::compile! {
        pipeline { print_input = structures; print = cq_workload_passes::sequence_targets(take); }
        struct Query;
        relation R(k: i32, id: i32, v: i32);
        relation answer(k: i32, n: usize, total: i32);
        answer(k, count(), sum(v)) :- R(k, id, v);
    };
    let rows = vec![
        vec![1, 10, 7],
        vec![1, 10, 7],
        vec![1, 11, 7],
        vec![1, 12, 3],
        vec![2, 20, 9],
    ];
    check(targets, "R", 3, &rows, &["(1, 3, 17)", "(2, 1, 9)"]);
    check(
        targets,
        "R",
        3,
        &rows.into_iter().rev().collect::<Vec<_>>(),
        &["(1, 3, 17)", "(2, 1, 9)"],
    );
    check(targets, "R", 3, &[], &[]);
}

#[test]
fn global_empty_group_has_monoid_identities() {
    let targets = mini_linq::compile! {
        pipeline { print_input = structures; print = cq_workload_passes::sequence_targets(take); }
        struct Query;
        relation R(id: i32, v: i32);
        relation answer(n: usize, total: i32, low: Option<i32>, high: Option<i32>);
        answer(count(), sum(v), min(v), max(v)) :- R(id, v);
    };
    check(targets, "R", 2, &[], &["(0, 0, None, None)"]);
    check(
        targets,
        "R",
        2,
        &[vec![1, 5], vec![2, 5], vec![2, 5], vec![3, -2]],
        &["(3, 8, Some(-2), Some(5))"],
    );
}

#[test]
fn anonymous_witnesses_are_projected_before_the_fold() {
    let targets = mini_linq::compile! {
        pipeline { print_input = structures; print = cq_workload_passes::sequence_targets(take); }
        struct Query;
        relation R(k: i32, witness: i32, v: i32);
        relation answer(k: i32, n: usize, total: i32);
        answer(k, count(), sum(v)) :- R(k, _, v);
    };
    check(
        targets,
        "R",
        3,
        &[vec![1, 10, 7], vec![1, 11, 7], vec![1, 12, 3]],
        &["(1, 2, 10)"],
    );
}

#[test]
fn aggregate_expressions_share_the_full_named_valuation_domain() {
    let targets = mini_linq::compile! {
        pipeline { print_input = structures; print = cq_workload_passes::sequence_targets(take); }
        struct Query;
        relation R(k: i32, id: i32, v: i32);
        relation answer(k: i32, total: i32, low: Option<i32>, high: Option<i32>);
        answer(k, sum(*v * 2), min(v), max(v)) :- R(k, id, v);
    };
    check(
        targets,
        "R",
        3,
        &[vec![1, 10, 7], vec![1, 11, 7], vec![1, 11, 7]],
        &["(1, 28, Some(7), Some(7))"],
    );
}

#[test]
fn triangle_count_is_a_head_aggregate() {
    let targets = mini_linq::compile! {
        pipeline { print_input = structures; print = cq_workload_passes::sequence_targets(take); }
        struct Triangles;
        relation Edge(a: i32, b: i32);
        relation answer(n: usize);
        answer(count()) :- Edge(x,y), Edge(y,z), Edge(z,x);
    };
    check(
        targets,
        "Edge",
        2,
        &[vec![1, 2], vec![2, 3], vec![3, 1], vec![1, 2]],
        &["(3,)"],
    );
    check(targets, "Edge", 2, &[vec![1, 2], vec![2, 3]], &["(0,)"]);
    check(targets, "Edge", 2, &[], &["(0,)"]);
}

#[test]
fn count_only_group_retains_non_head_join_witnesses() {
    let targets = mini_linq::compile! {
        pipeline { print_input = structures; print = cq_workload_passes::sequence_targets(take); }
        struct Query;
        relation Edge(a: i32, b: i32);
        relation answer(x: i32, n: usize);
        answer(x, count()) :- Edge(x,y), Edge(y,z);
    };
    check(
        targets,
        "Edge",
        2,
        &[vec![1, 2], vec![2, 3], vec![2, 4], vec![2, 4]],
        &["(1, 2)"],
    );
}

#[test]
fn global_count_of_existential_body_has_unit_domain() {
    let targets = mini_linq::compile! {
        pipeline { print_input = structures; print = cq_workload_passes::sequence_targets(take); }
        struct Query;
        relation R(witness: i32);
        relation answer(n: usize);
        answer(count()) :- R(_);
    };
    check(targets, "R", 1, &[vec![1], vec![2], vec![2]], &["(1,)"]);
    check(targets, "R", 1, &[], &["(0,)"]);
}

#[test]
fn catalog_triangle_rewritten_as_count_matches_its_duckdb_goldens() {
    let (sequential, tasks, canonical) = mini_linq::compile! {
        pipeline { print_input = structures; print = cq_workload_passes::sequence_targets(take); }
        struct TriangleCount;
        relation R(c0: i32, c1: i32);
        relation S(c0: i32, c1: i32);
        relation T(c0: i32, c1: i32);
        relation answer(n: usize);
        answer(count()) :- R(x,y), S(y,z), T(z,x);
    };
    for scenario in Scenario::ALL {
        let golden = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../golden/classic/triangle")
            .join(scenario.as_str())
            .join("tiny.csv");
        let rows = cq_workloads::dataset::read_result_rows(golden, 3).unwrap();
        let expected = vec![format!("({},)", rows.len())];
        let reference = sequential(&mut cq_workloads::execution::inputs("triangle", scenario));
        let actual = tasks(&mut cq_workloads::execution::inputs("triangle", scenario));
        assert_answers_equal(
            "DuckDB triangle cardinality",
            expected,
            "Sequential",
            reference.clone(),
            true,
        );
        assert_answers_equal("Sequential", reference, "Tasks", actual, canonical);
    }
}
