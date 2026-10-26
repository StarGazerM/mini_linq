use cq_load::{Date, Decimal, Load};
use cq_testkit::conformance::assert_answers_equal;
use cq_testkit::duckdb::{RelationData, evaluate_sql};

#[allow(unused_imports, unused_macros, dead_code)]
#[path = "../src/tpch_runtime.rs"]
mod tpch_runtime;

#[allow(dead_code)]
#[path = "../src/queries/retail/tpch/retail_h06.rs"]
mod h06;
#[allow(dead_code)]
#[path = "../src/queries/retail/tpch/retail_h10.rs"]
mod h10;
#[allow(dead_code)]
#[path = "../src/queries/retail/tpch/retail_h12.rs"]
mod h12;

fn decimal(value: i32) -> Decimal {
    Decimal::from_scaled(i64::from(value)).unwrap()
}
fn date(value: &str) -> Date {
    Date::load(value).unwrap()
}
fn raw(rows: Vec<Vec<i32>>, arity: usize) -> RelationData {
    RelationData { name: "Raw".into(), arity, rows }
}
fn integer(value: i64) -> i32 {
    i32::try_from(value).unwrap()
}

#[test]
fn h06_exact_revenue_preserves_lines_and_empty_identity() {
    let rows = vec![
        vec![1, 1, 123, 5, 2300, 0],
        vec![1, 1, 123, 5, 2300, 0],
        vec![1, 2, 123, 5, 2300, 0],
        vec![2, 1, 321, 7, 2300, 0],
        vec![3, 1, 100, 4, 2300, 0],
        vec![4, 1, 100, 8, 2300, 0],
        vec![5, 1, 100, 6, 2400, 0],
        vec![6, 1, 100, 6, 2300, 365],
        vec![7, 1, 100, 6, 2300, -1],
    ];
    for rows in [rows.clone(), rows.into_iter().rev().collect(), vec![]] {
        let data = h06::Data { lineitem: rows.iter().map(|r| (
            i64::from(r[0]), i64::from(r[1]), decimal(r[2]), decimal(r[3]), decimal(r[4]),
            date(match r[5] { -1 => "1993-12-31", 365 => "1995-01-01", _ => "1994-01-01" }),
        )).collect() };
        let sql = format!(r#"WITH lineitem AS (
            SELECT c0 AS l_orderkey, c1 AS l_linenumber,
                CAST(c2 * 0.01 AS DECIMAL(18,2)) AS l_extendedprice,
                CAST(c3 * 0.01 AS DECIMAL(18,2)) AS l_discount,
                CAST(c4 * 0.01 AS DECIMAL(18,2)) AS l_quantity,
                DATE '1994-01-01' + c5 AS l_shipdate FROM "mini-linq/raw/0") {}"#, h06::SQL);
        let expected = evaluate_sql(&[raw(rows, 6)], &sql).unwrap();
        for config in ["default", "transfer"] {
            let actual = h06::run(&data, config).into_iter().map(|(v,)| vec![integer(v)]).collect::<Vec<_>>();
            assert_answers_equal("DuckDB H06", expected.clone(), config, actual, true);
        }
    }
}

#[test]
fn h10_customer_revenue_matches_duckdb_without_limit() {
    // Dates and return flags include both exclusion boundaries; repeated lines
    // collapse, whereas different line identities with equal prices survive.
    let rows = vec![vec![1, 1, 123, 5, 0, 82], vec![1, 1, 123, 5, 0, 82],
        vec![1, 2, 123, 5, 0, 82], vec![2, 1, 321, 7, 91, 82],
        vec![3, 1, 100, 6, 92, 82], vec![4, 1, 100, 6, -1, 82],
        vec![5, 1, 100, 6, 0, 78]];
    for rows in [rows.clone(), rows.into_iter().rev().collect(), vec![]] {
        let data = h10::Data {
            customer: vec![(1, "C".into(), decimal(12345), 1, "A".into(), "P".into(), "X".into())],
            nation: vec![(1, "N".into())],
            orders: rows.iter().map(|r| (i64::from(r[0]), 1, date(match r[4] {
                -1 => "1993-09-30", 91 => "1993-12-31", 92 => "1994-01-01", _ => "1993-10-01",
            }))).collect(),
            lineitem: rows.iter().map(|r| (i64::from(r[0]), i64::from(r[1]), decimal(r[2]), decimal(r[3]),
                char::from_u32(r[5] as u32).unwrap().to_string())).collect(),
        };
        let sql = format!(r#"WITH customer AS (SELECT 1 AS c_custkey, 'C' AS c_name,
            123.45::DECIMAL(18,2) AS c_acctbal, 1 AS c_nationkey, 'A' AS c_address,
            'P' AS c_phone, 'X' AS c_comment), nation AS (SELECT 1 AS n_nationkey, 'N' AS n_name),
            orders AS (SELECT DISTINCT c0 AS o_orderkey, 1 AS o_custkey,
                DATE '1993-10-01' + c4 AS o_orderdate FROM "mini-linq/raw/0"),
            lineitem AS (SELECT c0 AS l_orderkey, c1 AS l_linenumber,
                CAST(c2 * 0.01 AS DECIMAL(18,2)) AS l_extendedprice,
                CAST(c3 * 0.01 AS DECIMAL(18,2)) AS l_discount, chr(c5) AS l_returnflag
                FROM "mini-linq/raw/0")
            SELECT c_custkey, unicode(c_name), revenue, account_balance, unicode(n_name),
                unicode(c_address), unicode(c_phone), unicode(c_comment) FROM ({})"#, h10::SQL);
        let expected = evaluate_sql(&[raw(rows, 6)], &sql).unwrap();
        for config in ["default", "transfer"] {
            let actual = h10::run(&data, config).into_iter().map(|(key, name, revenue, balance, nation, address, phone, comment)| {
                vec![integer(key), name.as_bytes()[0] as i32, integer(revenue), integer(balance),
                    nation.as_bytes()[0] as i32, address.as_bytes()[0] as i32,
                    phone.as_bytes()[0] as i32, comment.as_bytes()[0] as i32]
            }).collect::<Vec<_>>();
            assert_answers_equal("DuckDB H10", expected.clone(), config, actual, true);
        }
    }
}

#[test]
fn h12_priority_counts_match_duckdb_and_date_guards() {
    let rows = vec![vec![1, 1, 0, 0, 1, -1, 0], vec![1, 1, 0, 0, 1, -1, 0],
        vec![1, 2, 0, 0, 1, -1, 0], vec![2, 1, 0, 0, 1, -1, 1],
        vec![3, 1, 1, 0, 1, -1, 2], vec![4, 1, 2, 0, 1, -1, 0],
        vec![5, 1, 0, 0, 0, -1, 0], vec![6, 1, 0, 0, 1, 0, 0],
        vec![7, 1, 0, 1, 365, 0, 0], vec![8, 1, 0, -1, 0, -2, 0]];
    let modes = ["MAIL", "SHIP", "AIR"];
    let priorities = ["1-URGENT", "2-HIGH", "3-MEDIUM"];
    let day = |offset| date(match offset {
        -2 => "1993-12-30", -1 => "1993-12-31", 0 => "1994-01-01",
        1 => "1994-01-02", 365 => "1995-01-01", _ => unreachable!(),
    });
    for rows in [rows.clone(), rows.into_iter().rev().collect(), vec![]] {
        let data = h12::Data {
            orders: rows.iter().map(|r| (i64::from(r[0]), priorities[r[6] as usize].into())).collect(),
            lineitem: rows.iter().map(|r| (i64::from(r[0]), i64::from(r[1]), modes[r[2] as usize].into(),
                day(r[3]), day(r[4]), day(r[5]))).collect(),
        };
        let sql = format!(r#"WITH orders AS (SELECT DISTINCT c0 AS o_orderkey,
            CASE c6 WHEN 0 THEN '1-URGENT' WHEN 1 THEN '2-HIGH' ELSE '3-MEDIUM' END AS o_orderpriority
            FROM "mini-linq/raw/0"), lineitem AS (SELECT c0 AS l_orderkey, c1 AS l_linenumber,
            CASE c2 WHEN 0 THEN 'MAIL' WHEN 1 THEN 'SHIP' ELSE 'AIR' END AS l_shipmode,
            DATE '1994-01-01' + c3 AS l_commitdate, DATE '1994-01-01' + c4 AS l_receiptdate,
            DATE '1994-01-01' + c5 AS l_shipdate FROM "mini-linq/raw/0")
            SELECT unicode(l_shipmode), high_line_count, low_line_count FROM ({})"#, h12::SQL);
        let expected = evaluate_sql(&[raw(rows, 7)], &sql).unwrap();
        for config in ["default", "transfer"] {
            let actual = h12::run(&data, config).into_iter().map(|(mode, high, low)| {
                vec![mode.as_bytes()[0] as i32, integer(high), integer(low)]
            }).collect::<Vec<_>>();
            assert_answers_equal("DuckDB H12", expected.clone(), config, actual, true);
        }
    }
}

#[test]
#[allow(non_snake_case)]
fn integer_arithmetic_wraps_in_debug_queries_and_range_guards() {
    let Input = &[(i64::MAX, 2_i64), (i64::MIN, -1), (10, 3)];
    let actual: Vec<(i64, i64, i64, i64)> = mini_linq::compile! {
        pipeline {}
        struct Query;
        relation Input(x: i64, y: i64);
        relation answer(a: i64, b: i64, c: i64, d: i64);
        answer(a, b, c, d) :- Input(x, y),
            let a: i64 = *x + *y,
            let b: i64 = *x - *y,
            let c: i64 = *x * *y,
            let d: i64 = *x / -1,
            if *x + *y == *a;
    }.collect();
    let expected = Input.iter().map(|&(x, y)| (
        x.wrapping_add(y), x.wrapping_sub(y), x.wrapping_mul(y), x.wrapping_div(-1),
    )).collect::<std::collections::BTreeSet<_>>();
    assert_eq!(actual.into_iter().collect::<std::collections::BTreeSet<_>>(), expected);
}

#[test]
#[allow(non_snake_case)]
fn h06_integer_coefficient_query_is_exact() {
    let Lineitem = &[(1_i64, 123_i64, 5_i64, 23_i64, 365_i32),
        (2, 123, 5, 23, 365), (3, 321, 7, 23, 400), (4, 100, 6, 24, 400),
        (5, 100, 6, 23, 730), (1, 123, 5, 23, 365)];
    let actual: Vec<(i64,)> = mini_linq::compile! {
        pipeline {}
        struct Query;
        relation Lineitem(id: i64, price: i64, discount: i64, quantity: i64, date: i32);
        relation answer(revenue: i64);
        answer(sum(*price * *discount)) :- Lineitem(id, price, discount, quantity, date),
            if *date >= 365 && *date < 730,
            if *discount >= 5 && *discount <= 7,
            if *quantity < 24;
    }.collect();
    assert_eq!(actual, vec![(3477,)]);
}
