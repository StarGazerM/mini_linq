//! TPC-H H14 promotional and total revenue components for September 1995.
//! The final percentage ratio is omitted. Scale-two price/discount inputs
//! produce exact scale-four revenue coefficients, without per-line rounding.
//! Order/line identity survives the distinct valuation before the global fold.
use cq_load::{Date, Decimal, Load};

pub const NAME: &str = "retail_h14";
pub const CONFIGS: &[&str] = &["default"];
pub const EXPORTS: &[(&str, &str)] = &[
    ("lineitem", "l_orderkey,l_linenumber,l_partkey,l_extendedprice,l_discount,l_shipdate"),
    ("part", "p_partkey,p_type"),
];
pub const SQL: &str = r#"SELECT
    CAST(COALESCE(SUM(CASE WHEN p_type LIKE 'PROMO%' THEN
        l_extendedprice * (1.00 - l_discount) ELSE 0 END), 0) * 10000 AS BIGINT) AS promo_revenue,
    CAST(COALESCE(SUM(l_extendedprice * (1.00 - l_discount)), 0) * 10000 AS BIGINT) AS total_revenue
FROM (SELECT DISTINCT l.l_orderkey, l.l_linenumber, l.l_partkey,
    l.l_extendedprice, l.l_discount, l.l_shipdate, p.p_type
    FROM lineitem l JOIN part p ON l.l_partkey = p.p_partkey
    WHERE l.l_shipdate >= DATE '1995-09-01' AND l.l_shipdate < DATE '1995-10-01') AS matches"#;

pub type Lineitem = (i64, i64, i64, Decimal, Decimal, Date);
pub type Part = (i64, String);
pub type Answer = (i64, i64);
pub struct Data {
    pub lineitem: Vec<Lineitem>,
    pub part: Vec<Part>,
}

mini_linq::relation_loaders! {
    relation Lineitem(l_orderkey: i64, l_linenumber: i64, l_partkey: i64,
        l_extendedprice: Decimal, l_discount: Decimal, l_shipdate: Date);
    relation Part(p_partkey: i64, p_type: String);
}

pub fn load(root: &std::path::Path, config: &str) -> Result<Data, Box<dyn std::error::Error>> {
    assert_eq!(config, "default", "unknown H14 configuration");
    Ok(Data {
        lineitem: <relation_loader!(Lineitem)>::parquet(
            root.join("lineitem.parquet"), cq_load::ParquetOptions::default(),
        )?,
        part: <relation_loader!(Part)>::parquet(
            root.join("part.parquet"), cq_load::ParquetOptions::default(),
        )?,
    })
}

#[allow(non_snake_case)]
pub fn run(data: &Data, config: &str) -> Vec<Answer> {
    assert_eq!(config, "default", "unknown H14 configuration");
    let Lineitem = data.lineitem.as_slice();
    let Part = data.part.as_slice();
    let start = Date::load("1995-09-01").expect("authored ISO date");
    let end = Date::load("1995-10-01").expect("authored ISO date");
    mini_linq::compile! {
        pipeline {}
        struct Query;
        relation Lineitem(l_orderkey: i64, l_linenumber: i64, l_partkey: i64,
            l_extendedprice: Decimal, l_discount: Decimal, l_shipdate: Date);
        relation Part(p_partkey: i64, p_type: String);
        relation answer(promo_revenue: i64, total_revenue: i64);
        answer(sum(promo_units), sum(revenue_units)) :-
            Lineitem(order_key, line_number, part_key, price, discount, ship_date),
            if *ship_date >= start && *ship_date < end,
            Part(part_key, part_type),
            let revenue_units: i64 = price.scaled() * (100 - discount.scaled()),
            let promo_units: i64 = if part_type.starts_with("PROMO") { *revenue_units } else { 0 };
    }
    .collect()
}

pub fn canonical(rows: &[Answer]) -> Vec<Vec<String>> {
    rows.iter()
        .map(|(promo, total)| vec![promo.to_string(), total.to_string()])
        .collect()
}
