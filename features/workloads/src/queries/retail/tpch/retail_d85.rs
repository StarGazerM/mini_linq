// TPC-H-derived re-authoring of the TPC-DS-inspired D85 composite-key core.
// Roles: web_sales -> lineitem/orders; web_returns -> partsupp on (part,supplier),
// web_page -> part; the two demographic aliases -> customer and supplier;
// customer_address/reason -> their two nation aliases; date_dim -> orders' date.
// dbgen has no return/reason relation: those roles are explicitly repurposed as
// supplier availability and supplier geography, not fabricated return records.
use crate::tpch_runtime::date;
use cq_load::Date;

crate::tpch_runtime::retail_kernel! { $;
    name "retail_d85";
    sql r#"SELECT DISTINCT o.o_orderkey, p.p_partkey, ns.n_nationkey
FROM lineitem l JOIN partsupp ps ON l.l_partkey = ps.ps_partkey AND l.l_suppkey = ps.ps_suppkey
JOIN part p ON p.p_partkey = l.l_partkey
JOIN orders o ON o.o_orderkey = l.l_orderkey
JOIN customer c ON c.c_custkey = o.o_custkey
JOIN supplier s ON s.s_suppkey = ps.ps_suppkey
JOIN nation nc ON nc.n_nationkey = c.c_nationkey
JOIN nation ns ON ns.n_nationkey = s.s_nationkey
WHERE o.o_orderdate >= DATE '1995-01-01' AND o.o_orderdate < DATE '1995-04-01'
AND nc.n_name = 'UNITED STATES' AND ps.ps_availqty > 0"#;
    candidate generic_smallest;
    extra hypergraph;
    setup { let start = date("1995-01-01"); let end = date("1995-04-01"); }
    relations {
        relation Lineitem => lineitem(l_orderkey: i64, l_partkey: i64, l_suppkey: i64);
        relation PartSupp => partsupp(ps_partkey: i64, ps_suppkey: i64, ps_availqty: i64);
        relation Part => part(p_partkey: i64);
        relation Orders => orders(o_orderkey: i64, o_custkey: i64, o_orderdate: Date);
        relation Customer => customer(c_custkey: i64, c_nationkey: i32);
        relation Supplier => supplier(s_suppkey: i64, s_nationkey: i32);
        relation Nation => nation(n_nationkey: i32, n_name: String);
    }
    answer(order_key: i64, part_key: i64, supplier_nation: i32) :-
        Lineitem(order_key, part_key, supplier_key), PartSupp(part_key, supplier_key, available), if *available > 0,
        Part(part_key), Orders(order_key, customer_key, order_date), if *order_date >= start && *order_date < end,
        Customer(customer_key, customer_nation), Supplier(supplier_key, supplier_nation),
        Nation(customer_nation, customer_name), if customer_name == "UNITED STATES",
        Nation(supplier_nation, _supplier_name);
}
