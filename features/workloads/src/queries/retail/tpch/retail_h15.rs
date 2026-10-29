//! TPC-H Q15's maximum-revenue suppliers, without final ordering.
//! Per-supplier revenue feeds a global maximum; equality retains every tie.
//! Revenue is an exact scale-four coefficient, with line identities preserved.
//! Supplier text stays dictionary-coded until final answer publication.
use cq_load::{Date, Decimal, Dictionaries, Load, StringCode};

pub const NAME: &str = "retail_h15";

pub const EXPORTS: &[(&str, &str)] = &[
    ("supplier", "s_suppkey,s_name,s_address,s_phone"),
    (
        "lineitem",
        "l_orderkey,l_linenumber,l_suppkey,l_extendedprice,l_discount,l_shipdate",
    ),
];
pub const SQL: &str = r#"WITH revenue AS (
    SELECT l_suppkey AS supplier_key,
        CAST(SUM(l_extendedprice * (1 - l_discount)) * 10000 AS BIGINT) AS total_revenue
    FROM (SELECT DISTINCT l_orderkey, l_linenumber, l_suppkey,
        l_extendedprice, l_discount, l_shipdate FROM lineitem
        WHERE l_shipdate >= DATE '1996-01-01' AND l_shipdate < DATE '1996-04-01') matches
    GROUP BY l_suppkey)
SELECT DISTINCT s.s_suppkey, s.s_name, s.s_address, s.s_phone, r.total_revenue
FROM supplier s JOIN revenue r ON s.s_suppkey = r.supplier_key
WHERE r.total_revenue = (SELECT MAX(total_revenue) FROM revenue)"#;

pub type Supplier = (i64, StringCode, StringCode, StringCode);
pub type Lineitem = (i64, i64, i64, Decimal, Decimal, Date);
pub type Answer = (i64, String, String, String, i64);
pub struct Data {
    pub supplier: Vec<Supplier>,
    pub lineitem: Vec<Lineitem>,
    pub dictionaries: Dictionaries,
}

mini_linq::interned_relation_loaders! {
    relation Supplier(s_suppkey: i64, s_name: StringCode, s_address: StringCode, s_phone: StringCode);
    relation Lineitem(l_orderkey: i64, l_linenumber: i64, l_suppkey: i64,
        l_extendedprice: Decimal, l_discount: Decimal, l_shipdate: Date);
}

pub fn load(root: &std::path::Path, config: &str) -> Result<Data, Box<dyn std::error::Error>> {
    assert!(CONFIGS.contains(&config), "unknown H15 configuration");
    let mut dictionaries = Dictionaries::default();
    Ok(Data {
        supplier: <relation_loader!(Supplier)>::parquet_interned(
            root.join("supplier.parquet"),
            cq_load::ParquetOptions::default(),
            &mut dictionaries,
        )?,
        lineitem: <relation_loader!(Lineitem)>::parquet_interned(
            root.join("lineitem.parquet"),
            cq_load::ParquetOptions::default(),
            &mut dictionaries,
        )?,
        dictionaries,
    })
}

crate::tpch_runtime::standalone_configs! { $;
    pub(super) fn run(data: &Data, config: &str) -> Vec<Answer> {
        assert!(CONFIGS.contains(&config), "unknown H15 configuration");
        let Supplier = data.supplier.as_slice();
        let Lineitem = data.lineitem.as_slice();
        let start = Date::load("1996-01-01").expect("authored ISO date");
        let end = Date::load("1996-04-01").expect("authored ISO date");
        let Revenue: Vec<(i64, i64)> = query! {
            plan(no_reduce, order(Lineitem));
            struct RevenueBySupplier;
            relation Lineitem(l_orderkey: i64, l_linenumber: i64, l_suppkey: i64,
                l_extendedprice: Decimal, l_discount: Decimal, l_shipdate: Date);
            relation answer(supplier_key: i64, revenue: i64);
            answer(supplier_key, sum(revenue_units)) :-
                Lineitem(order_key, line_number, supplier_key, price, discount, shipdate),
                where shipdate >= start && shipdate < end,
                let revenue_units: i64 = price.scaled() * (100 - discount.scaled());
        }.map(|(supplier, revenue)| (*supplier, revenue)).collect();
        let Revenue = Revenue.as_slice();
        let Maximum: Vec<(Option<i64>,)> = query! {
            plan(no_reduce, order(Revenue));
            struct MaximumRevenue;
            relation Revenue(supplier_key: i64, revenue: i64);
            relation answer(maximum: Option<i64>);
            answer(max(revenue)) :- Revenue(supplier_key, revenue);
        }
        .collect();
        let Maximum = Maximum.as_slice();
        query! {
            plan(no_reduce, order(Maximum, Revenue, Supplier));
            struct Query;
            relation Supplier(s_suppkey: i64, s_name: StringCode, s_address: StringCode, s_phone: StringCode);
            relation Revenue(supplier_key: i64, revenue: i64);
            relation Maximum(maximum: Option<i64>);
            relation answer(supplier_key: i64, name: StringCode, address: StringCode, phone: StringCode, revenue: i64);
            answer(supplier_key, name, address, phone, revenue) :-
                Revenue(supplier_key, revenue), Maximum(maximum),
                if Some(*revenue) == *maximum,
                Supplier(supplier_key, name, address, phone);
        }.map(|(supplier, name, address, phone, revenue)| {
            // DECODE P: Supplier text is materialized only when publishing answers.
            (*supplier, data.dictionaries.publish("s_name", *name),
                data.dictionaries.publish("s_address", *address),
                data.dictionaries.publish("s_phone", *phone), *revenue)
        }).collect()
    }
}

pub fn canonical(rows: &[Answer]) -> Vec<Vec<String>> {
    rows.iter()
        .map(|(supplier, name, address, phone, revenue)| {
            vec![
                supplier.to_string(),
                name.clone(),
                address.clone(),
                phone.clone(),
                revenue.to_string(),
            ]
        })
        .collect()
}
