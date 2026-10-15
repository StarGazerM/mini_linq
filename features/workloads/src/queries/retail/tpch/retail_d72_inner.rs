// TPC-H-derived re-authoring of the TPC-DS-inspired D72 inner positive core.
// Roles: catalog_sales -> lineitem/orders; inventory -> partsupp;
// warehouse -> supplier; item -> part; customer_demographics -> customer;
// household_demographics -> nation; sold/ship dates -> real order/ship dates.
// The original inventory-week equality has no dbgen counterpart: this explicitly
// different kernel joins stock by (part,supplier) and supplier/customer by nation,
// with a real shipping interval. It does not pretend to preserve D72 semantics.
use crate::tpch_runtime::date;
use cq_load::{Date, Decimal};

crate::tpch_runtime::retail_kernel! { $;
    name "retail_d72_inner";
    sql r#"SELECT DISTINCT o.o_orderkey, p.p_partkey, s.s_suppkey, n.n_nationkey
FROM lineitem l JOIN orders o ON l.l_orderkey = o.o_orderkey
JOIN partsupp ps ON l.l_partkey = ps.ps_partkey AND l.l_suppkey = ps.ps_suppkey
JOIN supplier s ON ps.ps_suppkey = s.s_suppkey
JOIN part p ON ps.ps_partkey = p.p_partkey
JOIN customer c ON o.o_custkey = c.c_custkey AND c.c_nationkey = s.s_nationkey
JOIN nation n ON c.c_nationkey = n.n_nationkey
WHERE l.l_shipdate >= DATE '1995-01-01' AND l.l_shipdate < DATE '1995-04-01'
AND l.l_quantity > 20.00 AND ps.ps_availqty > 0"#;
    candidate generic_smallest;
    // ps_availqty > 0 keeps every PartSupp row, while the Lineitem window keeps
    // 124,846 of 800,000 (part, supplier) keys: reduce PartSupp by Lineitem
    // instead of Lineitem by PartSupp.
    advise plan(skip(Lineitem with PartSupp), reduce(PartSupp with Lineitem));
    setup {
        let start = date("1995-01-01"); let end = date("1995-04-01");
        let minimum = Decimal::from_scaled(2000).expect("20.00");
    }
    relations {
        relation Lineitem => lineitem(l_orderkey: i64, l_partkey: i64, l_suppkey: i64, l_shipdate: Date, l_quantity: Decimal);
        relation Orders => orders(o_orderkey: i64, o_custkey: i64);
        relation PartSupp => partsupp(ps_partkey: i64, ps_suppkey: i64, ps_availqty: i64);
        relation Supplier => supplier(s_suppkey: i64, s_nationkey: i32);
        relation Part => part(p_partkey: i64);
        relation Customer => customer(c_custkey: i64, c_nationkey: i32);
        relation Nation => nation(n_nationkey: i32);
    }
    answer(order_key: i64, part_key: i64, supplier_key: i64, nation_key: i32) :-
        Orders(order_key, customer_key),
        Lineitem(order_key, part_key, supplier_key, ship_date, quantity),
        if *ship_date >= start && *ship_date < end, if *quantity > minimum,
        PartSupp(part_key, supplier_key, available), if *available > 0,
        Supplier(supplier_key, nation_key), Part(part_key), Customer(customer_key, nation_key),
        Nation(nation_key);
}
