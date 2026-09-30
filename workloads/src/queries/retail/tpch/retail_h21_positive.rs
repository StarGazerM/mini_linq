// TPC-H-derived positive Q21 join core: a late Saudi supplier and a different
// supplier on a finalized order. The NOT EXISTS condition and counts of Q21
// are deliberately absent, so this query is only the named positive core.
use cq_load::Date;

crate::tpch_runtime::retail_kernel! { $;
    name "retail_h21_positive";
    sql r#"SELECT DISTINCT l1.l_orderkey, s.s_suppkey, l2.l_suppkey
FROM supplier s JOIN lineitem l1 ON s.s_suppkey = l1.l_suppkey
JOIN orders o ON o.o_orderkey = l1.l_orderkey
JOIN nation n ON n.n_nationkey = s.s_nationkey
JOIN lineitem l2 ON l1.l_orderkey = l2.l_orderkey
WHERE n.n_name = 'SAUDI ARABIA' AND o.o_orderstatus = 'F'
AND l1.l_receiptdate > l1.l_commitdate AND l2.l_suppkey <> l1.l_suppkey"#;
    candidate generic_smallest;
    setup {}
    relations {
        relation Supplier => supplier(s_suppkey: i64, s_nationkey: i32);
        relation Lineitem => lineitem(l_orderkey: i64, l_suppkey: i64, l_receiptdate: Date, l_commitdate: Date);
        relation Orders => orders(o_orderkey: i64, o_orderstatus: String);
        relation Nation => nation(n_nationkey: i32, n_name: String);
    }
    answer(order_key: i64, waiting_supplier: i64, other_supplier: i64) :-
        Supplier(waiting_supplier, nation_key),
        Lineitem(order_key, waiting_supplier, receipt_date, commit_date), if receipt_date > commit_date,
        Orders(order_key, status), if status == "F",
        Nation(nation_key, nation_name), if nation_name == "SAUDI ARABIA",
        Lineitem(order_key, other_supplier, _other_receipt, _other_commit),
        if waiting_supplier != other_supplier;
}
