//! TPC-H Q22 customer-without-orders core with the seven phone country codes.
//! Omits the scalar average-balance subquery, grouping, sums, and ordering.
//! SQL below is the independent DuckDB oracle for exactly this CQ core.
crate::tpch_runtime::retail_kernel! { $;
    name "retail_h22";
    sql r#"SELECT DISTINCT c.c_custkey FROM customer c
WHERE substring(c.c_phone FROM 1 FOR 2) IN ('13', '31', '23', '29', '30', '18', '17')
AND NOT EXISTS (SELECT 1 FROM orders o WHERE o.o_custkey = c.c_custkey)"#;
    candidate hypergraph;
    setup {}
    relations {
        relation Customer => customer(c_custkey: i64, c_phone: String);
        relation Orders => orders(o_orderkey: i64, o_custkey: i64);
    }
    answer(custkey: i64) :-
        Customer(custkey, phone),
        if ["13", "31", "23", "29", "30", "18", "17"].iter().any(|code| phone.starts_with(*code)),
        !Orders(_, custkey);
}
