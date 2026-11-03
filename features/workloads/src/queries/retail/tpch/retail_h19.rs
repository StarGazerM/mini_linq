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
type LineitemTable<F> = cq_storage::scan::ScanTable<(
    cq_storage::scan::ScanColumn<i64, F>,
    cq_storage::scan::ScanColumn<i64, F>,
    cq_storage::scan::ScanColumn<i64, cq_storage::scan::Plain>,
    cq_storage::scan::ScanColumn<Decimal, F>,
    cq_storage::scan::ScanColumn<Decimal, cq_storage::scan::Plain>,
    cq_storage::scan::ScanColumn<Decimal, F>,
    cq_storage::scan::ScanColumn<StringCode, F>,
    cq_storage::scan::ScanColumn<StringCode, F>,
)>;
type PartTable<F> = cq_storage::scan::ScanTable<(
    cq_storage::scan::ScanColumn<i64, F>,
    cq_storage::scan::ScanColumn<StringCode, F>,
    cq_storage::scan::ScanColumn<StringCode, F>,
    cq_storage::scan::ScanColumn<i32, F>,
)>;

enum Inputs {
    Rows(Vec<Lineitem>, Vec<Part>),
    Plain(LineitemTable<cq_storage::scan::Plain>, PartTable<cq_storage::scan::Plain>),
    Bytes(LineitemTable<cq_storage::scan::Bytes>, PartTable<cq_storage::scan::Bytes>),
}

impl Inputs {
    fn rows(&self) -> (&[Lineitem], &[Part]) {
        let Self::Rows(lineitem, part) = self else { unreachable!("row route requires row inputs") };
        (lineitem, part)
    }
    fn plain(&self) -> (&LineitemTable<cq_storage::scan::Plain>, &PartTable<cq_storage::scan::Plain>) {
        let Self::Plain(lineitem, part) = self else { unreachable!("Plain route requires Plain inputs") };
        (lineitem, part)
    }
    fn bytes(&self) -> (&LineitemTable<cq_storage::scan::Bytes>, &PartTable<cq_storage::scan::Bytes>) {
        let Self::Bytes(lineitem, part) = self else { unreachable!("Bytes route requires Bytes inputs") };
        (lineitem, part)
    }
}

pub struct Data {
    input: Inputs,
    pub dictionaries: Dictionaries,
}

mini_linq::interned_relation_loaders! {
    relation Lineitem(l_orderkey: i64, l_linenumber: i64, l_partkey: i64,
        l_quantity: Decimal, l_extendedprice: Decimal, l_discount: Decimal,
        l_shipmode: StringCode, l_shipinstruct: StringCode);
    relation Part(p_partkey: i64, p_brand: StringCode, p_container: StringCode, p_size: i32);
}

mini_linq::interned_relation_loaders! {
    columnar;
    relation PlainLineitem(l_orderkey: i64 @ plain, l_linenumber: i64 @ plain,
        l_partkey: i64 @ plain, l_quantity: Decimal @ plain,
        l_extendedprice: Decimal @ plain, l_discount: Decimal @ plain,
        l_shipmode: StringCode @ plain, l_shipinstruct: StringCode @ plain);
    relation BytesLineitem(l_orderkey: i64 @ bytes, l_linenumber: i64 @ bytes,
        l_partkey: i64 @ plain, l_quantity: Decimal @ bytes,
        l_extendedprice: Decimal @ plain, l_discount: Decimal @ bytes,
        l_shipmode: StringCode @ bytes, l_shipinstruct: StringCode @ bytes);
    relation PlainPart(p_partkey: i64 @ plain, p_brand: StringCode @ plain,
        p_container: StringCode @ plain, p_size: i32 @ plain);
    relation BytesPart(p_partkey: i64 @ bytes, p_brand: StringCode @ bytes,
        p_container: StringCode @ bytes, p_size: i32 @ bytes);
}

fn scan_domain(namespace: u64, carrier: cq_storage::scan::Width) -> cq_storage::scan::format::Domain {
    cq_storage::scan::format::Domain {
        id: cq_storage::scan::format::DomainId { namespace, version: 0 },
        carrier, equality: true, order: true,
    }
}
fn lineitem_domains() -> [cq_storage::scan::format::Domain; 8] {
    use cq_storage::scan::Width::{W32, W64};
    [scan_domain(1, W64), scan_domain(1, W64), scan_domain(1, W64),
        scan_domain(3, W64), scan_domain(3, W64), scan_domain(3, W64),
        scan_domain(2, W32), scan_domain(4, W32)]
}
fn part_domains() -> [cq_storage::scan::format::Domain; 4] {
    use cq_storage::scan::Width::{W32, W64};
    [scan_domain(1, W64), scan_domain(6, W32), scan_domain(8, W32), scan_domain(7, W32)]
}

impl Data {
    /// Fixture constructor; production blocked loads publish columns directly.
    pub fn from_strings(lineitem: Vec<SourceLineitem>, part: Vec<SourcePart>, config: &str) -> Self {
        let mut dictionaries = Dictionaries::default();
        let lineitem = LineitemLoader::intern(lineitem, &mut dictionaries);
        let part = PartLoader::intern(part, &mut dictionaries);
        fn column<T: cq_storage::scan::OrdinalDecode, F: cq_storage::scan::Family>(
            values: impl Iterator<Item = T>,
        ) -> cq_storage::scan::ScanColumn<T, F> {
            let mut builder = cq_storage::scan::ScanColumnBuilder::new(
                cq_storage::scan::BlockRows::new(1024).unwrap(), false);
            builder.extend(values);
            builder.finish()
        }
        macro_rules! tables {
            ($variant:ident) => { Inputs::$variant(
                cq_storage::scan::ScanTable::new((
                    column(lineitem.iter().map(|r| r.0)), column(lineitem.iter().map(|r| r.1)),
                    column(lineitem.iter().map(|r| r.2)), column(lineitem.iter().map(|r| r.3)),
                    column(lineitem.iter().map(|r| r.4)), column(lineitem.iter().map(|r| r.5)),
                    column(lineitem.iter().map(|r| r.6)), column(lineitem.iter().map(|r| r.7)),
                )).bind_domains(&lineitem_domains()).unwrap(),
                cq_storage::scan::ScanTable::new((
                    column(part.iter().map(|r| r.0)), column(part.iter().map(|r| r.1)),
                    column(part.iter().map(|r| r.2)), column(part.iter().map(|r| r.3)),
                )).bind_domains(&part_domains()).unwrap(),
            ) };
        }
        let input = match config {
            "blocks-plain" | "blocks-plain-par" => tables!(Plain),
            "blocks-bytes" | "blocks-bytes-par" => tables!(Bytes),
            _ => Inputs::Rows(lineitem, part),
        };
        Self { input, dictionaries }
    }
}

pub fn load(root: &std::path::Path, config: &str) -> Result<Data, Box<dyn std::error::Error>> {
    assert!(CONFIGS.contains(&config), "unknown H19 configuration");
    let mut dictionaries = Dictionaries::default();
    let blocks = cq_storage::scan::BlockRows::new(1024).unwrap();
    let input = match config {
        "blocks-plain" | "blocks-plain-par" => Inputs::Plain(
            PlainLineitemLoader::parquet_interned(root.join("lineitem.parquet"), Default::default(), blocks, false, &mut dictionaries)?
                .bind_domains(&lineitem_domains())?,
            PlainPartLoader::parquet_interned(root.join("part.parquet"), Default::default(), blocks, false, &mut dictionaries)?
                .bind_domains(&part_domains())?,
        ),
        "blocks-bytes" | "blocks-bytes-par" => Inputs::Bytes(
            BytesLineitemLoader::parquet_interned(root.join("lineitem.parquet"), Default::default(), blocks, false, &mut dictionaries)?
                .bind_domains(&lineitem_domains())?,
            BytesPartLoader::parquet_interned(root.join("part.parquet"), Default::default(), blocks, false, &mut dictionaries)?
                .bind_domains(&part_domains())?,
        ),
        _ => Inputs::Rows(
            LineitemLoader::parquet_interned(root.join("lineitem.parquet"), Default::default(), &mut dictionaries)?,
            PartLoader::parquet_interned(root.join("part.parquet"), Default::default(), &mut dictionaries)?,
        ),
    };
    Ok(Data { input, dictionaries })
}

macro_rules! query_body {
    ($input:ident; [$($integer:tt)*], [$($decimal:tt)*], [$($code:tt)*], [$($size:tt)*];
        [$($mode:tt)*], [$($instruction:tt)*], [$($brand:tt)*], [$($container:tt)*]) => {
    pub(super) fn run(data: &Data, config: &str) -> Vec<Answer> {
        assert!(CONFIGS.contains(&config), "unknown H19 configuration");
        let (Lineitem, Part) = data.input.$input();
        let q1_low = Decimal::from_scaled(100).unwrap();
        let q1_high = Decimal::from_scaled(1100).unwrap();
        let q2_low = Decimal::from_scaled(1000).unwrap();
        let q2_high = Decimal::from_scaled(2000).unwrap();
        let q3_low = Decimal::from_scaled(2000).unwrap();
        let q3_high = Decimal::from_scaled(3000).unwrap();
        query! {
            plan(no_reduce, order(Lineitem, Part));
            struct Query;
            relation Lineitem(l_orderkey: i64 $($integer)*, l_linenumber: i64 $($integer)*, l_partkey: i64 $($integer)*,
                l_quantity: Decimal $($decimal)*, l_extendedprice: Decimal $($decimal)*, l_discount: Decimal $($decimal)*,
                l_shipmode: StringCode $($code)* in $($mode)* (data.dictionaries.domain("l_shipmode")),
                l_shipinstruct: StringCode $($code)* in $($instruction)* (data.dictionaries.domain("l_shipinstruct")));
            relation Part(p_partkey: i64 $($integer)*, p_brand: StringCode $($code)* in $($brand)* (data.dictionaries.domain("p_brand")),
                p_container: StringCode $($code)* in $($container)* (data.dictionaries.domain("p_container")), p_size: i32 $($size)*);
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
    };
}

mod row_configs {
    use super::*;
    crate::tpch_runtime::standalone_configs! { $;
        query_body!(rows; [], [], [], []; [], [], [], []);
    }
}

pub const CONFIGS: &[&str] = crate::tpch_runtime::cpu_routes::cpu_route_names!(
    "advised", "row-join-par", "blocks-plain", "blocks-bytes", "blocks-plain-par", "blocks-bytes-par");

crate::tpch_runtime::standalone_configuration!(row_join_par, $, {
    plan = ::mini_linq::passes::plan(no_reduce, order(Lineitem, Part));
    decompose = [::cq_sequence_tasks_macros::plan];
    grade = ::cq_sequence_tasks_macros::tasks;
    print = ::cq_sequence_tasks_macros::translate_tasks;
}, [], { query_body!(rows; [], [], [], []; [], [], [], []); });

macro_rules! blocked_configuration {
    ($module:ident, $d:tt, $family:ident, $input:ident, [$($parallel:tt)*]) => {
        crate::tpch_runtime::standalone_configuration!($module, $d, {
            plan = ::mini_linq::passes::plan(no_reduce, order(Lineitem, Part));
            select = ::mini_linq::passes::select(blocks(
                Lineitem { block_rows 1024; summary none;
                    columns [l_orderkey: $family u64, l_linenumber: $family u64,
                        l_partkey: plain u64, l_quantity: $family u64,
                        l_extendedprice: plain u64, l_discount: $family u64,
                        l_shipmode: $family u32, l_shipinstruct: $family u32]; },
                Part { block_rows 1024; summary none;
                    columns [p_partkey: $family u64, p_brand: $family u32,
                        p_container: $family u32, p_size: $family u32]; }
            ));
            $($parallel)*
        }, [], {
            query_body!($input; [[0]], [[1]], [[2]], [[3]]; [[0]], [[1]], [[2]], [[3]]);
        });
    };
}
blocked_configuration!(blocks_plain, $, plain, plain, []);
blocked_configuration!(blocks_bytes, $, bytes, bytes, []);
blocked_configuration!(blocks_plain_par, $, plain, plain, [
    decompose = [::cq_sequence_tasks_macros::plan];
    grade = ::cq_sequence_tasks_macros::tasks;
    print = ::cq_sequence_tasks_macros::translate_tasks;
]);
blocked_configuration!(blocks_bytes_par, $, bytes, bytes, [
    decompose = [::cq_sequence_tasks_macros::plan];
    grade = ::cq_sequence_tasks_macros::tasks;
    print = ::cq_sequence_tasks_macros::translate_tasks;
]);

pub fn run(data: &Data, config: &str) -> Vec<Answer> {
    match config {
        "row-join-par" => row_join_par::run(data, config),
        "blocks-plain" => blocks_plain::run(data, config),
        "blocks-bytes" => blocks_bytes::run(data, config),
        "blocks-plain-par" => blocks_plain_par::run(data, config),
        "blocks-bytes-par" => blocks_bytes_par::run(data, config),
        _ => row_configs::run(data, config),
    }
}

pub fn canonical(rows: &[Answer]) -> Vec<Vec<String>> {
    rows.iter()
        .map(|(revenue,)| vec![revenue.to_string()])
        .collect()
}
