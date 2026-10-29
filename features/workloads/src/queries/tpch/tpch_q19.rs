//! TPC-H-derived Q19 with a positive join, relational comparison predicates and exact SUM.
//! Part keys are unique in dbgen; retaining the lineitem primary key preserves
//! SQL multiplicity even when different lineitems have equal revenues.
//! Text columns use load-time sorted dictionary codes, including query literals.

use cq_load::{Decimal, StringCode};
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

::mini_linq::interned_relation_loaders! {
    relation Lineitem(
        l_partkey: i64, l_orderkey: i64, l_linenumber: i64,
        l_quantity: Decimal, l_extendedprice: Decimal, l_discount: Decimal,
        l_shipmode: StringCode, l_shipinstruct: StringCode
    );
    relation Part(p_partkey: i64, p_brand: StringCode, p_container: StringCode, p_size: i32);
}

type LineitemRow = (i64, i64, i64, Decimal, Decimal, Decimal, StringCode, StringCode);
pub struct Data {
    lineitem: Vec<LineitemRow>,
    part: Vec<(i64, StringCode, StringCode, i32)>,
    dictionaries: cq_load::Dictionaries,
}

/// Coefficient at scale four; None is the SQL SUM result for empty input.
pub type Answer = Option<i128>;

pub fn load(root: &Path, _config: &str) -> Result<Data, Box<dyn Error>> {
    let mut dictionaries = cq_load::Dictionaries::default();
    Ok(Data {
        lineitem: LineitemLoader::parquet_interned(
            root.join("lineitem.parquet"), Default::default(), &mut dictionaries)?,
        part: PartLoader::parquet_interned(
            root.join("part.parquet"), Default::default(), &mut dictionaries)?,
        dictionaries,
    })
}

macro_rules! query_body {
    () => {
        pub(super) fn execute(data: &Data) -> Vec<Answer> {
            let lineitem = data.lineitem.as_slice();
            let part = data.part.as_slice();
            let air = data.dictionaries.literal("l_shipmode", "AIR");
            let air_reg = data.dictionaries.literal("l_shipmode", "AIR REG");
            let deliver = data.dictionaries.literal("l_shipinstruct", "DELIVER IN PERSON");
            let brand12 = data.dictionaries.literal("p_brand", "Brand#12");
            let brand23 = data.dictionaries.literal("p_brand", "Brand#23");
            let brand34 = data.dictionaries.literal("p_brand", "Brand#34");
            let sm_case = data.dictionaries.literal("p_container", "SM CASE");
            let sm_box = data.dictionaries.literal("p_container", "SM BOX");
            let sm_pack = data.dictionaries.literal("p_container", "SM PACK");
            let sm_pkg = data.dictionaries.literal("p_container", "SM PKG");
            let med_bag = data.dictionaries.literal("p_container", "MED BAG");
            let med_box = data.dictionaries.literal("p_container", "MED BOX");
            let med_pkg = data.dictionaries.literal("p_container", "MED PKG");
            let med_pack = data.dictionaries.literal("p_container", "MED PACK");
            let lg_case = data.dictionaries.literal("p_container", "LG CASE");
            let lg_box = data.dictionaries.literal("p_container", "LG BOX");
            let lg_pack = data.dictionaries.literal("p_container", "LG PACK");
            let lg_pkg = data.dictionaries.literal("p_container", "LG PKG");
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
                    shipmode: StringCode, shipinstruct: StringCode
                );
                relation part(partkey: i64, brand: StringCode, container: StringCode, size: i32);
                rows(group, orderkey, linenumber, amount) :-
                    lineitem(partkey, orderkey, linenumber, quantity, price, discount, shipmode, shipinstruct),
                    part(partkey, brand, container, size),
                    where shipmode == air || shipmode == air_reg,
                    where shipinstruct == deliver,
                    where (brand == brand12
                        && (container == sm_case || container == sm_box || container == sm_pack || container == sm_pkg)
                        && quantity >= q1_low && quantity <= q1_high && size >= 1 && size <= 5)
                    || (brand == brand23
                        && (container == med_bag || container == med_box || container == med_pkg || container == med_pack)
                        && quantity >= q2_low && quantity <= q2_high && size >= 1 && size <= 10)
                    || (brand == brand34
                        && (container == lg_case || container == lg_box || container == lg_pack || container == lg_pkg)
                        && quantity >= q3_low && quantity <= q3_high && size >= 1 && size <= 15),
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
        let mut dictionaries = cq_load::Dictionaries::default();
        dictionaries.insert("p_brand", ["Brand#12", "Brand#23", "Brand#34"]);
        dictionaries.insert("p_container", ["SM CASE", "MED BAG", "LG PKG"]);
        dictionaries.insert("l_shipmode", ["AIR", "AIR REG", "SHIP"]);
        dictionaries.insert("l_shipinstruct", ["DELIVER IN PERSON", "NONE"]);
        let row = |key, number, quantity, mode: &str, instruction: &str| {
            (
                key,
                1,
                number,
                decimal(quantity),
                decimal(10_000),
                decimal(5),
                dictionaries.literal("l_shipmode", mode),
                dictionaries.literal("l_shipinstruct", instruction),
            )
        };
        let data = Data {
            part: vec![
                (1, dictionaries.literal("p_brand", "Brand#12"), dictionaries.literal("p_container", "SM CASE"), 5),
                (2, dictionaries.literal("p_brand", "Brand#23"), dictionaries.literal("p_container", "MED BAG"), 10),
                (3, dictionaries.literal("p_brand", "Brand#34"), dictionaries.literal("p_container", "LG PKG"), 15),
                (4, dictionaries.literal("p_brand", "Brand#12"), dictionaries.literal("p_container", "SM CASE"), 6),
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
            dictionaries,
        };
        for config in CONFIGS {
            assert_eq!(run(&data, config), [Some(5_700_000)]);
            assert_eq!(canonical(&run(&data, config)), [vec!["570.0000"]]);
            assert_eq!(
                run(
                    &Data {
                        lineitem: vec![],
                        part: vec![],
                        dictionaries: cq_load::Dictionaries::default(),
                    },
                    config
                ),
                [None]
            );
        }
    }
}
