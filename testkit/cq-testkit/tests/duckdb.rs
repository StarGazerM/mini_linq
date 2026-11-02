#![cfg(feature = "duckdb")]

use cq_testkit::{CASES, duckdb, fuzz};
use duckdb::RelationData;

#[test]
fn independently_authored_sql_reproduces_corpus_answers() {
    for (name, sql) in [
        (
            "scan",
            "SELECT DISTINCT c0 FROM \"mini-linq/raw/0\" ORDER BY 1",
        ),
        (
            "self_chain",
            "SELECT DISTINCT l.c0, r.c1 FROM \"mini-linq/raw/0\" AS l JOIN \"mini-linq/raw/0\" AS r ON l.c1 = r.c0 ORDER BY 1, 2",
        ),
        (
            "composite",
            "SELECT DISTINCT r.c2, s.c2 FROM \"mini-linq/raw/0\" AS r JOIN \"mini-linq/raw/1\" AS s ON r.c0 = s.c1 AND r.c1 = s.c0 ORDER BY 1, 2",
        ),
    ] {
        let case = CASES.iter().find(|case| case.name == name).unwrap();
        let actual = duckdb::evaluate_case(case, sql).unwrap();
        assert_eq!(
            actual.into_iter().collect::<cq_testkit::Rows>(),
            case.expected_rows(),
            "{name}"
        );
    }
}

#[test]
fn raw_oracle_preserves_duplicates_and_sql_controls_the_answer_order() {
    let inputs = [RelationData {
        name: "R".into(),
        arity: 1,
        rows: vec![vec![2], vec![-1], vec![2]],
    }];
    assert_eq!(
        duckdb::evaluate_sql(&inputs, "SELECT c0 FROM \"mini-linq/raw/0\" ORDER BY c0").unwrap(),
        vec![vec![-1], vec![2], vec![2]]
    );
    assert_eq!(
        duckdb::evaluate_sql(
            &inputs,
            "SELECT DISTINCT c0 FROM \"mini-linq/raw/0\" ORDER BY c0 DESC"
        )
        .unwrap(),
        vec![vec![2], vec![-1]]
    );
    assert_eq!(duckdb::quote_identifier("a\"b"), "\"a\"\"b\"");
}

#[test]
fn raw_oracle_rejects_bad_width_null_outputs_and_out_of_range_integers() {
    let inputs = [RelationData {
        name: "R".into(),
        arity: 2,
        rows: vec![vec![1]],
    }];
    assert!(duckdb::evaluate_sql(&inputs, "SELECT 1").is_err());
    assert!(duckdb::evaluate_sql(&[], "SELECT NULL::INTEGER").is_err());
    assert!(duckdb::evaluate_sql(&[], "SELECT 2147483648::BIGINT").is_err());
}

#[test]
fn fuzz_sql_adapter_routes_opaque_rust_and_preserves_sorted_distinct_results() {
    for seed in [1, 3, 5, 7, 9, 11, 13, 15] {
        let rows = duckdb::evaluate_fuzz(&fuzz::generate(seed)).unwrap();
        assert!(rows.windows(2).all(|pair| pair[0] < pair[1]));
    }
    assert!(
        duckdb::evaluate_fuzz(&fuzz::generate(0))
            .unwrap_err()
            .to_string()
            .contains("opaque Rust operations")
    );
    let mut malformed = fuzz::generate(1);
    malformed.data.clear();
    assert!(duckdb::evaluate_fuzz(&malformed).is_err());
}

#[test]
fn cli_preserves_exact_csv_values_and_session_state() {
    let connection = duckdb::Connection::open_in_memory().unwrap();
    connection.execute_batch("CREATE TABLE t AS SELECT 42 AS n; SET threads=1").unwrap();
    assert_eq!(connection.scalar("SELECT n FROM t").unwrap(), "42");
    assert_eq!(
        connection.query("SELECT 'a,\"b' || chr(10) || 'c', ''::VARCHAR, 123456789012345678.90::DECIMAL(20,2), 18446744073709551615::UBIGINT").unwrap(),
        vec![vec!["a,\"b\nc".to_owned(), "".to_owned(), "123456789012345678.90".to_owned(), "18446744073709551615".to_owned()]]
    );
    assert!(connection.query("SELECT n FROM t WHERE false").unwrap().is_empty());
    assert_eq!(connection.scalar("SELECT NULL::INTEGER").unwrap(), "\\N");
}

#[test]
fn cli_reports_sql_errors_instead_of_accepting_partial_output() {
    let connection = duckdb::Connection::open_in_memory().unwrap();
    let error = connection.execute_batch("SELECT 1; SELECT * FROM missing_cq_table").unwrap_err();
    assert!(error.to_string().contains("missing_cq_table"), "{error}");
}
