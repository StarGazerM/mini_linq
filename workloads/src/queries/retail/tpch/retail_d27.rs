// TPC-H-derived re-authoring of the TPC-DS-inspired D27 fact/dimension join.
// Roles: store_sales -> lineitem; date_dim -> orders and its date;
// item -> part; store -> supplier; customer_demographics -> supplier's nation.
// Nation is an actual dbgen relation, not simulated demographic data.
use crate::tpch_runtime::date;
use cq_load::Date;

crate::tpch_runtime::retail_kernel! { $;
    name "retail_d27";
    sql r#"SELECT DISTINCT o.o_orderkey, p.p_partkey, s.s_suppkey
FROM lineitem l JOIN orders o ON l.l_orderkey = o.o_orderkey
JOIN part p ON l.l_partkey = p.p_partkey
JOIN supplier s ON l.l_suppkey = s.s_suppkey
JOIN nation n ON s.s_nationkey = n.n_nationkey
WHERE o.o_orderdate >= DATE '1995-01-01' AND o.o_orderdate < DATE '1995-04-01'
AND p.p_brand = 'Brand#12' AND n.n_name = 'UNITED STATES'"#;
    candidate hypergraph;
    setup { let start = date("1995-01-01"); let end = date("1995-04-01"); }
    relations {
        relation Lineitem => lineitem(l_orderkey: i64, l_partkey: i64, l_suppkey: i64);
        relation Orders => orders(o_orderkey: i64, o_orderdate: Date);
        relation Part => part(p_partkey: i64, p_brand: String);
        relation Supplier => supplier(s_suppkey: i64, s_nationkey: i32);
        relation Nation => nation(n_nationkey: i32, n_name: String);
    }
    answer(order_key: i64, part_key: i64, supplier_key: i64) :-
        Lineitem(order_key, part_key, supplier_key),
        Orders(order_key, order_date), if *order_date >= start && *order_date < end,
        Part(part_key, brand), if brand == "Brand#12",
        Supplier(supplier_key, nation_key), Nation(nation_key, nation_name),
        if nation_name == "UNITED STATES";
}
