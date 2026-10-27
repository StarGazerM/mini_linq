use cq_load::{Date, Decimal, Load};
use cq_testkit::conformance::assert_answers_equal;

#[allow(dead_code, unused_imports, unused_macros)]
#[path = "../src/tpch_runtime.rs"]
mod tpch_runtime;

#[allow(dead_code)]
#[path = "../src/queries/retail/tpch/retail_h01.rs"]
mod h01;

#[test]
fn h01_grouped_head_matches_duckdb_distinct_join() {
    // Two different line identities have equal aggregate arguments; duplicate
    // physical rows contribute once, distinct identities contribute twice.
    let raw = vec![
        vec![1, 1, 65, 70, 200, 1250, 0],
        vec![1, 1, 65, 70, 200, 1250, 0],
        vec![1, 2, 65, 70, 200, 1250, 0],
        vec![2, 1, 78, 79, 300, 700, 0],
        vec![3, 1, 65, 70, 900, 9900, 1],
    ];
    for rows in [raw.clone(), raw.into_iter().rev().collect(), Vec::new()] {
        let data = h01::Data {
            lineitem: rows
                .iter()
                .map(|r| {
                    (
                        i64::from(r[0]),
                        i64::from(r[1]),
                        char::from_u32(r[2] as u32).unwrap().to_string(),
                        char::from_u32(r[3] as u32).unwrap().to_string(),
                        Decimal::from_scaled(i64::from(r[4])).unwrap(),
                        Decimal::from_scaled(i64::from(r[5])).unwrap(),
                        Date::load(if r[6] == 0 {
                            "1998-09-02"
                        } else {
                            "1998-09-03"
                        })
                        .unwrap(),
                    )
                })
                .collect(),
        };
        let actual = tpch_runtime::same_plan_answers(|config| h01::run(&data, config), h01::canonical)
            .into_iter()
            .map(|(flag, status, quantity, price, count)| {
                vec![
                    flag.as_bytes()[0] as i32,
                    status.as_bytes()[0] as i32,
                    i32::try_from(quantity).unwrap(),
                    i32::try_from(price).unwrap(),
                    i32::try_from(count).unwrap(),
                ]
            })
            .collect::<Vec<_>>();
        let sql = format!(
            r#"WITH lineitem AS (
            SELECT c0 AS l_orderkey, c1 AS l_linenumber,
                chr(c2) AS l_returnflag, chr(c3) AS l_linestatus,
                CAST(c4 AS DECIMAL(18,2)) / 100 AS l_quantity,
                CAST(c5 AS DECIMAL(18,2)) / 100 AS l_extendedprice,
                DATE '1998-09-02' + c6 AS l_shipdate
            FROM "mini-linq/raw/0")
            SELECT unicode(l_returnflag), unicode(l_linestatus),
                sum_quantity, sum_price, count_order FROM ({})"#,
            h01::SQL
        );
        let expected = cq_testkit::duckdb::evaluate_sql(
            &[cq_testkit::duckdb::RelationData {
                name: "Lineitem".into(),
                arity: 7,
                rows,
            }],
            &sql,
        )
        .expect("DuckDB H01 oracle");
        assert_answers_equal("DuckDB H01", expected, "grouped CQ H01", actual, true);
    }
}
