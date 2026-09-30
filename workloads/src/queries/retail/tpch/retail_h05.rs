// TPC-H-derived positive Q5 join core, including the customer/supplier nation cycle.
// Revenue and grouping are deliberately outside this inspired kernel.
use crate::tpch_runtime::date;
use cq_load::Date;

crate::tpch_runtime::retail_kernel! { $;
    name "retail_h05";
    sql r#"SELECT DISTINCT o.o_orderkey, s.s_suppkey, n.n_nationkey
FROM customer c JOIN orders o ON c.c_custkey = o.o_custkey
JOIN lineitem l ON l.l_orderkey = o.o_orderkey
JOIN supplier s ON s.s_suppkey = l.l_suppkey AND s.s_nationkey = c.c_nationkey
JOIN nation n ON n.n_nationkey = s.s_nationkey
JOIN region r ON r.r_regionkey = n.n_regionkey
WHERE r.r_name = 'ASIA' AND o.o_orderdate >= DATE '1994-01-01'
AND o.o_orderdate < DATE '1995-01-01'"#;
    candidate generic_smallest;
    setup { let start = date("1994-01-01"); let end = date("1995-01-01"); }
    relations {
        relation Customer => customer(c_custkey: i64, c_nationkey: i32);
        relation Orders => orders(o_orderkey: i64, o_custkey: i64, o_orderdate: Date);
        relation Lineitem => lineitem(l_orderkey: i64, l_suppkey: i64);
        relation Supplier => supplier(s_suppkey: i64, s_nationkey: i32);
        relation Nation => nation(n_nationkey: i32, n_regionkey: i32);
        relation Region => region(r_regionkey: i32, r_name: String);
    }
    answer(order_key: i64, supplier_key: i64, nation_key: i32) :-
        Customer(customer_key, nation_key),
        Orders(order_key, customer_key, order_date), if *order_date >= start && *order_date < end,
        Lineitem(order_key, supplier_key), Supplier(supplier_key, nation_key),
        Nation(nation_key, region_key), Region(region_key, region_name), if region_name == "ASIA";
}
