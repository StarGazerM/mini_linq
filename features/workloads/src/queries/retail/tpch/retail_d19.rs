// TPC-H-derived re-authoring of the TPC-DS-inspired D19 dimensional join shape.
// Roles: store_sales -> lineitem plus orders (customer/orderdate live on orders),
// date_dim -> orders' real orderdate filter; item -> part; customer -> customer;
// customer_address -> nation; store -> supplier. There are no fabricated TPC-DS
// tables, surrogate dates, or claims that dbgen can supply TPC-DS semantics.
use crate::tpch_runtime::date;
use cq_load::Date;

crate::tpch_runtime::retail_kernel! { $;
    name "retail_d19";
    sql r#"SELECT DISTINCT o.o_orderkey, p.p_partkey, c.c_custkey, s.s_suppkey
FROM lineitem l JOIN orders o ON l.l_orderkey = o.o_orderkey
JOIN part p ON l.l_partkey = p.p_partkey
JOIN customer c ON o.o_custkey = c.c_custkey
JOIN nation n ON c.c_nationkey = n.n_nationkey
JOIN supplier s ON l.l_suppkey = s.s_suppkey
WHERE o.o_orderdate >= DATE '1995-01-01' AND o.o_orderdate < DATE '1995-04-01'
AND n.n_name = 'UNITED STATES'"#;
    candidate hypergraph;
    setup { let start = date("1995-01-01"); let end = date("1995-04-01"); }
    relations {
        relation Lineitem => lineitem(l_orderkey: i64, l_partkey: i64, l_suppkey: i64);
        relation Orders => orders(o_orderkey: i64, o_custkey: i64, o_orderdate: Date);
        relation Part => part(p_partkey: i64);
        relation Customer => customer(c_custkey: i64, c_nationkey: i32);
        relation Nation => nation(n_nationkey: i32, n_name: String);
        relation Supplier => supplier(s_suppkey: i64);
    }
    answer(order_key: i64, part_key: i64, customer_key: i64, supplier_key: i64) :-
        Lineitem(order_key, part_key, supplier_key),
        Orders(order_key, customer_key, order_date), if *order_date >= start && *order_date < end,
        Part(part_key), Customer(customer_key, nation_key),
        Nation(nation_key, nation_name), if nation_name == "UNITED STATES",
        Supplier(supplier_key);
}
