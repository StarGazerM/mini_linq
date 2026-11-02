//! TPC-H H19 global discounted revenue with the three standard branches.
//! Scale-two price/discount inputs yield exact scale-four revenue coefficients.
//! The disjunction is one relational `where`, not a bag-producing rule union.
//! Named order/line identities retain equal revenues from different lineitems.
//! The common loader interns four columns independently using sorted dictionaries;
//! query literals at those columns translate through their declared domains, and
//! absent literals use a code no source row can contain.
use cq_load::{Decimal, Dictionaries, StringCode};

pub const NAME: &str = "retail_h19";

pub const EXPORTS: &[(&str, &str)] = &[
    (
        "lineitem",
        "l_orderkey,l_linenumber,l_partkey,l_quantity,l_extendedprice,l_discount,l_shipmode,l_shipinstruct",
    ),
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
pub type Lineitem = (
    i64,
    i64,
    i64,
    Decimal,
    Decimal,
    Decimal,
    StringCode,
    StringCode,
);
pub type Part = (i64, StringCode, StringCode, i32);
pub type Answer = (i64,);
pub struct Data {
    pub lineitem: Vec<Lineitem>,
    pub part: Vec<Part>,
    pub dictionaries: Dictionaries,
}

mini_linq::interned_relation_loaders! {
    relation Lineitem(l_orderkey: i64, l_linenumber: i64, l_partkey: i64,
        l_quantity: Decimal, l_extendedprice: Decimal, l_discount: Decimal,
        l_shipmode: StringCode, l_shipinstruct: StringCode);
    relation Part(p_partkey: i64, p_brand: StringCode, p_container: StringCode, p_size: i32);
}

impl Data {
    /// Prepares exact source strings for equality predicates without changing rows.
    pub fn from_strings(lineitem: Vec<SourceLineitem>, part: Vec<SourcePart>) -> Self {
        let mut dictionaries = Dictionaries::default();
        let lineitem = <relation_loader!(Lineitem)>::intern(lineitem, &mut dictionaries);
        let part = <relation_loader!(Part)>::intern(part, &mut dictionaries);
        Self {
            lineitem,
            part,
            dictionaries,
        }
    }
}

pub fn load(root: &std::path::Path, config: &str) -> Result<Data, Box<dyn std::error::Error>> {
    assert!(CONFIGS.contains(&config), "unknown H19 configuration");
    let mut dictionaries = Dictionaries::default();
    Ok(Data {
        lineitem: <relation_loader!(Lineitem)>::parquet_interned(
            root.join("lineitem.parquet"),
            cq_load::ParquetOptions::default(),
            &mut dictionaries,
        )?,
        part: <relation_loader!(Part)>::parquet_interned(
            root.join("part.parquet"),
            cq_load::ParquetOptions::default(),
            &mut dictionaries,
        )?,
        dictionaries,
    })
}

crate::tpch_runtime::standalone_configs! { $;
    pub(super) fn run(data: &Data, config: &str) -> Vec<Answer> {
        assert!(CONFIGS.contains(&config), "unknown H19 configuration");
        let Lineitem = data.lineitem.as_slice();
        let Part = data.part.as_slice();
        let q1_low = Decimal::from_scaled(100).unwrap();
        let q1_high = Decimal::from_scaled(1100).unwrap();
        let q2_low = Decimal::from_scaled(1000).unwrap();
        let q2_high = Decimal::from_scaled(2000).unwrap();
        let q3_low = Decimal::from_scaled(2000).unwrap();
        let q3_high = Decimal::from_scaled(3000).unwrap();
        query! {
            plan(no_reduce, order(Lineitem, Part));
            struct Query;
            relation Lineitem(l_orderkey: i64, l_linenumber: i64, l_partkey: i64,
                l_quantity: Decimal, l_extendedprice: Decimal, l_discount: Decimal,
                l_shipmode: StringCode in (data.dictionaries.domain("l_shipmode")),
                l_shipinstruct: StringCode in (data.dictionaries.domain("l_shipinstruct")));
            relation Part(p_partkey: i64, p_brand: StringCode in (data.dictionaries.domain("p_brand")),
                p_container: StringCode in (data.dictionaries.domain("p_container")), p_size: i32);
            relation answer(revenue: i64);
            answer(sum(revenue_units)) :-
                Lineitem(order_key, line_number, part_key, quantity, price, discount, ship_mode, ship_instruction),
                where (ship_mode == "AIR" || ship_mode == "AIR REG") && ship_instruction == "DELIVER IN PERSON",
                Part(part_key, brand, container, size),
                where (brand == "Brand#12"
                    && (container == "SM CASE" || container == "SM BOX" || container == "SM PACK" || container == "SM PKG")
                    && quantity >= q1_low && quantity <= q1_high && size >= 1 && size <= 5)
                    || (brand == "Brand#23"
                    && (container == "MED BAG" || container == "MED BOX" || container == "MED PKG" || container == "MED PACK")
                    && quantity >= q2_low && quantity <= q2_high && size >= 1 && size <= 10)
                    || (brand == "Brand#34"
                    && (container == "LG CASE" || container == "LG BOX" || container == "LG PACK" || container == "LG PKG")
                    && quantity >= q3_low && quantity <= q3_high && size >= 1 && size <= 15),
                let revenue_units: i64 = price.scaled() * (100 - discount.scaled());
        }
        .collect()
    }
}

pub fn canonical(rows: &[Answer]) -> Vec<Vec<String>> {
    rows.iter()
        .map(|(revenue,)| vec![revenue.to_string()])
        .collect()
}
