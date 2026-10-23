use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use cq_workloads::{Scenario, extended::CASES, schema::ColumnType};

#[test]
fn authored_cases_schemas_and_sql_cover_all_families() {
    let mut counts = BTreeMap::new();
    let mut names = BTreeSet::new();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for case in CASES {
        assert!(names.insert(case.name));
        *counts.entry(case.family).or_insert(0) += 1;
        let schema = case.schema();
        assert_eq!(
            schema,
            cq_workload_authored::schema(root.join(case.rust_path)).unwrap(),
            "{} catalog schema differs from the literal query",
            case.name,
        );
        assert!(!schema.is_empty());
        assert_eq!(schema.len(), case.relations.len());
        let mut relation_names = BTreeSet::new();
        for (declaration, relation) in schema.iter().zip(case.relations) {
            assert!(relation_names.insert(relation.name));
            assert_eq!(declaration.name, relation.name);
            assert!(!relation.columns.is_empty());
            assert_eq!(declaration.arity(), relation.columns.len());
            let mut column_names = BTreeSet::new();
            for (column, (name, ty)) in declaration.columns.iter().zip(relation.columns) {
                assert!(column_names.insert(name));
                assert_eq!(column.name, *name);
                assert!(matches!(ty, ColumnType::I32));
                let syn::Type::Path(path) = &column.ty else {
                    panic!("integer column type");
                };
                assert!(path.qself.is_none() && path.path.is_ident("i32"));
            }
        }
        assert!(root.join(case.rust_path).is_file());
        assert_eq!(
            case.sql,
            fs::read_to_string(root.join("sql/extended").join(format!("{}.sql", case.name)))
                .unwrap()
        );
        let operator = match case.family {
            "surface" => "SELECT DISTINCT",
            "retail" => "SELECT DISTINCT",
            "union" | "stacked" => "UNION",
            "negation" => "NOT EXISTS",
            "aggregate" => "GROUP BY",
            "recursion" => "WITH RECURSIVE",
            "correlated" => {
                if case.name == "correlated_parameter" {
                    ">2"
                } else {
                    "SELECT"
                }
            }
            _ => panic!("unknown family"),
        };
        assert!(
            case.sql.contains(operator),
            "{} missing SQL operator",
            case.name
        );
    }
    assert_eq!(
        counts,
        BTreeMap::from([
            ("surface", 12),
            ("union", 3),
            ("negation", 3),
            ("aggregate", 3),
            ("recursion", 3),
            ("correlated", 3),
            ("stacked", 2),
            ("retail", 4),
        ])
    );
}

#[test]
fn tiny_fixtures_and_goldens_have_exact_provenance_and_shapes() {
    let mut scenarios = 0;
    for case in CASES {
        for scenario in Scenario::ALL {
            let tracked = case.load(scenario).unwrap();
            assert_eq!(tracked, case.generate(scenario).unwrap());
            assert_eq!(tracked.relations.len(), case.relations.len());
            for (data, relation) in tracked.relations.iter().zip(case.relations) {
                assert_eq!(data.name, relation.name);
                assert!(
                    data.rows
                        .iter()
                        .all(|row| row.len() == relation.columns.len())
                );
            }
            let rows = cq_workloads::dataset::read_result_rows(
                case.path("golden").join(scenario.as_str()).join("tiny.csv"),
                case.arity,
            )
            .unwrap();
            match scenario {
                Scenario::Coverage => assert!(!rows.is_empty(), "{} coverage", case.name),
                Scenario::NoMatch => assert!(rows.is_empty(), "{} no-match", case.name),
                Scenario::EmptyInput => assert!(tracked.relations[0].rows.is_empty()),
            }
            scenarios += 1;
        }
    }
    assert_eq!(scenarios, 99);
}

#[test]
fn retail_goldens_match_independent_existential_semantics() {
    for name in [
        "retail_h04",
        "retail_h04_not_exists",
        "retail_h16",
        "retail_h22",
    ] {
        let case = cq_workloads::extended::case(name).unwrap();
        for scenario in Scenario::ALL {
            let data = case.generate(scenario).unwrap();
            let rows = |index: usize| &data.relations[index].rows;
            let mut expected = BTreeSet::new();
            for left in rows(0) {
                match name {
                    "retail_h04" | "retail_h04_not_exists" => {
                        let exists = rows(1).iter().any(|right| right[0] == left[0]);
                        if exists == (name == "retail_h04") {
                            expected.insert(vec![left[0]]);
                        }
                    }
                    "retail_h16" => {
                        for ps in rows(1).iter().filter(|ps| ps[0] == left[0]) {
                            if !rows(2).iter().any(|bad| bad[0] == ps[1]) {
                                expected.insert(vec![left[0], ps[1]]);
                            }
                        }
                    }
                    "retail_h22" => {
                        if !rows(1).iter().any(|order| order[1] == left[0]) {
                            expected.insert(vec![left[0]]);
                        }
                    }
                    _ => unreachable!(),
                }
            }
            let golden = cq_workloads::dataset::read_result_rows(
                case.path("golden").join(scenario.as_str()).join("tiny.csv"),
                case.arity,
            )
            .unwrap();
            assert_eq!(
                golden,
                expected.into_iter().collect::<Vec<_>>(),
                "{name}/{scenario:?}"
            );
        }
    }
}
