//! TPC-H H19 global discounted revenue with the three standard branches.
//! Scale-two price/discount inputs yield exact scale-four revenue coefficients.
//! The disjunction remains a pinned Rust guard, not a bag-producing rule union.
//! Named order/line identities retain equal revenues from different lineitems.
use cq_load::Decimal;

pub const NAME: &str = "retail_h19";
pub const CONFIGS: &[&str] = &["default"];
pub const EXPORTS: &[(&str, &str)] = &[
    ("lineitem", "l_orderkey,l_linenumber,l_partkey,l_quantity,l_extendedprice,l_discount,l_shipmode,l_shipinstruct"),
    ("part", "p_partkey,p_brand,p_container,p_size"),
];
pub const SQL: &str = r#"SELECT
    CAST(COALESCE(SUM(l_extendedprice * (1.00 - l_discount)), 0) * 10000 AS BIGINT) AS revenue
FROM (SELECT DISTINCT l.l_orderkey, l.l_linenumber, l.l_partkey, l.l_quantity,
    l.l_extendedprice, l.l_discount, l.l_shipmode, l.l_shipinstruct,
    p.p_brand, p.p_container, p.p_size
    FROM lineitem l JOIN part p ON l.l_partkey = p.p_partkey
    WHERE l.l_shipmode IN ('AIR', 'AIR REG') AND l.l_shipinstruct = 'DELIVER IN PERSON'
    AND ((p.p_brand = 'Brand#12' AND p.p_container IN ('SM CASE', 'SM BOX', 'SM PACK', 'SM PKG')
            AND l.l_quantity BETWEEN 1.00 AND 11.00 AND p.p_size BETWEEN 1 AND 5)
        OR (p.p_brand = 'Brand#23' AND p.p_container IN ('MED BAG', 'MED BOX', 'MED PKG', 'MED PACK')
            AND l.l_quantity BETWEEN 10.00 AND 20.00 AND p.p_size BETWEEN 1 AND 10)
        OR (p.p_brand = 'Brand#34' AND p.p_container IN ('LG CASE', 'LG BOX', 'LG PACK', 'LG PKG')
            AND l.l_quantity BETWEEN 20.00 AND 30.00 AND p.p_size BETWEEN 1 AND 15))) AS matches"#;

pub type Lineitem = (i64, i64, i64, Decimal, Decimal, Decimal, String, String);
pub type Part = (i64, String, String, i32);
pub type Answer = (i64,);
pub struct Data {
    pub lineitem: Vec<Lineitem>,
    pub part: Vec<Part>,
}

mini_linq::relation_loaders! {
    relation Lineitem(l_orderkey: i64, l_linenumber: i64, l_partkey: i64,
        l_quantity: Decimal, l_extendedprice: Decimal, l_discount: Decimal,
        l_shipmode: String, l_shipinstruct: String);
    relation Part(p_partkey: i64, p_brand: String, p_container: String, p_size: i32);
}

pub fn load(root: &std::path::Path, config: &str) -> Result<Data, Box<dyn std::error::Error>> {
    assert_eq!(config, "default", "unknown H19 configuration");
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
    assert_eq!(config, "default", "unknown H19 configuration");
    let Lineitem = data.lineitem.as_slice();
    let Part = data.part.as_slice();
    mini_linq::compile! {
        pipeline {}
        struct Query;
        relation Lineitem(l_orderkey: i64, l_linenumber: i64, l_partkey: i64,
            l_quantity: Decimal, l_extendedprice: Decimal, l_discount: Decimal,
            l_shipmode: String, l_shipinstruct: String);
        relation Part(p_partkey: i64, p_brand: String, p_container: String, p_size: i32);
        relation answer(revenue: i64);
        answer(sum(revenue_units)) :-
            Lineitem(order_key, line_number, part_key, quantity, price, discount, ship_mode, ship_instruction),
            if (ship_mode == "AIR" || ship_mode == "AIR REG") && ship_instruction == "DELIVER IN PERSON",
            Part(part_key, brand, container, size),
            if (brand == "Brand#12"
                && matches!(container.as_str(), "SM CASE" | "SM BOX" | "SM PACK" | "SM PKG")
                && quantity.scaled() >= 100 && quantity.scaled() <= 1100 && *size >= 1 && *size <= 5)
                || (brand == "Brand#23"
                && matches!(container.as_str(), "MED BAG" | "MED BOX" | "MED PKG" | "MED PACK")
                && quantity.scaled() >= 1000 && quantity.scaled() <= 2000 && *size >= 1 && *size <= 10)
                || (brand == "Brand#34"
                && matches!(container.as_str(), "LG CASE" | "LG BOX" | "LG PACK" | "LG PKG")
                && quantity.scaled() >= 2000 && quantity.scaled() <= 3000 && *size >= 1 && *size <= 15),
            let revenue_units: i64 = price.scaled() * (100 - discount.scaled());
    }
    .collect()
}

pub fn canonical(rows: &[Answer]) -> Vec<Vec<String>> {
    rows.iter().map(|(revenue,)| vec![revenue.to_string()]).collect()
}
