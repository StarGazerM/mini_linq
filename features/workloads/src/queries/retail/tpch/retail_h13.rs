//! TPC-H Q13's per-customer order count, including customers with zero matches.
//! The outer distribution of these counts and its ordering are intentionally absent.
//! Qualifying orders are filtered once; zero counts use a native CQ anti-join.

pub const NAME: &str = "retail_h13";
pub const CONFIGS: &[&str] = &["default", "radix", "radix_retained"];
pub const EXPORTS: &[(&str, &str)] = &[
    ("customer", "c_custkey"),
    ("orders", "o_orderkey,o_custkey,o_comment"),
];
pub const SQL: &str = r#"SELECT c.c_custkey, COUNT(o.o_orderkey)
FROM (SELECT DISTINCT c_custkey FROM customer) c
LEFT JOIN (SELECT DISTINCT o_orderkey, o_custkey FROM orders
    WHERE o_comment NOT LIKE '%special%requests%') o ON o.o_custkey = c.c_custkey
GROUP BY c.c_custkey"#;

pub type Customer = (i64,);
pub type Orders = (i64, i64, String);
pub type Answer = (i64, usize);
pub struct Data {
    pub customer: Vec<Customer>,
    pub orders: Vec<Orders>,
}

mini_linq::relation_loaders! {
    relation Customer(c_custkey: i64);
    relation Orders(o_orderkey: i64, o_custkey: i64, o_comment: String);
}

pub fn load(root: &std::path::Path, config: &str) -> Result<Data, Box<dyn std::error::Error>> {
    assert!(CONFIGS.contains(&config), "unknown H13 configuration");
    Ok(Data {
        customer: <relation_loader!(Customer)>::parquet(
            root.join("customer.parquet"), cq_load::ParquetOptions::default(),
        )?,
        orders: <relation_loader!(Orders)>::parquet(
            root.join("orders.parquet"), cq_load::ParquetOptions::default(),
        )?,
    })
}

crate::tpch_runtime::same_plan_kernel! { $;
#[allow(non_snake_case)]
pub(super) fn execute(data: &Data) -> Vec<Answer> {
    let Customer = data.customer.as_slice();
    let QualifiedOrders: Vec<(i64, i64)> = data.orders.iter()
        .filter(|row| !row.2.split_once("special")
            .is_some_and(|(_, rest)| rest.contains("requests")))
        .map(|row| (row.0, row.1))
        .collect();
    let QualifiedOrders = QualifiedOrders.as_slice();
    let counts = query! {
        pipeline {}
        struct Counts;
        relation Customer(c_custkey: i64);
        relation QualifiedOrders(order_key: i64, customer_key: i64);
        relation answer(customer_key: i64, order_count: usize);
        answer(customer_key, count()) :-
            Customer(customer_key), QualifiedOrders(order_key, customer_key);
    }
    .map(|(customer_key, count)| (*customer_key, count));
    let zeros = query! {
        pipeline {}
        struct Zeros;
        relation Customer(c_custkey: i64);
        relation QualifiedOrders(order_key: i64, customer_key: i64);
        relation answer(customer_key: i64);
        answer(customer_key) :- Customer(customer_key), !QualifiedOrders(_, customer_key);
    }
    .map(|(customer_key,)| (*customer_key, 0));
    counts.chain(zeros).collect()
}
}

pub fn canonical(rows: &[Answer]) -> Vec<Vec<String>> {
    rows.iter().map(|(customer_key, count)| {
        vec![customer_key.to_string(), count.to_string()]
    }).collect()
}
