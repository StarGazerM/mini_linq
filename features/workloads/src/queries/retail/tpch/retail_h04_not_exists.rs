//! Q4's NOT EXISTS variant: orders without a late lineitem in the Q4 quarter.
//! SQL below is the independent DuckDB oracle; priority aggregation is omitted.
use crate::tpch_runtime::date;
use cq_load::Date;

crate::tpch_runtime::retail_kernel! { $;
    name "retail_h04_not_exists";
    sql r#"SELECT DISTINCT o.o_orderkey FROM orders o
WHERE o.o_orderdate >= DATE '1993-07-01' AND o.o_orderdate < DATE '1993-10-01'
AND NOT EXISTS (SELECT 1 FROM lineitem l WHERE l.l_orderkey = o.o_orderkey
AND l.l_commitdate < l.l_receiptdate)"#;
    candidate hypergraph;
    setup {
        let start = date("1993-07-01"); let end = date("1993-10-01");
        let LateLineitem: Vec<(i64,)> = Lineitem.iter()
            .filter(|row| row.1 < row.2).map(|row| (row.0,)).collect();
    }
    relations {
        relation Orders => orders(o_orderkey: i64, o_orderdate: Date);
        relation Lineitem => lineitem(l_orderkey: i64, l_commitdate: Date, l_receiptdate: Date);
    }
    derived { relation LateLineitem(order_key: i64); }
    answer(order_key: i64) :-
        Orders(order_key, order_date), if *order_date >= start && *order_date < end,
        !LateLineitem(order_key);
}
