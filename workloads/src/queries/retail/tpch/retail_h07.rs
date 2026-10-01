// TPC-H-derived positive Q7 join core with two aliases of the real nation table.
// Dates and the France/Germany predicate are retained; revenue/grouping are absent.
use crate::tpch_runtime::date;
use cq_load::Date;

crate::tpch_runtime::retail_kernel! { $;
    name "retail_h07";
    sql r#"SELECT DISTINCT o.o_orderkey, ns.n_nationkey, nc.n_nationkey
FROM supplier s JOIN lineitem l ON s.s_suppkey = l.l_suppkey
JOIN orders o ON l.l_orderkey = o.o_orderkey
JOIN customer c ON c.c_custkey = o.o_custkey
JOIN nation ns ON s.s_nationkey = ns.n_nationkey
JOIN nation nc ON c.c_nationkey = nc.n_nationkey
WHERE ((ns.n_name = 'FRANCE' AND nc.n_name = 'GERMANY')
OR (ns.n_name = 'GERMANY' AND nc.n_name = 'FRANCE'))
AND l.l_shipdate >= DATE '1995-01-01' AND l.l_shipdate <= DATE '1996-12-31'"#;
    candidate hypergraph;
    setup { let start = date("1995-01-01"); let end = date("1996-12-31"); }
    relations {
        relation Supplier => supplier(s_suppkey: i64, s_nationkey: i32);
        relation Lineitem => lineitem(l_orderkey: i64, l_suppkey: i64, l_shipdate: Date);
        relation Orders => orders(o_orderkey: i64, o_custkey: i64);
        relation Customer => customer(c_custkey: i64, c_nationkey: i32);
        relation Nation => nation(n_nationkey: i32, n_name: String);
    }
    answer(order_key: i64, supplier_nation: i32, customer_nation: i32) :-
        Supplier(supplier_key, supplier_nation),
        Lineitem(order_key, supplier_key, ship_date), ship_date between start and end,
        Orders(order_key, customer_key), Customer(customer_key, customer_nation),
        Nation(supplier_nation, supplier_name), Nation(customer_nation, customer_name),
        (supplier_name = "FRANCE" and customer_name = "GERMANY") or
            (supplier_name = "GERMANY" and customer_name = "FRANCE");
}
