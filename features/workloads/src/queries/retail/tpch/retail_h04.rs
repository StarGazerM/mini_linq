//! TPC-H Q4 existence core: DISTINCT joins implement EXISTS; no priority aggregation.
//! SQL below is the independent DuckDB oracle, not the full canonical Q4.
use crate::tpch_runtime::date;
use cq_load::Date;

crate::tpch_runtime::retail_kernel! { $;
    name "retail_h04";
    sql r#"SELECT DISTINCT o.o_orderkey
FROM orders o JOIN lineitem l ON l.l_orderkey = o.o_orderkey
WHERE o.o_orderdate >= DATE '1993-07-01' AND o.o_orderdate < DATE '1993-10-01'
AND l.l_commitdate < l.l_receiptdate"#;
    candidate hypergraph;
    advise plan(no_reduce, order(Lineitem, Orders));
    setup { let start = date("1993-07-01"); let end = date("1993-10-01"); }
    relations {
        relation Orders => orders(o_orderkey: i64, o_orderdate: Date);
        relation Lineitem => lineitem(l_orderkey: i64, l_commitdate: Date, l_receiptdate: Date);
    }
    answer(order_key: i64) :-
        Orders(order_key, order_date), if *order_date >= start && *order_date < end,
        Lineitem(order_key, commit_date, receipt_date), if commit_date < receipt_date;
}
