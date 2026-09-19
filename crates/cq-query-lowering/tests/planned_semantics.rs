//! Production logical planning checked against independent relational semantics.
//! These checks stop at RA; executable-provider acceptance belongs downstream.
#[allow(dead_code)]
#[path = "../../../test-support/relational/cases.rs"]
mod cases;
#[allow(dead_code)]
#[path = "../../../test-support/relational/databases.rs"]
mod databases;
#[allow(dead_code)]
#[path = "../../../test-support/relational/join_trees.rs"]
mod join_trees;
#[allow(dead_code)]
#[path = "../../../test-support/relational/semantics.rs"]
mod oracle;
use cq_ir::{cq, relational_plan as ra};
use cq_query_lowering::composition::{check_cq_plan, compile_cq_planned, explain_cq_plan};
use cq_query_lowering::{JoinPolicy as Strategy, PlanningOptions as Options};
use databases::binary_database;

#[test]
fn all_classic_patterns_preserve_answers_and_full_ya_support() {
    for case in cases::CASES {
        let known: ra::Module = syn::parse_str(case.plan).unwrap();
        let source = cq::Module {
            program: known.program,
        };
        let mut databases = vec![database(case)];
        databases.extend((0..64).map(|seed| binary_database(&source.program, seed)));
        assert_eq!(
            oracle::evaluate_cq(&source.program, &databases[0]),
            case.expected.iter().map(|row| row.to_vec()).collect()
        );
        for strategy in [
            Strategy::Written,
            Strategy::Connected,
            Strategy::GreedyWidth,
            Strategy::Auto,
        ] {
            let (ra, report) = explain_cq_plan(
                &source,
                Options {
                    strategy,
                    max_arity: None,
                },
            )
            .unwrap();
            check_cq_plan(&source, &ra, &report).unwrap();
            assert_eq!(
                report.join_tree.is_some(),
                strategy == Strategy::Auto && join_trees::has_join_tree(&source.program)
            );
            for database in &databases {
                let evaluation = oracle::evaluate_plan(source.program.inputs(), &ra.plan, database);
                assert_eq!(
                    evaluation.output,
                    oracle::evaluate_cq(&source.program, database),
                    "{} {strategy:?}",
                    case.name
                );
                if report.join_tree.is_some() {
                    assert_full_reduction(&source, &ra, database, &evaluation);
                }
            }
        }
    }
}

#[test]
fn comparisons_and_extreme_literals_preserve_independent_valuations() {
    for predicate in [
        "x == 0",
        "x != 0",
        "x < -1",
        "x <= 0",
        "x > 0",
        "x >= 1",
        "-2147483648 <= x",
        "x <= 2147483647",
        "0 == 1",
    ] {
        let source: cq::Module = syn::parse_str(&format!(
            "struct Compare; relation R(v:i32); answer(x) :- R(x), if ({predicate});"
        ))
        .unwrap();
        let database = std::collections::BTreeMap::from([(
            "R".into(),
            [-2, -1, 0, 1, 2, i32::MIN, i32::MAX]
                .into_iter()
                .map(|x| vec![x])
                .collect(),
        )]);
        let target = compile_cq_planned(&source).unwrap();
        assert_eq!(
            oracle::evaluate_ra(&target, &database).output,
            oracle::evaluate_cq(&source.program, &database),
            "{predicate}"
        );
    }
}

fn assert_full_reduction(
    source: &cq::Module,
    ra: &ra::Module,
    database: &oracle::Database,
    evaluation: &oracle::Evaluation,
) {
    use cq_ir::symbol_name;
    let mut latest = std::collections::BTreeMap::new();
    let mut owners = std::collections::BTreeMap::new();
    let mut occurrence = 0;
    // All these fixtures have positive occurrences and no predicates.
    // Semijoins end at freeze, before any assembly joins or projections.
    for definition in &ra.plan.definitions {
        let name = symbol_name(&definition.result);
        match &definition.operator {
            ra::Operator::Rename(_) => {
                owners.insert(name.clone(), occurrence);
                latest.insert(occurrence, name);
                occurrence += 1;
            }
            ra::Operator::Semijoin(join) => {
                let owner = owners[&symbol_name(&join.left)];
                owners.insert(name.clone(), owner);
                latest.insert(owner, name);
            }
            _ => break,
        }
    }
    let valuations = oracle::valuations(&source.program, database);
    for (i, heading) in source
        .program
        .query
        .body
        .iter()
        .filter_map(cq::BodyItem::atom)
        .map(|a| {
            a.variables()
                .map(cq_ir::symbol_name)
                .collect::<std::collections::BTreeSet<_>>()
        })
        .collect::<Vec<_>>()
        .into_iter()
        .enumerate()
    {
        assert_eq!(
            evaluation.definitions[&latest[&i]],
            valuations.project(heading),
            "frozen occurrence {i}"
        );
    }
}

fn database(case: &cases::Case) -> oracle::Database {
    case.inputs
        .iter()
        .map(|(name, rows)| {
            (
                name.to_string(),
                rows.iter().map(|row| row.to_vec()).collect(),
            )
        })
        .collect()
}
