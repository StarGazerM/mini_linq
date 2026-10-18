//! TPC-H-derived Q6 with exact scale-four revenue and SQL bag multiplicity.
//! The lineitem primary key survives the CQ head before the aggregate fold.

use cq_load::{Date, Decimal};
use std::{error::Error, path::Path};

pub const NAME: &str = "tpch_q06";
pub const SQL: &str = include_str!("../../../sql/tpch/q06.sql");
pub const EXPORTS: &[(&str, &str)] = &[(
    "lineitem",
    "l_orderkey,l_linenumber,l_quantity,l_extendedprice,l_discount,l_shipdate",
)];
pub const CONFIGS: &[&str] = &[
    "default",
    "no-reduce",
    "generic-smallest",
    "staged",
    "transfer",
    "le",
    "le_runs",
];

::mini_linq::relation_loaders! {
    relation Lineitem(
        l_orderkey: i64, l_linenumber: i64, l_quantity: Decimal,
        l_extendedprice: Decimal, l_discount: Decimal, l_shipdate: Date
    );
}

pub struct Data {
    lineitem: Vec<(i64, i64, Decimal, Decimal, Decimal, Date)>,
}

/// Coefficient at scale four; None is the SQL SUM result for empty input.
pub type Answer = Option<i128>;

pub fn load(root: &Path, _config: &str) -> Result<Data, Box<dyn Error>> {
    Ok(Data {
        lineitem: LineitemLoader::parquet(root.join("lineitem.parquet"), Default::default())?,
    })
}

macro_rules! query_body {
    () => {
        pub(super) fn execute(data: &Data) -> Vec<Answer> {
            let lineitem = data.lineitem.as_slice();
            let first: Date = "1994-01-01".parse().unwrap();
            let after: Date = "1995-01-01".parse().unwrap();
            let discount_low = Decimal::from_scaled(5).unwrap();
            let discount_high = Decimal::from_scaled(7).unwrap();
            let quantity_limit = Decimal::from_scaled(2400).unwrap();
            let aggregates = query! {
                positional(lineitem); aggregate! {
                by (group); revenue = sum(amount);
                struct Query06;
                relation lineitem(
                    orderkey: i64, linenumber: i64, quantity: Decimal,
                    price: Decimal, discount: Decimal, shipdate: Date
                );
                rows(group, orderkey, linenumber, amount) :-
                    lineitem(orderkey, linenumber, quantity, price, discount, shipdate),
                    if *shipdate >= first && *shipdate < after,
                    if *discount >= discount_low && *discount <= discount_high,
                    if *quantity < quantity_limit,
                    let group: i32 = 0,
                    let amount: i128 = i128::from(price.scaled()) * i128::from(discount.scaled());
            }};
            vec![aggregates
                .iter()
                .flat_map(|(_, values)| values.iter())
                .map(|(amount,)| *amount)
                .next()]
        }
    };
}

// Each configuration expands the same authored query through caller-selected
// planner and checked Physical/Structures rewrites.
crate::tpch_runtime::configuration!(default, $, {}, {
    query_body!();
});
crate::tpch_runtime::configuration!(generic_smallest, $,
    { plan = ::cq_planners::order(written, fine); physical = [::cq_rewrites::canon_prefixes, ::cq_rewrites::smallest_intersections]; }, {
    query_body!();
});
crate::tpch_runtime::configuration!(transfer, $, { plan = ::cq_planners::order(transfer); }, {
    query_body!();
});
crate::tpch_runtime::configuration!(le, $, { plan = ::cq_planners::order(le); }, {
    query_body!();
});
crate::tpch_runtime::configuration!(le_runs, $, {
    plan = ::cq_planners::order(le);
}, [::cq_rewrites::sorted_runs, ::cq_rewrites::retain_lookups], {
    query_body!();
});

// Recorded representation and evaluation-unit decisions: the scan chain is
// carried as positions and its total range predicates run 1,024 rows ahead.
crate::tpch_runtime::configuration!(staged, $,
    { represent = ::mini_linq::passes::represent(positions); stage = ::mini_linq::passes::stage(batch(1024)); }, {
    query_body!();
});

pub fn run(data: &Data, config: &str) -> Vec<Answer> {
    match config {
        "default" => default::execute(data),
        "staged" => staged::execute(data),
        "no-reduce" => no_reduce::execute(data),
        "generic-smallest" => generic_smallest::execute(data),
        "transfer" => transfer::execute(data),
        "le" => le::execute(data),
        "le_runs" => le_runs::execute(data),
        _ => panic!("unknown Q6 configuration: {config}"),
    }
}

pub fn canonical(rows: &[Answer]) -> Vec<Vec<String>> {
    rows.iter()
        .map(|value| vec![value.map_or_else(|| "\\N".into(), fixed_four)])
        .collect()
}

fn fixed_four(value: i128) -> String {
    let sign = if value < 0 { "-" } else { "" };
    let magnitude = value.unsigned_abs();
    format!("{sign}{}.{:04}", magnitude / 10_000, magnitude % 10_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_identity_and_filter_boundaries_preserve_sql_sum() {
        let decimal = |value| Decimal::from_scaled(value).unwrap();
        let row = |number, quantity, discount, date: &str| {
            (
                1,
                number,
                decimal(quantity),
                decimal(10_000),
                decimal(discount),
                date.parse().unwrap(),
            )
        };
        let data = Data {
            lineitem: vec![
                row(1, 2300, 5, "1994-01-01"),
                row(2, 2300, 5, "1994-01-01"),
                row(3, 2300, 7, "1994-12-31"),
                row(4, 2400, 5, "1994-01-01"),
                row(5, 2300, 4, "1994-01-01"),
                row(6, 2300, 8, "1994-01-01"),
                row(7, 2300, 5, "1995-01-01"),
                row(8, 2300, 5, "1993-12-31"),
            ],
        };
        for config in CONFIGS {
            assert_eq!(run(&data, config), [Some(170_000)]);
            assert_eq!(canonical(&run(&data, config)), [vec!["17.0000"]]);
            assert_eq!(run(&Data { lineitem: vec![] }, config), [None]);
            assert_eq!(canonical(&[None]), [vec!["\\N"]]);
        }
    }
}
