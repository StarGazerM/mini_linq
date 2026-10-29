//! String columns use load-time sorted dictionary codes throughout query execution.
//! Dictionary predicate memberships are prepared once during loading.
//! TPC-H Q13's per-customer order count, including customers with zero matches.
//! The outer distribution of these counts and its ordering are intentionally absent.
//! Comments qualify once per dictionary value; projected order pairs enter the count.
//! Zero counts use a native CQ anti-join.

use cq_load::StringCode;

pub const NAME: &str = "retail_h13";

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
pub type Orders = (i64, i64, StringCode);
pub type Answer = (i64, usize);
pub struct Data {
    pub customer: Vec<Customer>,
    pub orders: Vec<Orders>,
    pub qualified_comments: cq_storage::DenseMembership<StringCode, 16777216>,
    pub dictionaries: cq_load::Dictionaries,
}

mini_linq::interned_relation_loaders! {
    relation Customer(c_custkey: i64);
    relation Orders(o_orderkey: i64, o_custkey: i64, o_comment: StringCode);
}

pub fn load(root: &std::path::Path, config: &str) -> Result<Data, Box<dyn std::error::Error>> {
    assert!(CONFIGS.contains(&config), "unknown H13 configuration");
    let mut dictionaries = cq_load::Dictionaries::default();
    Ok(Data {
        customer: <relation_loader!(Customer)>::parquet_interned(
            root.join("customer.parquet"),
            cq_load::ParquetOptions::default(),
            &mut dictionaries,
        )?,
        orders: <relation_loader!(Orders)>::parquet_interned(
            root.join("orders.parquet"),
            cq_load::ParquetOptions::default(),
            &mut dictionaries,
        )?,
        qualified_comments: cq_storage::DenseMembership::<_, 16777216>::from_dense_keys(
            cq_load::dictionary_where(&dictionaries, "o_comment", |value: &str| {
                !value
                    .split_once("special")
                    .is_some_and(|(_, rest)| rest.contains("requests"))
            })
            .into_iter()
            .map(|(code,)| code),
        ),
        dictionaries,
    })
}

crate::tpch_runtime::standalone_configs! { $;
    pub(super) fn run(data: &Data, config: &str) -> Vec<Answer> {
        assert!(CONFIGS.contains(&config), "unknown H13 configuration");
        let Customer = data.customer.as_slice();
        // Project before canonical valuation: distinct comments do not make
        // distinct order/customer pairs, and no input key is assumed.
        let QualifiedOrders: Vec<(i64, i64)> = data.orders.iter()
            .filter(|(_, _, comment)| cq_storage::ExactMembership::contains(&data.qualified_comments, comment))
            .map(|(order_key, customer_key, _)| (*order_key, *customer_key))
            .collect();
        let counts: Vec<Answer> = query! {
            plan(no_reduce, order(Customer, QualifiedOrders));
            struct Counts;
            relation Customer(c_custkey: i64);
            relation QualifiedOrders(o_orderkey: i64, o_custkey: i64);
            relation answer(customer_key: i64, order_count: usize);
            answer(customer_key, count()) :-
                Customer(customer_key), QualifiedOrders(order_key, customer_key);
        }
        .map(|(customer_key, count)| (*customer_key, count))
        .collect();
        let CountedCustomers = counts.as_slice();
        let zeros: Vec<Answer> = query! {
            plan(no_reduce, order(Customer));
            struct Zeros;
            relation Customer(c_custkey: i64);
            relation CountedCustomers(customer_key: i64, order_count: usize);
            relation answer(customer_key: i64);
            answer(customer_key) :- Customer(customer_key), !CountedCustomers(customer_key, _);
        }
        .map(|(customer_key,)| (*customer_key, 0))
        .collect();
        counts.into_iter().chain(zeros).collect()
    }
}

pub fn canonical(rows: &[Answer]) -> Vec<Vec<String>> {
    rows.iter()
        .map(|(customer_key, count)| vec![customer_key.to_string(), count.to_string()])
        .collect()
}
