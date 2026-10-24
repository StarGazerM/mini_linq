//! TPC-H Q20 Canadian suppliers with excess stock of forest-prefixed parts.
//! A single keyed shipment sum feeds a non-aggregate join and the original
//! availability predicate. Missing shipment groups do not qualify (SQL NULL).
//! Quantities are scale-two coefficients; final ordering is omitted.
use cq_load::{Date, Decimal, Load};

pub const NAME: &str = "retail_h20";
pub const CONFIGS: &[&str] = &["default"];
pub const EXPORTS: &[(&str, &str)] = &[
    ("part", "p_partkey,p_name"),
    ("partsupp", "ps_partkey,ps_suppkey,ps_availqty"),
    ("lineitem", "l_orderkey,l_linenumber,l_partkey,l_suppkey,l_quantity,l_shipdate"),
    ("supplier", "s_suppkey,s_nationkey,s_name,s_address"),
    ("nation", "n_nationkey,n_name"),
];
pub const SQL: &str = r#"SELECT DISTINCT s.s_name, s.s_address
FROM supplier s JOIN nation n ON n.n_nationkey = s.s_nationkey
WHERE n.n_name = 'CANADA' AND s.s_suppkey IN (
    SELECT ps.ps_suppkey FROM partsupp ps
    WHERE ps.ps_partkey IN (SELECT p.p_partkey FROM part p WHERE p.p_name LIKE 'forest%')
    AND ps.ps_availqty > 0.5 * (
        SELECT SUM(l.l_quantity) FROM (
            SELECT DISTINCT l_orderkey, l_linenumber, l_partkey, l_suppkey, l_quantity, l_shipdate
            FROM lineitem WHERE l_shipdate >= DATE '1994-01-01'
                AND l_shipdate < DATE '1995-01-01') l
        WHERE l.l_partkey = ps.ps_partkey AND l.l_suppkey = ps.ps_suppkey))"#;

pub type Part = (i64, String);
pub type Partsupp = (i64, i64, i64);
pub type Lineitem = (i64, i64, i64, i64, Decimal, Date);
pub type Supplier = (i64, i32, String, String);
pub type Nation = (i32, String);
pub type Answer = (String, String);
pub struct Data {
    pub part: Vec<Part>,
    pub partsupp: Vec<Partsupp>,
    pub lineitem: Vec<Lineitem>,
    pub supplier: Vec<Supplier>,
    pub nation: Vec<Nation>,
}

mini_linq::relation_loaders! {
    relation Part(p_partkey: i64, p_name: String);
    relation Partsupp(ps_partkey: i64, ps_suppkey: i64, ps_availqty: i64);
    relation Lineitem(l_orderkey: i64, l_linenumber: i64, l_partkey: i64,
        l_suppkey: i64, l_quantity: Decimal, l_shipdate: Date);
    relation Supplier(s_suppkey: i64, s_nationkey: i32, s_name: String, s_address: String);
    relation Nation(n_nationkey: i32, n_name: String);
}

pub fn load(root: &std::path::Path, config: &str) -> Result<Data, Box<dyn std::error::Error>> {
    assert_eq!(config, "default", "unknown H20 configuration");
    Ok(Data {
        part: <relation_loader!(Part)>::parquet(
            root.join("part.parquet"), cq_load::ParquetOptions::default(),
        )?,
        partsupp: <relation_loader!(Partsupp)>::parquet(
            root.join("partsupp.parquet"), cq_load::ParquetOptions::default(),
        )?,
        lineitem: <relation_loader!(Lineitem)>::parquet(
            root.join("lineitem.parquet"), cq_load::ParquetOptions::default(),
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
    assert_eq!(config, "default", "unknown H20 configuration");
    let Part = data.part.as_slice();
    let Partsupp = data.partsupp.as_slice();
    let Lineitem = data.lineitem.as_slice();
    let Supplier = data.supplier.as_slice();
    let Nation = data.nation.as_slice();
    let start = Date::load("1994-01-01").expect("authored ISO date");
    let end = Date::load("1995-01-01").expect("authored ISO date");
    let Shipped: Vec<(i64, i64, i64)> = mini_linq::compile! {
        pipeline {}
        struct Shipments;
        relation Lineitem(l_orderkey: i64, l_linenumber: i64, l_partkey: i64,
            l_suppkey: i64, l_quantity: Decimal, l_shipdate: Date);
        relation answer(part_key: i64, supplier_key: i64, quantity_units: i64);
        answer(part_key, supplier_key, sum(quantity_units)) :-
            Lineitem(order_key, line_number, part_key, supplier_key, quantity, shipdate),
            if *shipdate >= start && *shipdate < end,
            let quantity_units: i64 = quantity.scaled();
    }.map(|(part, supplier, quantity)| (*part, *supplier, quantity)).collect();
    let Shipped = Shipped.as_slice();
    mini_linq::compile! {
        pipeline {}
        struct Query;
        relation Part(p_partkey: i64, p_name: String);
        relation Partsupp(ps_partkey: i64, ps_suppkey: i64, ps_availqty: i64);
        relation Shipped(part_key: i64, supplier_key: i64, quantity_units: i64);
        relation Supplier(s_suppkey: i64, s_nationkey: i32, s_name: String, s_address: String);
        relation Nation(n_nationkey: i32, n_name: String);
        relation answer(supplier_name: String, address: String);
        answer(supplier_name, address) :-
            Part(part_key, part_name), if part_name.starts_with("forest"),
            Partsupp(part_key, supplier_key, available), Shipped(part_key, supplier_key, quantity_units),
            if i128::from(*available) * 200 > i128::from(*quantity_units),
            Supplier(supplier_key, nation_key, supplier_name, address),
            Nation(nation_key, nation_name), if nation_name == "CANADA";
    }.map(|(name, address)| (name.clone(), address.clone())).collect()
}

pub fn canonical(rows: &[Answer]) -> Vec<Vec<String>> {
    rows.iter().map(|(name, address)| vec![name.clone(), address.clone()]).collect()
}
