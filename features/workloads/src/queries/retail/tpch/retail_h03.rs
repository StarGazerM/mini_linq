// TPC-H-derived positive Q3 join core: no revenue, grouping, ordering, or limit.
use crate::tpch_runtime::date;
use cq_load::Date;

crate::tpch_runtime::retail_kernel! { $;
    name "retail_h03";
    sql r#"SELECT DISTINCT o.o_orderkey, c.c_custkey
FROM customer c JOIN orders o ON o.o_custkey = c.c_custkey
JOIN lineitem l ON l.l_orderkey = o.o_orderkey
WHERE c.c_mktsegment = 'BUILDING' AND o.o_orderdate < DATE '1995-03-15'
AND l.l_shipdate > DATE '1995-03-15'"#;
    candidate hypergraph;
    advise join_filter ::cq_join_filters::BloomFilter<16384>;
    setup { let cutoff = date("1995-03-15"); }
    relations {
        relation Customer => customer(c_custkey: i64, c_mktsegment: String);
        relation Orders => orders(o_orderkey: i64, o_custkey: i64, o_orderdate: Date);
        relation Lineitem => lineitem(l_orderkey: i64, l_shipdate: Date);
    }
    answer(order_key: i64, customer_key: i64) :-
        Customer(customer_key, "BUILDING"),
        Orders(order_key, customer_key, order_date), if *order_date < cutoff,
        Lineitem(order_key, ship_date), if *ship_date > cutoff;
}
