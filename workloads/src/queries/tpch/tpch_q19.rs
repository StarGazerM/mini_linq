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
pub const CONFIGS: &[&str] = &["default", "generic-smallest"];

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

pub fn load(root: &Path) -> Result<Data, Box<dyn Error>> {
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
            let computed: Vec<(i32, i64, i64, i128)> = query! {
                struct Query19;
                relation lineitem(
                    partkey: i64, orderkey: i64, linenumber: i64,
                    quantity: Decimal, price: Decimal, discount: Decimal,
                    shipmode: String, shipinstruct: String
                );
                relation part(partkey: i64, brand: String, container: String, size: i32);
                rows(group, orderkey, linenumber, amount) :-
                    part(partkey, brand, container, size),
                    lineitem(partkey, orderkey, linenumber, quantity, price, discount, shipmode, shipinstruct),
                    shipmode in { "AIR", "AIR REG" },
                    shipinstruct = "DELIVER IN PERSON",
                    (brand = "Brand#12"
                        and container in { "SM CASE", "SM BOX", "SM PACK", "SM PKG" }
                        and quantity between q1_low and q1_high and size between 1 and 5)
                    or (brand = "Brand#23"
                        and container in { "MED BAG", "MED BOX", "MED PKG", "MED PACK" }
                        and quantity between q2_low and q2_high and size between 1 and 10)
                    or (brand = "Brand#34"
                        and container in { "LG CASE", "LG BOX", "LG PACK", "LG PKG" }
                        and quantity between q3_low and q3_high and size between 1 and 15),
                    let group: i32 = 0,
                    let amount: i128 = i128::from(price.scaled()) * (100 - i128::from(discount.scaled()));
            }.map(|(group, orderkey, linenumber, amount)| (group, *orderkey, *linenumber, amount)).collect();
            // Aggregate currently expects borrowed measure columns. Materialize
            // the authored CQ's computed rows, then compose the aggregate hole.
            // This allocation and both CQ executions remain inside query time.
            let computed = computed.as_slice();
            let aggregates = ::cq_ext_aggregate::aggregate! {
                by (group); revenue = sum(amount);
                struct Revenue;
                relation computed(group: i32, orderkey: i64, linenumber: i64, amount: i128);
                rows(group, orderkey, linenumber, amount) :- computed(group, orderkey, linenumber, amount);
            };
            vec![aggregates
                .iter()
                .flat_map(|(_, values)| values.iter())
                .map(|(amount,)| *amount)
                .next()]
        }
    };
}

crate::tpch_runtime::configuration!(default, $, cq_to_ra, {}, {
    query_body!();
});
crate::tpch_runtime::configuration!(generic_smallest, $, cq_to_ra_generic,
    { project = canon; intersect = smallest; }, {
    query_body!();
});

pub fn run(data: &Data, config: &str) -> Vec<Answer> {
    match config {
        "default" => default::execute(data),
        "generic-smallest" => generic_smallest::execute(data),
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
