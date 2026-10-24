// TPC-H-derived positive Q9 join core with the composite partsupp key.
// The canonical '%green%' pattern is exactly the literal substring predicate;
// profit arithmetic, grouping, and ordering remain outside this inspired core.
// The author evaluates the opaque string predicate on Part before the joins;
// its written Before domain is Part alone, not a joined prefix.

crate::tpch_runtime::retail_kernel! { $;
    name "retail_h09";
    sql r#"SELECT DISTINCT o.o_orderkey, p.p_partkey, s.s_suppkey
FROM lineitem l JOIN partsupp ps ON l.l_partkey = ps.ps_partkey AND l.l_suppkey = ps.ps_suppkey
JOIN part p ON p.p_partkey = l.l_partkey
JOIN supplier s ON s.s_suppkey = l.l_suppkey
JOIN orders o ON o.o_orderkey = l.l_orderkey
JOIN nation n ON n.n_nationkey = s.s_nationkey
WHERE p.p_name LIKE '%green%'"#;
    candidate generic_smallest;
    advise plan(no_reduce, order(Part, Lineitem, PartSupp, Orders, Supplier, Nation));
    setup {}
    relations {
        relation Lineitem => lineitem(l_orderkey: i64, l_partkey: i64, l_suppkey: i64);
        relation PartSupp => partsupp(ps_partkey: i64, ps_suppkey: i64);
        relation Part => part(p_partkey: i64, p_name: String);
        relation Supplier => supplier(s_suppkey: i64, s_nationkey: i32);
        relation Orders => orders(o_orderkey: i64);
        relation Nation => nation(n_nationkey: i32);
    }
    answer(order_key: i64, part_key: i64, supplier_key: i64) :-
        Part(part_key, part_name), if part_name.contains("green"),
        Orders(order_key),
        Lineitem(order_key, part_key, supplier_key), PartSupp(part_key, supplier_key),
        Supplier(supplier_key, nation_key), Nation(nation_key);
}
