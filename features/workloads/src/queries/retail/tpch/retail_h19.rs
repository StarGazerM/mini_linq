//! TPC-H H19 global discounted revenue with the three standard branches.
//! Scale-two price/discount inputs yield exact scale-four revenue coefficients.
//! The disjunction remains a pinned Rust guard, not a bag-producing rule union.
//! Named order/line identities retain equal revenues from different lineitems.
//! Caller preparation interns four columns independently using sorted-distinct
//! dictionaries; absent query literals use a code no source row can contain.
use cq_load::{Decimal, StringDictionary};

pub const NAME: &str = "retail_h19";
pub const CONFIGS: &[&str] = &["default", "radix", "radix_retained"];
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

pub type SourceLineitem = (i64, i64, i64, Decimal, Decimal, Decimal, String, String);
pub type SourcePart = (i64, String, String, i32);
pub type Lineitem = (i64, i64, i64, Decimal, Decimal, Decimal, u32, u32);
pub type Part = (i64, u32, u32, i32);
pub type Answer = (i64,);
pub struct Data {
    pub lineitem: Vec<Lineitem>,
    pub part: Vec<Part>,
    shipmode: StringDictionary,
    shipinstruct: StringDictionary,
    brand: StringDictionary,
    container: StringDictionary,
}

impl Data {
    /// Prepares exact source strings for equality predicates without changing rows.
    pub fn from_strings(lineitem: Vec<SourceLineitem>, part: Vec<SourcePart>) -> Self {
        let shipmode = StringDictionary::new(lineitem.iter().map(|row| row.6.as_str()));
        let shipinstruct = StringDictionary::new(lineitem.iter().map(|row| row.7.as_str()));
        let brand = StringDictionary::new(part.iter().map(|row| row.1.as_str()));
        let container = StringDictionary::new(part.iter().map(|row| row.2.as_str()));
        let lineitem = lineitem.into_iter().map(|(order, line, part, quantity, price, discount, mode, instruction)| {
            (order, line, part, quantity, price, discount,
                shipmode.lookup(&mode).expect("dictionary contains source value"),
                shipinstruct.lookup(&instruction).expect("dictionary contains source value"))
        }).collect();
        let part = part.into_iter().map(|(key, value, packaging, size)| {
            (key, brand.lookup(&value).expect("dictionary contains source value"),
                container.lookup(&packaging).expect("dictionary contains source value"), size)
        }).collect();
        Self { lineitem, part, shipmode, shipinstruct, brand, container }
    }
}

mini_linq::relation_loaders! {
    relation Lineitem(l_orderkey: i64, l_linenumber: i64, l_partkey: i64,
        l_quantity: Decimal, l_extendedprice: Decimal, l_discount: Decimal,
        l_shipmode: String, l_shipinstruct: String);
    relation Part(p_partkey: i64, p_brand: String, p_container: String, p_size: i32);
}

pub fn load(root: &std::path::Path, config: &str) -> Result<Data, Box<dyn std::error::Error>> {
    assert!(CONFIGS.contains(&config), "unknown H19 configuration");
    Ok(Data::from_strings(
        <relation_loader!(Lineitem)>::parquet(
            root.join("lineitem.parquet"), cq_load::ParquetOptions::default(),
        )?,
        <relation_loader!(Part)>::parquet(
            root.join("part.parquet"), cq_load::ParquetOptions::default(),
        )?,
    ))
}

crate::tpch_runtime::same_plan_kernel! { $;
#[allow(non_snake_case)]
pub(super) fn execute(data: &Data) -> Vec<Answer> {
    let Lineitem = data.lineitem.as_slice();
    let Part = data.part.as_slice();
    let intern = |table: &str, column: &str, value: &str| -> u32 {
        let dictionary = match (table, column) {
            ("lineitem", "l_shipmode") => &data.shipmode,
            ("lineitem", "l_shipinstruct") => &data.shipinstruct,
            ("part", "p_brand") => &data.brand,
            ("part", "p_container") => &data.container,
            _ => panic!("unknown H19 interned column: {table}.{column}"),
        };
        dictionary.lookup(value).unwrap_or(u32::MAX)
    };
    let air: u32 = intern("lineitem", "l_shipmode", "AIR");
    let air_reg: u32 = intern("lineitem", "l_shipmode", "AIR REG");
    let deliver_in_person: u32 = intern("lineitem", "l_shipinstruct", "DELIVER IN PERSON");
    let brand12: u32 = intern("part", "p_brand", "Brand#12");
    let brand23: u32 = intern("part", "p_brand", "Brand#23");
    let brand34: u32 = intern("part", "p_brand", "Brand#34");
    let sm_case: u32 = intern("part", "p_container", "SM CASE");
    let sm_box: u32 = intern("part", "p_container", "SM BOX");
    let sm_pack: u32 = intern("part", "p_container", "SM PACK");
    let sm_pkg: u32 = intern("part", "p_container", "SM PKG");
    let med_bag: u32 = intern("part", "p_container", "MED BAG");
    let med_box: u32 = intern("part", "p_container", "MED BOX");
    let med_pkg: u32 = intern("part", "p_container", "MED PKG");
    let med_pack: u32 = intern("part", "p_container", "MED PACK");
    let lg_case: u32 = intern("part", "p_container", "LG CASE");
    let lg_box: u32 = intern("part", "p_container", "LG BOX");
    let lg_pack: u32 = intern("part", "p_container", "LG PACK");
    let lg_pkg: u32 = intern("part", "p_container", "LG PKG");
    query! {
        pipeline {}
        struct Query;
        relation Lineitem(l_orderkey: i64, l_linenumber: i64, l_partkey: i64,
            l_quantity: Decimal, l_extendedprice: Decimal, l_discount: Decimal,
            l_shipmode: u32, l_shipinstruct: u32);
        relation Part(p_partkey: i64, p_brand: u32, p_container: u32, p_size: i32);
        relation answer(revenue: i64);
        answer(sum(revenue_units)) :-
            Lineitem(order_key, line_number, part_key, quantity, price, discount, ship_mode, ship_instruction),
            if (*ship_mode == air || *ship_mode == air_reg) && *ship_instruction == deliver_in_person,
            Part(part_key, brand, container, size),
            if (*brand == brand12
                && (*container == sm_case || *container == sm_box || *container == sm_pack || *container == sm_pkg)
                && quantity.scaled() >= 100 && quantity.scaled() <= 1100 && *size >= 1 && *size <= 5)
                || (*brand == brand23
                && (*container == med_bag || *container == med_box || *container == med_pkg || *container == med_pack)
                && quantity.scaled() >= 1000 && quantity.scaled() <= 2000 && *size >= 1 && *size <= 10)
                || (*brand == brand34
                && (*container == lg_case || *container == lg_box || *container == lg_pack || *container == lg_pkg)
                && quantity.scaled() >= 2000 && quantity.scaled() <= 3000 && *size >= 1 && *size <= 15),
            let revenue_units: i64 = price.scaled() * (100 - discount.scaled());
    }
    .collect()
}
}

pub fn canonical(rows: &[Answer]) -> Vec<Vec<String>> {
    rows.iter().map(|(revenue,)| vec![revenue.to_string()]).collect()
}
