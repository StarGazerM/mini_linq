//! TPC-H-derived Q19 with a positive join, relational comparison predicates and exact SUM.
//! Part keys are unique in dbgen; retaining the lineitem primary key preserves
//! SQL multiplicity even when different lineitems have equal revenues.

use cq_load::Decimal;
use std::{error::Error, path::Path};

pub const NAME: &str = "tpch_q19";
pub const SQL: &str = include_str!("../../../sql/tpch/q19.sql");
pub const EXPORTS: &[(&str, &str)] = &[
    (
        "lineitem",
        "l_partkey,l_orderkey,l_linenumber,l_quantity,l_extendedprice,l_discount,l_shipmode,l_shipinstruct",
    ),
    ("part", "p_partkey,p_brand,p_container,p_size"),
];
pub const CONFIGS: &[&str] = &[
    "default",
    "radix",
    "radix_retained",
    "built_hash",
    "built_radix",
    "par",
    "no-reduce",
    "generic-smallest",
    "transfer",
    "le",
    "le_runs",
];

::mini_linq::relation_loaders! {
    relation Lineitem(
        l_partkey: i64, l_orderkey: i64, l_linenumber: i64,
        l_quantity: Decimal, l_extendedprice: Decimal, l_discount: Decimal,
        l_shipmode: String, l_shipinstruct: String
    );
    relation Part(p_partkey: i64, p_brand: String, p_container: String, p_size: i32);
}

type LineitemRow = (i64, i64, i64, Decimal, Decimal, Decimal, String, String);
pub struct Data {
    lineitem: Vec<LineitemRow>,
    part: Vec<(i64, String, String, i32)>,
}

/// Coefficient at scale four; None is the SQL SUM result for empty input.
pub type Answer = Option<i128>;

pub fn load(root: &Path, _config: &str) -> Result<Data, Box<dyn Error>> {
    Ok(Data {
        lineitem: LineitemLoader::parquet(root.join("lineitem.parquet"), Default::default())?,
        part: PartLoader::parquet(root.join("part.parquet"), Default::default())?,
    })
}

macro_rules! query_body {
    () => {
        pub(super) fn execute(data: &Data) -> Vec<Answer> {
            let lineitem = data.lineitem.as_slice();
            let part = data.part.as_slice();
            let q1_low = Decimal::from_scaled(100).unwrap();
            let q1_high = Decimal::from_scaled(1100).unwrap();
            let q2_low = Decimal::from_scaled(1000).unwrap();
            let q2_high = Decimal::from_scaled(2000).unwrap();
            let q3_low = Decimal::from_scaled(2000).unwrap();
            let q3_high = Decimal::from_scaled(3000).unwrap();
            let aggregates = query! {
                positional(part,lineitem); aggregate! {
                by (group); revenue = sum(amount);
                struct Query19;
                relation lineitem(
                    partkey: i64, orderkey: i64, linenumber: i64,
                    quantity: Decimal, price: Decimal, discount: Decimal,
                    shipmode: String, shipinstruct: String
                );
                relation part(partkey: i64, brand: String, container: String, size: i32);
                rows(group, orderkey, linenumber, amount) :-
                    lineitem(partkey, orderkey, linenumber, quantity, price, discount, shipmode, shipinstruct),
                    part(partkey, brand, container, size),
                    if *shipmode == "AIR" || *shipmode == "AIR REG",
                    if *shipinstruct == "DELIVER IN PERSON",
                    if (*brand == "Brand#12"
                        && (*container == "SM CASE" || *container == "SM BOX" || *container == "SM PACK" || *container == "SM PKG")
                        && *quantity >= q1_low && *quantity <= q1_high && *size >= 1 && *size <= 5)
                    || (*brand == "Brand#23"
                        && (*container == "MED BAG" || *container == "MED BOX" || *container == "MED PKG" || *container == "MED PACK")
                        && *quantity >= q2_low && *quantity <= q2_high && *size >= 1 && *size <= 10)
                    || (*brand == "Brand#34"
                        && (*container == "LG CASE" || *container == "LG BOX" || *container == "LG PACK" || *container == "LG PKG")
                        && *quantity >= q3_low && *quantity <= q3_high && *size >= 1 && *size <= 15),
                    let group: i32 = 0,
                    let amount: i128 = i128::from(price.scaled()) * (100 - i128::from(discount.scaled()));
            }};
            vec![aggregates
                .iter()
                .flat_map(|(_, values)| values.iter())
                .map(|(amount,)| *amount)
                .next()]
        }
    };
}

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

pub fn run(data: &Data, config: &str) -> Vec<Answer> {
    match config {
        "default" => default::execute(data),
        "radix" => radix::execute(data),
        "radix_retained" => radix_retained::execute(data),
        "built_hash" => built_hash::execute(data),
        "built_radix" => built_radix::execute(data),
        "par" => par::execute(data),
        "no-reduce" => no_reduce::execute(data),
        "generic-smallest" => generic_smallest::execute(data),
        "transfer" => transfer::execute(data),
        "le" => le::execute(data),
        "le_runs" => le_runs::execute(data),
        _ => panic!("unknown Q19 configuration: {config}"),
    }
}

pub fn canonical(rows: &[Answer]) -> Vec<Vec<String>> {
    rows.iter()
        .map(|value| {
            vec![value.map_or_else(
                || "\\N".into(),
                |value| {
                    let sign = if value < 0 { "-" } else { "" };
                    let magnitude = value.unsigned_abs();
                    format!("{sign}{}.{:04}", magnitude / 10_000, magnitude % 10_000)
                },
            )]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branches_boundaries_and_equal_valued_lines_preserve_sql_sum() {
        let decimal = |value| Decimal::from_scaled(value).unwrap();
        let row = |key, number, quantity, mode: &str, instruction: &str| {
            (
                key,
                1,
                number,
                decimal(quantity),
                decimal(10_000),
                decimal(5),
                mode.into(),
                instruction.into(),
            )
        };
        let data = Data {
            part: vec![
                (1, "Brand#12".into(), "SM CASE".into(), 5),
                (2, "Brand#23".into(), "MED BAG".into(), 10),
                (3, "Brand#34".into(), "LG PKG".into(), 15),
                (4, "Brand#12".into(), "SM CASE".into(), 6),
            ],
            lineitem: vec![
                row(1, 1, 100, "AIR", "DELIVER IN PERSON"),
                row(1, 2, 1100, "AIR REG", "DELIVER IN PERSON"),
                row(2, 3, 1000, "AIR", "DELIVER IN PERSON"),
                row(2, 4, 2000, "AIR", "DELIVER IN PERSON"),
                row(3, 5, 2000, "AIR", "DELIVER IN PERSON"),
                row(3, 6, 3000, "AIR", "DELIVER IN PERSON"),
                row(1, 7, 99, "AIR", "DELIVER IN PERSON"),
                row(1, 8, 1101, "AIR", "DELIVER IN PERSON"),
                row(2, 9, 1000, "SHIP", "DELIVER IN PERSON"),
                row(3, 10, 2000, "AIR", "NONE"),
                row(4, 11, 100, "AIR", "DELIVER IN PERSON"),
                row(5, 12, 100, "AIR", "DELIVER IN PERSON"),
            ],
        };
        for config in CONFIGS {
            assert_eq!(run(&data, config), [Some(5_700_000)]);
            assert_eq!(canonical(&run(&data, config)), [vec!["570.0000"]]);
            assert_eq!(
                run(
                    &Data {
                        lineitem: vec![],
                        part: vec![]
                    },
                    config
                ),
                [None]
            );
        }
    }
}
