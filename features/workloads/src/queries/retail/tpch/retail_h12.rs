//! TPC-H Q12 shipmode/priority counts, without final ordering.
//! Integer indicator sums retain named order/line identities.
use cq_load::{Date, Load};

pub const NAME: &str = "retail_h12";
pub const CONFIGS: &[&str] = &["default"];
pub const EXPORTS: &[(&str, &str)] = &[
    ("orders", "o_orderkey,o_orderpriority"),
    ("lineitem", "l_orderkey,l_linenumber,l_shipmode,l_commitdate,l_receiptdate,l_shipdate"),
];
pub const SQL: &str = r#"SELECT l_shipmode,
    CAST(SUM(CASE WHEN o_orderpriority IN ('1-URGENT', '2-HIGH') THEN 1 ELSE 0 END) AS BIGINT) AS high_line_count,
    CAST(SUM(CASE WHEN o_orderpriority NOT IN ('1-URGENT', '2-HIGH') THEN 1 ELSE 0 END) AS BIGINT) AS low_line_count
FROM (SELECT DISTINCT l.l_orderkey, l.l_linenumber, l.l_shipmode, l.l_commitdate,
    l.l_receiptdate, l.l_shipdate, o.o_orderpriority
    FROM orders o JOIN lineitem l ON o.o_orderkey = l.l_orderkey
    WHERE l.l_shipmode IN ('MAIL', 'SHIP') AND l.l_commitdate < l.l_receiptdate
      AND l.l_shipdate < l.l_commitdate AND l.l_receiptdate >= DATE '1994-01-01'
      AND l.l_receiptdate < DATE '1995-01-01') AS matches
GROUP BY l_shipmode"#;

pub type Orders = (i64, String);
pub type Lineitem = (i64, i64, String, Date, Date, Date);
pub type Answer = (String, i64, i64);
pub struct Data {
    pub orders: Vec<Orders>,
    pub lineitem: Vec<Lineitem>,
}

mini_linq::relation_loaders! {
    relation Orders(o_orderkey: i64, o_orderpriority: String);
    relation Lineitem(l_orderkey: i64, l_linenumber: i64, l_shipmode: String,
        l_commitdate: Date, l_receiptdate: Date, l_shipdate: Date);
}

pub fn load(root: &std::path::Path, config: &str) -> Result<Data, Box<dyn std::error::Error>> {
    assert_eq!(config, "default", "unknown H12 configuration");
    Ok(Data {
        orders: <relation_loader!(Orders)>::parquet(
            root.join("orders.parquet"), cq_load::ParquetOptions::default(),
        )?,
        lineitem: <relation_loader!(Lineitem)>::parquet(
            root.join("lineitem.parquet"), cq_load::ParquetOptions::default(),
        )?,
    })
}

#[allow(non_snake_case)]
pub fn run(data: &Data, config: &str) -> Vec<Answer> {
    assert_eq!(config, "default", "unknown H12 configuration");
    let Orders = data.orders.as_slice();
    let Lineitem = data.lineitem.as_slice();
    let start = Date::load("1994-01-01").expect("authored ISO date");
    let end = Date::load("1995-01-01").expect("authored ISO date");
    mini_linq::compile! {
        pipeline {}
        struct Query;
        relation Orders(o_orderkey: i64, o_orderpriority: String);
        relation Lineitem(l_orderkey: i64, l_linenumber: i64, l_shipmode: String,
            l_commitdate: Date, l_receiptdate: Date, l_shipdate: Date);
        relation answer(shipmode: String, high_line_count: i64, low_line_count: i64);
        answer(shipmode, sum(high), sum(low)) :-
            Lineitem(order_key, line_number, shipmode, commitdate, receiptdate, shipdate),
            if shipmode == "MAIL" || shipmode == "SHIP",
            if *shipdate < *commitdate && *commitdate < *receiptdate,
            if *receiptdate >= start && *receiptdate < end,
            Orders(order_key, priority),
            let high: i64 = if priority == "1-URGENT" || priority == "2-HIGH" { 1 } else { 0 },
            let low: i64 = if priority == "1-URGENT" || priority == "2-HIGH" { 0 } else { 1 };
    }
    .map(|(mode, high, low)| (mode.clone(), high, low))
    .collect()
}

pub fn canonical(rows: &[Answer]) -> Vec<Vec<String>> {
    rows.iter().map(|(mode, high, low)| {
        vec![mode.clone(), high.to_string(), low.to_string()]
    }).collect()
}
