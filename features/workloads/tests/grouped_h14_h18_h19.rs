use cq_load::{Date, Decimal, Load};
use cq_testkit::conformance::assert_answers_equal;
use cq_testkit::duckdb::{RelationData, evaluate_sql};

#[allow(dead_code, unused_imports, unused_macros)]
#[path = "../src/tpch_runtime.rs"]
mod tpch_runtime;

#[allow(dead_code)]
#[path = "../src/queries/retail/tpch/retail_h14.rs"]
mod h14;
#[allow(dead_code)]
#[path = "../src/queries/retail/tpch/retail_h18.rs"]
mod h18;
#[allow(dead_code)]
#[path = "../src/queries/retail/tpch/retail_h19.rs"]
mod h19;

fn decimal(value: i32) -> Decimal {
    Decimal::from_scaled(i64::from(value)).unwrap()
}

fn raw(name: &str, arity: usize, rows: Vec<Vec<i32>>) -> RelationData {
    RelationData { name: name.into(), arity, rows }
}

fn integer(value: i64) -> i32 {
    i32::try_from(value).unwrap()
}

#[test]
fn h14_components_preserve_identity_dates_and_empty_global_group() {
    let rows = vec![
        vec![1, 1, 1, 1234, 7, 0],
        vec![1, 1, 1, 1234, 7, 0], // Physical duplicate counts once.
        vec![1, 2, 1, 1234, 7, 0], // Equal revenue, distinct line.
        vec![2, 1, 2, 2001, 3, 29],
        vec![3, 1, 1, 9000, 0, -1],
        vec![4, 1, 1, 9000, 0, 30],
        vec![5, 1, 3, 1001, 1, 0], // Lowercase prefix is not PROMO.
        vec![6, 1, 99, 9000, 0, 0], // No matching part.
    ];
    let parts = vec![vec![1, 1], vec![1, 1], vec![2, 2], vec![3, 3]];
    for rows in [rows.clone(), rows.into_iter().rev().collect(), Vec::new()] {
        let data = h14::Data {
            lineitem: rows.iter().map(|r| {
                let date = match r[5] {
                    -1 => "1995-08-31", 0 => "1995-09-01",
                    29 => "1995-09-30", 30 => "1995-10-01", _ => unreachable!(),
                };
                (r[0].into(), r[1].into(), r[2].into(), decimal(r[3]),
                    decimal(r[4]), Date::load(date).unwrap())
            }).collect(),
            part: parts.iter().map(|r| {
                (r[0].into(), match r[1] {
                    1 => "PROMO BRASS", 2 => "STANDARD BRASS", 3 => "promo BRASS", _ => unreachable!(),
                }.into())
            }).collect(),
        };

        let sql = format!(r#"WITH lineitem AS (
            SELECT c0 AS l_orderkey, c1 AS l_linenumber, c2 AS l_partkey,
                CAST(c3 / 100.0 AS DECIMAL(18,2)) AS l_extendedprice,
                CAST(c4 / 100.0 AS DECIMAL(18,2)) AS l_discount,
                DATE '1995-09-01' + c5 AS l_shipdate FROM "mini-linq/raw/0"),
            part AS (SELECT c0 AS p_partkey, CASE c1 WHEN 1 THEN 'PROMO BRASS'
                WHEN 2 THEN 'STANDARD BRASS' ELSE 'promo BRASS' END AS p_type
                FROM "mini-linq/raw/1") {}"#, h14::SQL);
        let expected = evaluate_sql(&[raw("Lineitem", 6, rows), raw("Part", 2, parts.clone())], &sql)
            .expect("DuckDB H14 oracle");
        for &config in h14::CONFIGS {
            let actual = h14::run(&data, config).into_iter()
                .map(|(promo, total)| vec![integer(promo), integer(total)]).collect::<Vec<_>>();
            assert_answers_equal("DuckDB H14", expected.clone(), config, actual, true);
        }
    }
}

#[test]
fn h18_threshold_is_strict_and_joins_follow_identity_preserving_sum() {
    let rows = vec![
        vec![1, 1, 15000], vec![1, 2, 15000], // Exactly 300: excluded.
        vec![2, 1, 15000], vec![2, 1, 15000], vec![2, 2, 15001],
        vec![3, 1, 15100], vec![3, 2, 15100], // Equal values add twice.
        vec![4, 1, 40000], // Missing order.
        vec![5, 1, 40000], // Missing customer.
    ];
    let orders = vec![
        vec![1, 10, 0, 10001], vec![2, 10, 0, 12345],
        vec![2, 10, 0, 12345], vec![3, 20, 1, 12345], vec![5, 99, 0, 10001],
    ];
    let customers = vec![vec![10, 65], vec![10, 65], vec![20, 66]];
    for rows in [rows.clone(), rows.into_iter().rev().collect(), Vec::new()] {
        let data = h18::Data {
            lineitem: rows.iter().map(|r| (r[0].into(), r[1].into(), decimal(r[2]))).collect(),
            orders: orders.iter().map(|r| (r[0].into(), r[1].into(),
                Date::load(if r[2] == 0 { "1995-01-01" } else { "1995-01-02" }).unwrap(),
                decimal(r[3]))).collect(),
            customer: customers.iter().map(|r| (r[0].into(),
                char::from_u32(r[1] as u32).unwrap().to_string())).collect(),
        };

        let sql = format!(r#"WITH lineitem AS (
            SELECT c0 AS l_orderkey, c1 AS l_linenumber,
                CAST(c2 / 100.0 AS DECIMAL(18,2)) AS l_quantity FROM "mini-linq/raw/0"),
            orders AS (SELECT c0 AS o_orderkey, c1 AS o_custkey,
                DATE '1995-01-01' + c2 AS o_orderdate,
                CAST(c3 / 100.0 AS DECIMAL(18,2)) AS o_totalprice FROM "mini-linq/raw/1"),
            customer AS (SELECT c0 AS c_custkey, chr(c1) AS c_name FROM "mini-linq/raw/2")
            SELECT unicode(c_name), c_custkey, o_orderkey, o_orderdate,
                total_price, sum_quantity FROM ({})"#, h18::SQL);
        let expected = evaluate_sql(&[
            raw("Lineitem", 3, rows), raw("Orders", 4, orders.clone()),
            raw("Customer", 2, customers.clone()),
        ], &sql).expect("DuckDB H18 oracle");
        for &config in h18::CONFIGS {
            let actual = h18::run(&data, config).into_iter()
                .map(|(name, customer, order, date, price, quantity)| vec![
                    i32::from(name.as_bytes()[0]), integer(customer), integer(order),
                    date.days_since_epoch(),
                    integer(price), integer(quantity),
                ]).collect::<Vec<_>>();
            assert_answers_equal("DuckDB H18", expected.clone(), config, actual, true);
        }
    }
}

#[test]
fn h19_three_branches_boundaries_shipping_and_duplicate_identity() {
    let containers = ["SM CASE", "SM BOX", "SM PACK", "SM PKG",
        "MED BAG", "MED BOX", "MED PKG", "MED PACK",
        "LG CASE", "LG BOX", "LG PACK", "LG PKG"];
    let mut parts = Vec::new();
    let mut rows = Vec::new();
    for branch in 0..3_i32 {
        for container in 0..4_i32 {
            let part = branch * 4 + container + 1;
            parts.push(vec![part, branch, branch * 4 + container, (branch + 1) * 5]);
            let (low, high) = [(100, 1100), (1000, 2000), (2000, 3000)][branch as usize];
            for (line, quantity) in [(1, low), (2, high), (3, low - 1), (4, high + 1)] {
                rows.push(vec![part, line, part, quantity, 1234, 7, line % 2, 0]);
            }
        }
    }
    rows.push(rows[0].clone());
    rows.push(vec![90, 1, 1, 100, 1234, 7, 2, 0]); // Wrong ship mode.
    rows.push(vec![91, 1, 1, 100, 1234, 7, 0, 1]); // Wrong instruction.
    parts.extend([vec![13, 0, 0, 6], vec![14, 1, 4, 11], vec![15, 2, 8, 16],
        vec![16, 0, 4, 1], vec![17, 3, 0, 1], vec![18, 0, 0, 0]]);
    for part in 13..=18 {
        let quantity = if part == 15 { 2000 } else { 1000 };
        rows.push(vec![part, 1, part, quantity, 1234, 7, 0, 0]);
    }
    rows.push(vec![99, 1, 99, 1000, 1234, 7, 0, 0]); // Missing part.
    parts.push(parts[0].clone());
    for rows in [rows.clone(), rows.into_iter().rev().collect(), Vec::new()] {
        let data = h19::Data::from_strings(
            rows.iter().map(|r| (r[0].into(), r[1].into(), r[2].into(),
                decimal(r[3]), decimal(r[4]), decimal(r[5]),
                ["AIR", "AIR REG", "RAIL"][r[6] as usize].into(),
                ["DELIVER IN PERSON", "TAKE BACK RETURN"][r[7] as usize].into())).collect(),
            parts.iter().map(|r| (r[0].into(),
                ["Brand#12", "Brand#23", "Brand#34", "Brand#99"][r[1] as usize].into(),
                containers[r[2] as usize].into(), r[3])).collect(),
        );

        let sql = format!(r#"WITH lineitem AS (
            SELECT c0 AS l_orderkey, c1 AS l_linenumber, c2 AS l_partkey,
                CAST(c3 / 100.0 AS DECIMAL(18,2)) AS l_quantity,
                CAST(c4 / 100.0 AS DECIMAL(18,2)) AS l_extendedprice,
                CAST(c5 / 100.0 AS DECIMAL(18,2)) AS l_discount,
                CASE c6 WHEN 0 THEN 'AIR' WHEN 1 THEN 'AIR REG' ELSE 'RAIL' END AS l_shipmode,
                CASE c7 WHEN 0 THEN 'DELIVER IN PERSON' ELSE 'TAKE BACK RETURN' END AS l_shipinstruct
                FROM "mini-linq/raw/0"),
            part AS (SELECT c0 AS p_partkey,
                CASE c1 WHEN 0 THEN 'Brand#12' WHEN 1 THEN 'Brand#23'
                    WHEN 2 THEN 'Brand#34' ELSE 'Brand#99' END AS p_brand,
                CASE c2 WHEN 0 THEN 'SM CASE' WHEN 1 THEN 'SM BOX' WHEN 2 THEN 'SM PACK'
                    WHEN 3 THEN 'SM PKG' WHEN 4 THEN 'MED BAG' WHEN 5 THEN 'MED BOX'
                    WHEN 6 THEN 'MED PKG' WHEN 7 THEN 'MED PACK' WHEN 8 THEN 'LG CASE'
                    WHEN 9 THEN 'LG BOX' WHEN 10 THEN 'LG PACK' ELSE 'LG PKG' END AS p_container,
                c3 AS p_size FROM "mini-linq/raw/1") {}"#, h19::SQL);
        let expected = evaluate_sql(&[raw("Lineitem", 8, rows), raw("Part", 4, parts.clone())], &sql)
            .expect("DuckDB H19 oracle");
        for &config in h19::CONFIGS {
            let actual = h19::run(&data, config).into_iter()
                .map(|(revenue,)| vec![integer(revenue)]).collect::<Vec<_>>();
            assert_answers_equal("DuckDB H19", expected.clone(), config, actual, true);
        }
    }
}
