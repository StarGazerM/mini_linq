//! TPC-H Q22 customer-without-orders core with the seven phone country codes.
//! Omits the scalar average-balance subquery, grouping, sums, and ordering.
//! Phone-prefix membership is prepared by dictionary code at load time.
use cq_load::StringCode;

pub const NAME: &str = "retail_h22";
pub const SQL: &str = r#"SELECT DISTINCT c.c_custkey FROM customer c
WHERE substring(c.c_phone FROM 1 FOR 2) IN ('13', '31', '23', '29', '30', '18', '17')
AND NOT EXISTS (SELECT 1 FROM orders o WHERE o.o_custkey = c.c_custkey)"#;
pub const EXPORTS: &[(&str, &str)] = &[
    ("customer", "c_custkey,c_phone"),
    ("orders", "o_orderkey,o_custkey"),
];
pub const CONFIGS: &[&str] = &[
    "default", "par", "no-reduce", "transfer", "le", "le_runs", "hypergraph", "advised",
];
pub type Answer = (i64,);
pub struct Data {
    pub customer: Vec<(i64, StringCode)>,
    pub orders: Vec<(i64, i64)>,
    pub qualifying_phones: Vec<(StringCode,)>,
    pub dictionaries: cq_load::Dictionaries,
}

mini_linq::interned_relation_loaders! {
    relation Customer(c_custkey: i64, c_phone: StringCode);
    relation Orders(o_orderkey: i64, o_custkey: i64);
}

pub fn load(root: &std::path::Path, _config: &str) -> Result<Data, Box<dyn std::error::Error>> {
    let mut dictionaries = cq_load::Dictionaries::default();
    Ok(Data {
        customer: <relation_loader!(Customer)>::parquet_interned(
            root.join("customer.parquet"), cq_load::ParquetOptions::default(), &mut dictionaries)?,
        orders: <relation_loader!(Orders)>::parquet_interned(
            root.join("orders.parquet"), cq_load::ParquetOptions::default(), &mut dictionaries)?,
        qualifying_phones: cq_load::dictionary_where(
            &dictionaries, "c_phone", |value: &str|
                ["13", "31", "23", "29", "30", "18", "17"].iter()
                    .any(|code| value.starts_with(*code))),
        dictionaries,
    })
}

macro_rules! query_body {
    () => {
        #[allow(non_snake_case)]
        pub(super) fn run(data: &Data) -> Vec<Answer> {
            let Customer = data.customer.as_slice();
            let Orders = data.orders.as_slice();
            let QualifyingPhones = data.qualifying_phones.as_slice();
            query! {
                struct Query;
                relation Customer(c_custkey: i64, c_phone: StringCode);
                relation Orders(o_orderkey: i64, o_custkey: i64);
                relation QualifyingPhones(phone: StringCode);
                relation answer(custkey: i64);
                answer(custkey) :- Customer(custkey, phone), QualifyingPhones(phone),
                    !Orders(_, custkey);
            }.map(|(custkey,)| (*custkey,)).collect()
        }
    };
}

crate::tpch_runtime::configuration!(default, $, {
    select = ::mini_linq::passes::select(positional(Customer, Orders));
}, { query_body!(); });
crate::tpch_runtime::configuration!(transfer, $, {
    plan = ::cq_planners::order(transfer);
    select = ::mini_linq::passes::select(positional(Customer, Orders));
}, { query_body!(); });
crate::tpch_runtime::configuration!(le, $, {
    plan = ::cq_planners::order(le);
    select = ::mini_linq::passes::select(positional(Customer, Orders));
}, { query_body!(); });
crate::tpch_runtime::configuration!(le_runs, $, {
    plan = ::cq_planners::order(le);
    select = ::mini_linq::passes::select(positional(Customer, Orders));
}, [::cq_rewrites::sorted_runs, ::cq_rewrites::retain_lookups], { query_body!(); });
crate::tpch_runtime::candidate!(hypergraph; $; positional(Customer, Orders); query_body!(););
crate::tpch_runtime::advised_config!([no_reduce, order(Customer)]; $;
    positional(Customer, Orders); query_body!(););

pub fn run(data: &Data, config: &str) -> Vec<Answer> {
    match config {
        "default" => default::run(data),
        "par" => par::run(data),
        "no-reduce" => no_reduce::run(data),
        "transfer" => transfer::run(data),
        "le" => le::run(data),
        "le_runs" => le_runs::run(data),
        "hypergraph" => tuned::run(data),
        "advised" => advised::run(data),
        _ => panic!("unknown configuration {}", config),
    }
}

pub fn canonical(rows: &[Answer]) -> Vec<Vec<String>> {
    rows.iter().map(|(key,)| vec![key.to_string()]).collect()
}
