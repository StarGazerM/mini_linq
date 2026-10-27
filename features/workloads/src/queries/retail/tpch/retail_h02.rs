//! TPC-H Q2 minimum-cost European suppliers, without final ordering or LIMIT.
//! A keyed minimum is materialized and joined back; no aggregate consumes another
//! aggregate. All tied minimum-cost suppliers survive. Money uses scale-two units.
use cq_load::Decimal;

pub const NAME: &str = "retail_h02";

pub const EXPORTS: &[(&str, &str)] = &[
    ("part", "p_partkey,p_mfgr,p_size,p_type"),
    ("partsupp", "ps_partkey,ps_suppkey,ps_supplycost"),
    (
        "supplier",
        "s_suppkey,s_nationkey,s_acctbal,s_name,s_address,s_phone,s_comment",
    ),
    ("nation", "n_nationkey,n_regionkey,n_name"),
    ("region", "r_regionkey,r_name"),
];
pub const SQL: &str = r#"SELECT DISTINCT CAST(s.s_acctbal * 100 AS BIGINT),
    s.s_name, n.n_name, p.p_partkey, p.p_mfgr, s.s_address, s.s_phone, s.s_comment
FROM part p JOIN partsupp ps ON ps.ps_partkey = p.p_partkey
JOIN supplier s ON s.s_suppkey = ps.ps_suppkey
JOIN nation n ON n.n_nationkey = s.s_nationkey
JOIN region r ON r.r_regionkey = n.n_regionkey
WHERE p.p_size = 15 AND p.p_type LIKE '%BRASS' AND r.r_name = 'EUROPE'
AND ps.ps_supplycost = (
    SELECT MIN(ps2.ps_supplycost) FROM partsupp ps2
    JOIN supplier s2 ON s2.s_suppkey = ps2.ps_suppkey
    JOIN nation n2 ON n2.n_nationkey = s2.s_nationkey
    JOIN region r2 ON r2.r_regionkey = n2.n_regionkey
    WHERE ps2.ps_partkey = p.p_partkey AND r2.r_name = 'EUROPE')"#;

pub type Part = (i64, String, i32, String);
pub type Partsupp = (i64, i64, Decimal);
pub type Supplier = (i64, i32, Decimal, String, String, String, String);
pub type Nation = (i32, i32, String);
pub type Region = (i32, String);
pub type Answer = (i64, String, String, i64, String, String, String, String);
pub struct Data {
    pub part: Vec<Part>,
    pub partsupp: Vec<Partsupp>,
    pub supplier: Vec<Supplier>,
    pub nation: Vec<Nation>,
    pub region: Vec<Region>,
}

mini_linq::relation_loaders! {
    relation Part(p_partkey: i64, p_mfgr: String, p_size: i32, p_type: String);
    relation Partsupp(ps_partkey: i64, ps_suppkey: i64, ps_supplycost: Decimal);
    relation Supplier(s_suppkey: i64, s_nationkey: i32, s_acctbal: Decimal,
        s_name: String, s_address: String, s_phone: String, s_comment: String);
    relation Nation(n_nationkey: i32, n_regionkey: i32, n_name: String);
    relation Region(r_regionkey: i32, r_name: String);
}

pub fn load(root: &std::path::Path, config: &str) -> Result<Data, Box<dyn std::error::Error>> {
    assert!(CONFIGS.contains(&config), "unknown H02 configuration");
    Ok(Data {
        part: <relation_loader!(Part)>::parquet(
            root.join("part.parquet"),
            cq_load::ParquetOptions::default(),
        )?,
        partsupp: <relation_loader!(Partsupp)>::parquet(
            root.join("partsupp.parquet"),
            cq_load::ParquetOptions::default(),
        )?,
        supplier: <relation_loader!(Supplier)>::parquet(
            root.join("supplier.parquet"),
            cq_load::ParquetOptions::default(),
        )?,
        nation: <relation_loader!(Nation)>::parquet(
            root.join("nation.parquet"),
            cq_load::ParquetOptions::default(),
        )?,
        region: <relation_loader!(Region)>::parquet(
            root.join("region.parquet"),
            cq_load::ParquetOptions::default(),
        )?,
    })
}

crate::tpch_runtime::standalone_configs! { $;
    pub(super) fn run(data: &Data, config: &str) -> Vec<Answer> {
        assert!(CONFIGS.contains(&config), "unknown H02 configuration");
        let Part = data.part.as_slice();
        let Partsupp = data.partsupp.as_slice();
        let Supplier = data.supplier.as_slice();
        let Nation = data.nation.as_slice();
        let Region = data.region.as_slice();
        let MinimumCost: Vec<(i64, Option<i64>)> = query! {
            plan(no_reduce, order(Region, Nation, Supplier, Partsupp));
            struct Minimum;
            relation Partsupp(ps_partkey: i64, ps_suppkey: i64, ps_supplycost: Decimal);
            relation Supplier(s_suppkey: i64, s_nationkey: i32, s_acctbal: Decimal,
                s_name: String, s_address: String, s_phone: String, s_comment: String);
            relation Nation(n_nationkey: i32, n_regionkey: i32, n_name: String);
            relation Region(r_regionkey: i32, r_name: String);
            relation answer(part_key: i64, minimum_cost: Option<i64>);
            answer(part_key, min(cost_units)) :-
                Partsupp(part_key, supplier_key, cost),
                Supplier(supplier_key, nation_key, _, _, _, _, _),
                Nation(nation_key, region_key, _), Region(region_key, region_name),
                if region_name == "EUROPE",
                let cost_units: i64 = cost.scaled();
        }
        .map(|(part_key, cost)| (*part_key, cost))
        .collect();
        let MinimumCost = MinimumCost.as_slice();
        query! {
            plan(no_reduce, order(Part, Partsupp, MinimumCost, Supplier, Nation, Region));
            struct Query;
            relation Part(p_partkey: i64, p_mfgr: String, p_size: i32, p_type: String);
            relation Partsupp(ps_partkey: i64, ps_suppkey: i64, ps_supplycost: Decimal);
            relation Supplier(s_suppkey: i64, s_nationkey: i32, s_acctbal: Decimal,
                s_name: String, s_address: String, s_phone: String, s_comment: String);
            relation Nation(n_nationkey: i32, n_regionkey: i32, n_name: String);
            relation Region(r_regionkey: i32, r_name: String);
            relation MinimumCost(part_key: i64, minimum_cost: Option<i64>);
            relation answer(balance: i64, supplier_name: String, nation_name: String,
                part_key: i64, manufacturer: String, address: String, phone: String, comment: String);
            answer(*balance_units, supplier_name, nation_name, part_key, manufacturer,
                address, phone, comment) :-
                Part(part_key, manufacturer, size, kind),
                if *size == 15 && kind.ends_with("BRASS"),
                Partsupp(part_key, supplier_key, cost), MinimumCost(part_key, minimum_cost),
                if *minimum_cost == Some(cost.scaled()),
                Supplier(supplier_key, nation_key, balance, supplier_name, address, phone, comment),
                Nation(nation_key, region_key, nation_name), Region(region_key, region_name),
                if region_name == "EUROPE",
                let balance_units: i64 = balance.scaled();
        }
        .map(|(balance, supplier, nation, part, manufacturer, address, phone, comment)| {
            (balance, supplier.clone(), nation.clone(), *part, manufacturer.clone(),
                address.clone(), phone.clone(), comment.clone())
        })
        .collect()
    }
}

pub fn canonical(rows: &[Answer]) -> Vec<Vec<String>> {
    rows.iter()
        .map(
            |(balance, supplier, nation, part, manufacturer, address, phone, comment)| {
                vec![
                    balance.to_string(),
                    supplier.clone(),
                    nation.clone(),
                    part.to_string(),
                    manufacturer.clone(),
                    address.clone(),
                    phone.clone(),
                    comment.clone(),
                ]
            },
        )
        .collect()
}
