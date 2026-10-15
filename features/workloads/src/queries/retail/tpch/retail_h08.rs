// TPC-H-derived positive Q8 join core. The market-share ratio is not this kernel.
use crate::tpch_runtime::date;
use cq_load::Date;

crate::tpch_runtime::retail_kernel! { $;
    name "retail_h08";
    sql r#"SELECT DISTINCT o.o_orderkey, p.p_partkey, s.s_suppkey
FROM lineitem l JOIN part p ON l.l_partkey = p.p_partkey
JOIN supplier s ON l.l_suppkey = s.s_suppkey
JOIN orders o ON l.l_orderkey = o.o_orderkey
JOIN customer c ON o.o_custkey = c.c_custkey
JOIN nation nc ON c.c_nationkey = nc.n_nationkey
JOIN region r ON nc.n_regionkey = r.r_regionkey
JOIN nation ns ON s.s_nationkey = ns.n_nationkey
WHERE r.r_name = 'AMERICA' AND p.p_type = 'ECONOMY ANODIZED STEEL'
AND o.o_orderdate >= DATE '1995-01-01' AND o.o_orderdate <= DATE '1996-12-31'"#;
    candidate hypergraph;
    setup { let start = date("1995-01-01"); let end = date("1996-12-31"); }
    relations {
        relation Lineitem => lineitem(l_orderkey: i64, l_partkey: i64, l_suppkey: i64);
        relation Part => part(p_partkey: i64, p_type: String);
        relation Supplier => supplier(s_suppkey: i64, s_nationkey: i32);
        relation Orders => orders(o_orderkey: i64, o_custkey: i64, o_orderdate: Date);
        relation Customer => customer(c_custkey: i64, c_nationkey: i32);
        relation Nation => nation(n_nationkey: i32, n_regionkey: i32);
        relation Region => region(r_regionkey: i32, r_name: String);
    }
    answer(order_key: i64, part_key: i64, supplier_key: i64) :-
        Lineitem(order_key, part_key, supplier_key),
        Part(part_key, "ECONOMY ANODIZED STEEL"),
        Supplier(supplier_key, supplier_nation),
        Orders(order_key, customer_key, order_date), if *order_date >= start && *order_date <= end,
        Customer(customer_key, customer_nation), Nation(customer_nation, customer_region),
        Region(customer_region, "AMERICA"),
        Nation(supplier_nation, _supplier_region);
}
