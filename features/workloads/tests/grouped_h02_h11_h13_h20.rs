use cq_load::{Date, Decimal, Load};
use cq_testkit::conformance::assert_answers_equal;
use cq_testkit::duckdb::{RelationData, evaluate_sql};

#[allow(dead_code, unused_imports, unused_macros)]
#[path = "../src/tpch_runtime.rs"]
mod tpch_runtime;

#[allow(dead_code)]
#[path = "../src/queries/retail/tpch/retail_h02.rs"]
mod h02;
#[allow(dead_code)]
#[path = "../src/queries/retail/tpch/retail_h11.rs"]
mod h11;
#[allow(dead_code)]
#[path = "../src/queries/retail/tpch/retail_h13.rs"]
mod h13;
#[allow(dead_code)]
#[path = "../src/queries/retail/tpch/retail_h15.rs"]
mod h15;
#[allow(dead_code)]
#[path = "../src/queries/retail/tpch/retail_h20.rs"]
mod h20;

fn decimal(value: i32) -> Decimal {
    Decimal::from_scaled(i64::from(value)).unwrap()
}
fn integer(value: i64) -> i32 {
    i32::try_from(value).unwrap()
}
fn code(value: &str) -> i32 {
    value.as_bytes()[0] as i32
}
fn raw(rows: Vec<Vec<i32>>, arity: usize) -> RelationData {
    RelationData {
        name: "Raw".into(),
        arity,
        rows,
    }
}

#[test]
fn h02_minimum_preserves_ties_and_restricts_the_minimum_to_europe() {
    let rows = vec![
        vec![1, 1, 100],
        vec![1, 1, 100],
        vec![1, 2, 100],
        vec![1, 3, 1],
        vec![2, 1, 100],
        vec![3, 1, 100],
        vec![4, 3, 1],
    ];
    for rows in [rows.clone(), rows.into_iter().rev().collect(), vec![]] {
        let mut dictionaries = cq_load::Dictionaries::default();
        dictionaries.insert("p_mfgr", ["M"].into_iter());
        dictionaries.insert("p_type", ["BRASS", "BRASSX"].into_iter());
        dictionaries.insert("s_name", ["A", "B", "C"].into_iter());
        dictionaries.insert("s_address", ["X", "Y", "Z"].into_iter());
        dictionaries.insert("s_phone", ["P"].into_iter());
        dictionaries.insert("s_comment", ["C"].into_iter());
        dictionaries.insert("n_name", ["N", "O"].into_iter());
        dictionaries.insert("r_name", ["EUROPE", "ASIA"].into_iter());
        let data = h02::Data {
            part: vec![
                (
                    1,
                    dictionaries.literal("p_mfgr", "M"),
                    15,
                    dictionaries.literal("p_type", "BRASS"),
                ),
                (
                    2,
                    dictionaries.literal("p_mfgr", "M"),
                    14,
                    dictionaries.literal("p_type", "BRASS"),
                ),
                (
                    3,
                    dictionaries.literal("p_mfgr", "M"),
                    15,
                    dictionaries.literal("p_type", "BRASSX"),
                ),
                (
                    4,
                    dictionaries.literal("p_mfgr", "M"),
                    15,
                    dictionaries.literal("p_type", "BRASS"),
                ),
            ],
            partsupp: rows
                .iter()
                .map(|r| (i64::from(r[0]), i64::from(r[1]), decimal(r[2])))
                .collect(),
            supplier: vec![
                (
                    1,
                    1,
                    decimal(123),
                    dictionaries.literal("s_name", "A"),
                    dictionaries.literal("s_address", "X"),
                    dictionaries.literal("s_phone", "P"),
                    dictionaries.literal("s_comment", "C"),
                ),
                (
                    2,
                    1,
                    decimal(456),
                    dictionaries.literal("s_name", "B"),
                    dictionaries.literal("s_address", "Y"),
                    dictionaries.literal("s_phone", "P"),
                    dictionaries.literal("s_comment", "C"),
                ),
                (
                    3,
                    2,
                    decimal(789),
                    dictionaries.literal("s_name", "C"),
                    dictionaries.literal("s_address", "Z"),
                    dictionaries.literal("s_phone", "P"),
                    dictionaries.literal("s_comment", "C"),
                ),
            ],
            nation: vec![
                (1, 1, dictionaries.literal("n_name", "N")),
                (2, 2, dictionaries.literal("n_name", "O")),
            ],
            region: vec![
                (1, dictionaries.literal("r_name", "EUROPE")),
                (2, dictionaries.literal("r_name", "ASIA")),
            ],
            brass_types: cq_load::dictionary_where(&dictionaries, "p_type", |value| {
                value.ends_with("BRASS")
            }),
            dictionaries,
        };
        let sql = format!(
            r#"WITH
            part(p_partkey,p_mfgr,p_size,p_type) AS (VALUES
                (1,'M',15,'BRASS'),(2,'M',14,'BRASS'),(3,'M',15,'BRASSX'),(4,'M',15,'BRASS')),
            partsupp AS (SELECT c0 AS ps_partkey,c1 AS ps_suppkey,
                CAST(c2 * 0.01 AS DECIMAL(18,2)) AS ps_supplycost FROM "mini-linq/raw/0"),
            supplier(s_suppkey,s_nationkey,s_acctbal,s_name,s_address,s_phone,s_comment) AS (VALUES
                (1,1,1.23,'A','X','P','C'),(2,1,4.56,'B','Y','P','C'),(3,2,7.89,'C','Z','P','C')),
            nation(n_nationkey,n_regionkey,n_name) AS (VALUES (1,1,'N'),(2,2,'O')),
            region(r_regionkey,r_name) AS (VALUES (1,'EUROPE'),(2,'ASIA'))
            SELECT balance,unicode(supplier),unicode(nation),part,unicode(manufacturer),
                unicode(address),unicode(phone),unicode(comment)
            FROM ({}) AS result(balance,supplier,nation,part,manufacturer,address,phone,comment)"#,
            h02::SQL
        );
        let expected = evaluate_sql(&[raw(rows, 3)], &sql).unwrap();
        for config in ["default", "transfer"] {
            let actual = h02::run(&data, config)
                .into_iter()
                .map(
                    |(balance, supplier, nation, part, manufacturer, address, phone, comment)| {
                        vec![
                            integer(balance),
                            code(&supplier),
                            code(&nation),
                            integer(part),
                            code(&manufacturer),
                            code(&address),
                            code(&phone),
                            code(&comment),
                        ]
                    },
                )
                .collect::<Vec<_>>();
            assert_answers_equal("DuckDB H02", expected.clone(), config, actual, true);
        }
    }
}

#[test]
fn h11_threshold_is_strict_and_both_sums_preserve_supplier_identity() {
    // Total = 10000 units, so the threshold is exactly one unit. The two
    // different suppliers of part 2 each contribute one; a duplicate contributes once.
    let rows = vec![
        vec![1, 1, 1, 1],
        vec![2, 1, 1, 1],
        vec![2, 1, 1, 1],
        vec![2, 2, 1, 1],
        vec![3, 1, 1, 9997],
        vec![4, 3, 1, 999999],
    ];
    for rows in [rows.clone(), rows.into_iter().rev().collect(), vec![]] {
        let mut dictionaries = cq_load::Dictionaries::default();
        dictionaries.insert("n_name", ["GERMANY", "FRANCE"].into_iter());
        let data = h11::Data {
            partsupp: rows
                .iter()
                .map(|r| {
                    (
                        i64::from(r[0]),
                        i64::from(r[1]),
                        i64::from(r[2]),
                        decimal(r[3]),
                    )
                })
                .collect(),
            supplier: vec![(1, 1), (2, 1), (3, 2)],
            nation: vec![
                (1, dictionaries.literal("n_name", "GERMANY")),
                (2, dictionaries.literal("n_name", "FRANCE")),
            ],
            dictionaries,
        };
        let sql = format!(
            r#"WITH partsupp AS (SELECT c0 AS ps_partkey,c1 AS ps_suppkey,
            c2 AS ps_availqty,CAST(c3 * 0.01 AS DECIMAL(18,2)) AS ps_supplycost
            FROM "mini-linq/raw/0"), supplier(s_suppkey,s_nationkey) AS (VALUES (1,1),(2,1),(3,2)),
            nation(n_nationkey,n_name) AS (VALUES (1,'GERMANY'),(2,'FRANCE'))
            SELECT * FROM ({})"#,
            h11::SQL
        );
        let expected = evaluate_sql(&[raw(rows, 4)], &sql).unwrap();
        for config in ["default", "transfer"] {
            let actual = h11::run(&data, config)
                .into_iter()
                .map(|(part, value)| vec![integer(part), integer(value)])
                .collect::<Vec<_>>();
            assert_answers_equal("DuckDB H11", expected.clone(), config, actual, true);
        }
    }
}

#[test]
fn h13_counts_zero_and_obeys_ordered_comment_pattern() {
    let comments = [
        "ordinary",
        "special requests",
        "requests special",
        "specialspecialrequests",
    ];
    // Neither order/customer pairs nor comments are assumed unique.
    let rows = vec![
        vec![1, 1, 0],
        vec![2, 1, 0],
        vec![1, 1, 2],
        vec![1, 1, 1],
        vec![2, 1, 0],
        vec![3, 2, 1],
        vec![4, 3, 2],
        vec![5, 4, 3],
        vec![6, 99, 0],
    ];
    for rows in [rows.clone(), rows.into_iter().rev().collect(), vec![]] {
        let mut dictionaries = cq_load::Dictionaries::default();
        dictionaries.insert(
            "o_comment",
            [
                "ordinary",
                "special requests",
                "requests special",
                "specialspecialrequests",
            ]
            .into_iter(),
        );
        let data = h13::Data {
            customer: vec![(1,), (2,), (3,), (4,), (5,)],
            orders: rows
                .iter()
                .map(|r| {
                    (
                        i64::from(r[0]),
                        i64::from(r[1]),
                        dictionaries.literal("o_comment", comments[r[2] as usize]),
                    )
                })
                .collect(),
            qualified_comments: cq_storage::DenseMembership::from_dense_keys(
                cq_load::dictionary_where(&dictionaries, "o_comment", |value| {
                    !value
                        .split_once("special")
                        .is_some_and(|(_, rest)| rest.contains("requests"))
                })
                .into_iter()
                .map(|(code,)| code),
            ),
            dictionaries,
        };
        let sql = format!(
            r#"WITH customer(c_custkey) AS (VALUES (1),(2),(3),(4),(5)),
            orders AS (SELECT c0 AS o_orderkey,c1 AS o_custkey,
                CASE c2 WHEN 0 THEN 'ordinary' WHEN 1 THEN 'special requests'
                    WHEN 2 THEN 'requests special' ELSE 'specialspecialrequests' END AS o_comment
                FROM "mini-linq/raw/0") {}"#,
            h13::SQL
        );
        let expected = evaluate_sql(&[raw(rows.clone(), 3)], &sql).unwrap();
        for config in h13::CONFIGS {
            let actual = h13::run(&data, config)
                .into_iter()
                .map(|(customer, count)| vec![integer(customer), i32::try_from(count).unwrap()])
                .collect::<Vec<_>>();
            assert_answers_equal("DuckDB H13", &expected, config, &actual, true);
        }
    }
    let empty = h13::Data {
        customer: vec![],
        orders: vec![],
        qualified_comments: cq_storage::DenseMembership::from_dense_keys([]),
        dictionaries: cq_load::Dictionaries::default(),
    };
    for config in ["default", "transfer"] {
        assert!(h13::run(&empty, config).is_empty(), "{config}");
    }
}

#[test]
fn h20_preserves_line_identity_date_bounds_and_absent_group_null_semantics() {
    let rows = vec![
        vec![1, 1, 1, 1, 200, 0],
        vec![1, 1, 1, 1, 200, 0],
        vec![1, 2, 1, 1, 200, 0],
        vec![2, 1, 1, 2, 200, 0],
        vec![2, 2, 1, 2, 200, 364],
        vec![3, 1, 1, 3, 100, 0],
        vec![4, 1, 2, 1, 100, 0],
        vec![5, 1, 1, 2, 9999, -1],
        vec![6, 1, 1, 2, 9999, 365],
    ];
    for rows in [rows.clone(), rows.into_iter().rev().collect(), vec![]] {
        let mut dictionaries = cq_load::Dictionaries::default();
        dictionaries.insert("p_name", ["forest green", "xforest", "forest"].into_iter());
        dictionaries.insert("s_name", ["A", "B", "C"].into_iter());
        dictionaries.insert("s_address", ["X", "Y", "Z"].into_iter());
        dictionaries.insert("n_name", ["CANADA", "FRANCE"].into_iter());
        let data = h20::Data {
            part: vec![
                (1, dictionaries.literal("p_name", "forest green")),
                (2, dictionaries.literal("p_name", "xforest")),
                (3, dictionaries.literal("p_name", "forest")),
            ],
            partsupp: vec![(1, 1, 2), (1, 2, 3), (1, 3, 5), (2, 1, 100), (3, 1, 5)],
            supplier: vec![
                (
                    1,
                    1,
                    dictionaries.literal("s_name", "A"),
                    dictionaries.literal("s_address", "X"),
                ),
                (
                    2,
                    1,
                    dictionaries.literal("s_name", "B"),
                    dictionaries.literal("s_address", "Y"),
                ),
                (
                    3,
                    2,
                    dictionaries.literal("s_name", "C"),
                    dictionaries.literal("s_address", "Z"),
                ),
            ],
            nation: vec![
                (1, dictionaries.literal("n_name", "CANADA")),
                (2, dictionaries.literal("n_name", "FRANCE")),
            ],
            lineitem: rows
                .iter()
                .map(|r| {
                    (
                        i64::from(r[0]),
                        i64::from(r[1]),
                        i64::from(r[2]),
                        i64::from(r[3]),
                        decimal(r[4]),
                        Date::load(match r[5] {
                            -1 => "1993-12-31",
                            364 => "1994-12-31",
                            365 => "1995-01-01",
                            _ => "1994-01-01",
                        })
                        .unwrap(),
                    )
                })
                .collect(),
            forest_names: cq_load::dictionary_where(&dictionaries, "p_name", |value| {
                value.starts_with("forest")
            }),
            dictionaries,
        };
        let sql = format!(
            r#"WITH part(p_partkey,p_name) AS (VALUES
            (1,'forest green'),(2,'xforest'),(3,'forest')),
            partsupp(ps_partkey,ps_suppkey,ps_availqty) AS (VALUES
                (1,1,2),(1,2,3),(1,3,5),(2,1,100),(3,1,5)),
            supplier(s_suppkey,s_nationkey,s_name,s_address) AS (VALUES
                (1,1,'A','X'),(2,1,'B','Y'),(3,2,'C','Z')),
            nation(n_nationkey,n_name) AS (VALUES (1,'CANADA'),(2,'FRANCE')),
            lineitem AS (SELECT c0 AS l_orderkey,c1 AS l_linenumber,c2 AS l_partkey,c3 AS l_suppkey,
                CAST(c4 * 0.01 AS DECIMAL(18,2)) AS l_quantity,DATE '1994-01-01' + c5 AS l_shipdate
                FROM "mini-linq/raw/0")
            SELECT unicode(s_name),unicode(s_address) FROM ({})"#,
            h20::SQL
        );
        let expected = evaluate_sql(&[raw(rows, 6)], &sql).unwrap();
        for config in ["default", "transfer"] {
            let actual = h20::run(&data, config)
                .into_iter()
                .map(|(name, address)| vec![code(&name), code(&address)])
                .collect::<Vec<_>>();
            assert_answers_equal("DuckDB H20", expected.clone(), config, actual, true);
        }
    }
}

#[test]
fn h15_grouped_max_preserves_all_maximum_ties_and_quarter_boundaries() {
    // Supplier 1 has two distinct equal-valued lines; supplier 2 ties their
    // combined scale-four revenue exactly. Supplier 3 is strictly dominated.
    let rows = vec![
        vec![1, 1, 1, 101, 11, 0],
        vec![1, 1, 1, 101, 11, 0],
        vec![1, 2, 1, 101, 11, 90],
        vec![2, 1, 2, 202, 11, 59],
        vec![3, 1, 3, 200, 11, 0],
        vec![4, 1, 3, 9999, 0, -1],
        vec![5, 1, 3, 9999, 0, 91],
    ];
    let zeros = rows
        .iter()
        .cloned()
        .map(|mut row| {
            row[3] = 0;
            row
        })
        .collect();
    for rows in [
        rows.clone(),
        rows.into_iter().rev().collect(),
        zeros,
        vec![],
    ] {
        let mut dictionaries = cq_load::Dictionaries::default();
        dictionaries.insert("s_name", ["A", "B", "C", "D"].into_iter());
        dictionaries.insert("s_address", ["X", "Y", "Z", "W"].into_iter());
        dictionaries.insert("s_phone", ["P", "Q", "R", "S"].into_iter());
        let data = h15::Data {
            supplier: vec![
                (
                    1,
                    dictionaries.literal("s_name", "A"),
                    dictionaries.literal("s_address", "X"),
                    dictionaries.literal("s_phone", "P"),
                ),
                (
                    2,
                    dictionaries.literal("s_name", "B"),
                    dictionaries.literal("s_address", "Y"),
                    dictionaries.literal("s_phone", "Q"),
                ),
                (
                    3,
                    dictionaries.literal("s_name", "C"),
                    dictionaries.literal("s_address", "Z"),
                    dictionaries.literal("s_phone", "R"),
                ),
                (
                    4,
                    dictionaries.literal("s_name", "D"),
                    dictionaries.literal("s_address", "W"),
                    dictionaries.literal("s_phone", "S"),
                ),
            ],
            lineitem: rows
                .iter()
                .map(|r| {
                    (
                        i64::from(r[0]),
                        i64::from(r[1]),
                        i64::from(r[2]),
                        decimal(r[3]),
                        decimal(r[4]),
                        Date::load(match r[5] {
                            -1 => "1995-12-31",
                            59 => "1996-02-29",
                            90 => "1996-03-31",
                            91 => "1996-04-01",
                            _ => "1996-01-01",
                        })
                        .unwrap(),
                    )
                })
                .collect(),
            dictionaries,
        };
        let sql = format!(
            r#"WITH
            supplier(s_suppkey,s_name,s_address,s_phone) AS (VALUES
                (1,'A','X','P'),(2,'B','Y','Q'),(3,'C','Z','R'),(4,'D','W','S')),
            lineitem AS (SELECT c0 AS l_orderkey,c1 AS l_linenumber,c2 AS l_suppkey,
                CAST(c3 * 0.01 AS DECIMAL(18,2)) AS l_extendedprice,
                CAST(c4 * 0.01 AS DECIMAL(18,2)) AS l_discount,
                DATE '1996-01-01' + c5 AS l_shipdate FROM "mini-linq/raw/0")
            SELECT s_suppkey,unicode(s_name),unicode(s_address),unicode(s_phone),total_revenue
            FROM ({})"#,
            h15::SQL
        );
        let expected = evaluate_sql(&[raw(rows, 6)], &sql).unwrap();
        for config in ["default", "transfer"] {
            let actual = h15::run(&data, config)
                .into_iter()
                .map(|(supplier, name, address, phone, revenue)| {
                    vec![
                        integer(supplier),
                        code(&name),
                        code(&address),
                        code(&phone),
                        integer(revenue),
                    ]
                })
                .collect::<Vec<_>>();
            assert_answers_equal(
                "DuckDB H15 MAX oracle",
                expected.clone(),
                config,
                actual,
                true,
            );
        }
    }
}
