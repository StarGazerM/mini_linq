//! String columns use load-time sorted dictionary codes throughout query execution.
//! Dictionary predicate memberships are prepared once during loading.
//! Text is decoded only when publishing the final answer.
//! TPC-H Q12 shipmode/priority counts, without final ordering.
//! Total and high-priority counts retain named order/line identities.
//! Low counts are their difference; modes without high-priority lines survive.
use cq_load::{Date, Load};

use cq_load::StringCode;

pub const NAME: &str = "retail_h12";

pub const EXPORTS: &[(&str, &str)] = &[
    ("orders", "o_orderkey,o_orderpriority"),
    (
        "lineitem",
        "l_orderkey,l_linenumber,l_shipmode,l_commitdate,l_receiptdate,l_shipdate",
    ),
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

pub type Orders = (i64, StringCode);
pub type Lineitem = (i64, i64, StringCode, Date, Date, Date);
pub type Answer = (String, i64, i64);
pub struct Data {
    pub orders: Vec<Orders>,
    pub lineitem: Vec<Lineitem>,
    pub high_priorities: Vec<(StringCode,)>,
    pub dictionaries: cq_load::Dictionaries,
}

mini_linq::interned_relation_loaders! {
    relation Orders(o_orderkey: i64, o_orderpriority: StringCode);
    relation Lineitem(l_orderkey: i64, l_linenumber: i64, l_shipmode: StringCode,
        l_commitdate: Date, l_receiptdate: Date, l_shipdate: Date);
}

pub fn load(root: &std::path::Path, config: &str) -> Result<Data, Box<dyn std::error::Error>> {
    assert!(CONFIGS.contains(&config), "unknown H12 configuration");
    let mut dictionaries = cq_load::Dictionaries::default();
    Ok(Data {
        orders: <relation_loader!(Orders)>::parquet_interned(
            root.join("orders.parquet"),
            cq_load::ParquetOptions::default(),
            &mut dictionaries,
        )?,
        lineitem: <relation_loader!(Lineitem)>::parquet_interned(
            root.join("lineitem.parquet"),
            cq_load::ParquetOptions::default(),
            &mut dictionaries,
        )?,
        high_priorities: cq_load::dictionary_where(
            &dictionaries,
            "o_orderpriority",
            |value: &str| value == "1-URGENT" || value == "2-HIGH",
        ),
        dictionaries,
    })
}

crate::tpch_runtime::standalone_configs! { $;
    pub(super) fn run(data: &Data, config: &str) -> Vec<Answer> {
        assert!(CONFIGS.contains(&config), "unknown H12 configuration");
        let mail = data.dictionaries.literal("l_shipmode", "MAIL");
        let ship = data.dictionaries.literal("l_shipmode", "SHIP");
        let Orders = data.orders.as_slice();
        let Lineitem = data.lineitem.as_slice();
        let HighPriorities = data.high_priorities.as_slice();
        let start = Date::load("1994-01-01").expect("authored ISO date");
        let end = Date::load("1995-01-01").expect("authored ISO date");
        let MatchingLines: Vec<(i64, i64, StringCode, Date, Date, Date, StringCode)> = query! {
            plan(no_reduce, order(Lineitem, Orders));
            struct Matches;
            relation Orders(o_orderkey: i64, o_orderpriority: StringCode);
            relation Lineitem(l_orderkey: i64, l_linenumber: i64, l_shipmode: StringCode,
                l_commitdate: Date, l_receiptdate: Date, l_shipdate: Date);
            relation answer(order_key: i64, line_number: i64, shipmode: StringCode,
                commitdate: Date, receiptdate: Date, shipdate: Date, priority: StringCode);
            answer(order_key, line_number, shipmode, commitdate, receiptdate, shipdate, priority) :-
                Lineitem(order_key, line_number, shipmode, commitdate, receiptdate, shipdate),
                where shipmode == mail || shipmode == ship,
                where shipdate < commitdate && commitdate < receiptdate,
                where receiptdate >= start && receiptdate < end,
                Orders(order_key, priority);
        }
        .map(|(order, line, mode, commit, receipt, ship, priority)|
            (*order, *line, *mode, *commit, *receipt, *ship, *priority))
        .collect();
        let MatchingLines = MatchingLines.as_slice();
        let Totals: Vec<(StringCode, i64)> = query! {
            plan(no_reduce, order(MatchingLines));
            struct TotalCounts;
            relation MatchingLines(order_key: i64, line_number: i64, shipmode: StringCode,
                commitdate: Date, receiptdate: Date, shipdate: Date, priority: StringCode);
            relation answer(shipmode: StringCode, total: i64);
            answer(shipmode, sum(one)) :-
                MatchingLines(order_key, line_number, shipmode, commitdate, receiptdate, shipdate, priority),
                let one: i64 = 1;
        }
        .map(|(mode, total)| (*mode, total))
        .collect();
        let Totals = Totals.as_slice();
        let HighCounts: Vec<(StringCode, i64)> = query! {
            plan(no_reduce, order(MatchingLines, HighPriorities));
            struct HighPriorityCounts;
            relation MatchingLines(order_key: i64, line_number: i64, shipmode: StringCode,
                commitdate: Date, receiptdate: Date, shipdate: Date, priority: StringCode);
            relation HighPriorities(name: StringCode);
            relation answer(shipmode: StringCode, high: i64);
            answer(shipmode, sum(one)) :-
                MatchingLines(order_key, line_number, shipmode, commitdate, receiptdate, shipdate, priority),
                HighPriorities(priority), let one: i64 = 1;
        }
        .map(|(mode, high)| (*mode, high))
        .collect();
        let HighCounts = HighCounts.as_slice();
        let with_high = query! {
            plan(no_reduce, order(Totals, HighCounts));
            struct WithHigh;
            relation Totals(shipmode: StringCode, total: i64);
            relation HighCounts(shipmode: StringCode, high: i64);
            relation answer(shipmode: StringCode, high_line_count: i64, low_line_count: i64);
            answer(shipmode, high, *low) :-
                Totals(shipmode, total), HighCounts(shipmode, high),
                let low: i64 = *total - *high;
        }
        .map(|(mode, high, low)| (*mode, *high, low));
        let without_high = query! {
            plan(no_reduce, order(Totals));
            struct WithoutHigh;
            relation Totals(shipmode: StringCode, total: i64);
            relation HighCounts(shipmode: StringCode, high: i64);
            relation answer(shipmode: StringCode, high_line_count: i64, low_line_count: i64);
            answer(shipmode, 0, total) :- Totals(shipmode, total), !HighCounts(shipmode, _);
        }
        .map(|(mode, high, low)| (*mode, high, *low));
        with_high.chain(without_high)
            .map(|(mode, high, low)| {
                // DECODE P: publish l_shipmode in H12 answer.
                (data.dictionaries.publish("l_shipmode", mode), high, low)
            })
            .collect()
    }
}

pub fn canonical(rows: &[Answer]) -> Vec<Vec<String>> {
    rows.iter()
        .map(|(mode, high, low)| vec![mode.clone(), high.to_string(), low.to_string()])
        .collect()
}
