use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use cq_workloads::{Scenario, extended::CASES};

#[test]
fn authored_callbacks_schemas_and_sql_cover_all_families() {
    let mut counts = BTreeMap::new();
    let mut names = BTreeSet::new();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    for case in CASES {
        assert!(names.insert(case.name));
        *counts.entry(case.family).or_insert(0) += 1;
        let file =
            syn::parse_file(&fs::read_to_string(root.join(case.rust_path)).unwrap()).unwrap();
        assert_eq!(file.items.len(), 1);
        let syn::Item::Macro(invocation) = &file.items[0] else {
            panic!("authored callback")
        };
        assert_eq!(
            invocation.mac.path,
            syn::parse_quote!(crate::extended_query)
        );
        let Some(proc_macro2::TokenTree::Group(group)) =
            invocation.mac.tokens.clone().into_iter().last()
        else {
            panic!("program block")
        };
        assert_eq!(
            group.stream().to_string(),
            case.source
                .parse::<proc_macro2::TokenStream>()
                .unwrap()
                .to_string()
        );
        assert!(!case.schema().unwrap().is_empty());
        assert_eq!(
            case.sql,
            fs::read_to_string(
                root.join("workloads/sql/extended")
                    .join(format!("{}.sql", case.name))
            )
            .unwrap()
        );
        let operator = match case.family {
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
            ("union", 3),
            ("negation", 3),
            ("aggregate", 3),
            ("recursion", 3),
            ("correlated", 3),
            ("stacked", 2)
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
    assert_eq!(scenarios, 51);
}
