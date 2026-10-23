//! TPC-H Q11 German stock values above 0.0001 of the German total, unordered.
//! Per-part and global sums independently consume the same non-aggregate matches;
//! the final CQ joins their results and applies the original threshold exactly.
//! Supply costs and output values are scale-two integer coefficients.
use cq_load::Decimal;

pub const NAME: &str = "retail_h11";
pub const CONFIGS: &[&str] = &["default"];
pub const EXPORTS: &[(&str, &str)] = &[
    ("partsupp", "ps_partkey,ps_suppkey,ps_availqty,ps_supplycost"),
    ("supplier", "s_suppkey,s_nationkey"),
    ("nation", "n_nationkey,n_name"),
];
pub const SQL: &str = r#"WITH matches AS (
    SELECT DISTINCT ps.ps_partkey, ps.ps_suppkey, ps.ps_availqty, ps.ps_supplycost
    FROM partsupp ps JOIN supplier s ON s.s_suppkey = ps.ps_suppkey
    JOIN nation n ON n.n_nationkey = s.s_nationkey WHERE n.n_name = 'GERMANY')
SELECT ps_partkey, CAST(SUM(ps_supplycost * ps_availqty) * 100 AS BIGINT)
FROM matches GROUP BY ps_partkey
HAVING SUM(ps_supplycost * ps_availqty) >
    (SELECT SUM(ps_supplycost * ps_availqty) * 0.0001 FROM matches)"#;

pub type Partsupp = (i64, i64, i64, Decimal);
pub type Supplier = (i64, i32);
pub type Nation = (i32, String);
pub type Answer = (i64, i64);
pub struct Data {
    pub partsupp: Vec<Partsupp>,
    pub supplier: Vec<Supplier>,
    pub nation: Vec<Nation>,
}

mini_linq::relation_loaders! {
    relation Partsupp(ps_partkey: i64, ps_suppkey: i64, ps_availqty: i64, ps_supplycost: Decimal);
    relation Supplier(s_suppkey: i64, s_nationkey: i32);
    relation Nation(n_nationkey: i32, n_name: String);
}

pub fn load(root: &std::path::Path, config: &str) -> Result<Data, Box<dyn std::error::Error>> {
    assert_eq!(config, "default", "unknown H11 configuration");
    Ok(Data {
        partsupp: <relation_loader!(Partsupp)>::parquet(
            root.join("partsupp.parquet"), cq_load::ParquetOptions::default(),
        )?,
        supplier: <relation_loader!(Supplier)>::parquet(
            root.join("supplier.parquet"), cq_load::ParquetOptions::default(),
        )?,
        nation: <relation_loader!(Nation)>::parquet(
            root.join("nation.parquet"), cq_load::ParquetOptions::default(),
        )?,
    })
}

#[allow(non_snake_case)]
pub fn run(data: &Data, config: &str) -> Vec<Answer> {
    assert_eq!(config, "default", "unknown H11 configuration");
    let Partsupp = data.partsupp.as_slice();
    let Supplier = data.supplier.as_slice();
    let Nation = data.nation.as_slice();
    let Stock: Vec<(i64, i64, i64, i64)> = mini_linq::compile! {
        pipeline {}
        struct Matches;
        relation Partsupp(ps_partkey: i64, ps_suppkey: i64, ps_availqty: i64, ps_supplycost: Decimal);
        relation Supplier(s_suppkey: i64, s_nationkey: i32);
        relation Nation(n_nationkey: i32, n_name: String);
        relation answer(part_key: i64, supplier_key: i64, quantity: i64, cost_units: i64);
        answer(*part_key, *supplier_key, *quantity, *cost_units) :-
            Partsupp(part_key, supplier_key, quantity, cost),
            Supplier(supplier_key, nation_key), Nation(nation_key, nation_name),
            if nation_name == "GERMANY",
            let cost_units: i64 = cost.scaled();
    }.collect();
    let Stock = Stock.as_slice();
    let PartValue: Vec<(i64, i64)> = mini_linq::compile! {
        pipeline {}
        struct PerPart;
        relation Stock(part_key: i64, supplier_key: i64, quantity: i64, cost_units: i64);
        relation answer(part_key: i64, value: i64);
        answer(part_key, sum(value)) :- Stock(part_key, supplier_key, quantity, cost_units),
            let value: i64 = *quantity * *cost_units;
    }.map(|(part, value)| (*part, value)).collect();
    let TotalValue: Vec<(i64,)> = mini_linq::compile! {
        pipeline {}
        struct Total;
        relation Stock(part_key: i64, supplier_key: i64, quantity: i64, cost_units: i64);
        relation answer(value: i64);
        answer(sum(value)) :- Stock(part_key, supplier_key, quantity, cost_units),
            let value: i64 = *quantity * *cost_units;
    }.collect();
    let PartValue = PartValue.as_slice();
    let TotalValue = TotalValue.as_slice();
    mini_linq::compile! {
        pipeline {}
        struct Query;
        relation PartValue(part_key: i64, value: i64);
        relation TotalValue(value: i64);
        relation answer(part_key: i64, value: i64);
        answer(*part_key, *value) :- PartValue(part_key, value), TotalValue(total),
            if i128::from(*value) * 10_000 > i128::from(*total);
    }.collect()
}

pub fn canonical(rows: &[Answer]) -> Vec<Vec<String>> {
    rows.iter().map(|(part, value)| vec![part.to_string(), value.to_string()]).collect()
}
