//! TPC-H H18 large orders: per-order quantity strictly greater than 300.
//! Two compiled rules materialize the grouped quantity before the threshold and
//! customer/order join; CQ's same-head rule union is not a derived-relation DAG.
//! Quantity and total price are exact scale-two coefficients. Ordering and the
//! top-100 limit are omitted. Named order/line identities preserve equal amounts.
//! Customer names remain dictionary codes until answer publication.
use cq_load::{Date, Decimal, StringCode};

pub const NAME: &str = "retail_h18";

pub const EXPORTS: &[(&str, &str)] = &[
    ("lineitem", "l_orderkey,l_linenumber,l_quantity"),
    ("orders", "o_orderkey,o_custkey,o_orderdate,o_totalprice"),
    ("customer", "c_custkey,c_name"),
];
pub const SQL: &str = r#"WITH quantities AS (
    SELECT l_orderkey, CAST(SUM(l_quantity) * 100 AS BIGINT) AS sum_quantity
    FROM (SELECT DISTINCT l_orderkey, l_linenumber, l_quantity FROM lineitem) AS lines
    GROUP BY l_orderkey HAVING SUM(l_quantity) > 300.00
)
SELECT DISTINCT c.c_name, c.c_custkey, o.o_orderkey,
    (o.o_orderdate - DATE '1970-01-01')::INTEGER AS o_orderdate,
    CAST(o.o_totalprice * 100 AS BIGINT) AS total_price, q.sum_quantity
FROM quantities q JOIN orders o ON q.l_orderkey = o.o_orderkey
JOIN customer c ON o.o_custkey = c.c_custkey"#;

pub type Lineitem = (i64, i64, Decimal);
pub type Orders = (i64, i64, Date, Decimal);
pub type Customer = (i64, StringCode);
pub type Answer = (String, i64, i64, Date, i64, i64);
pub struct Data {
    pub lineitem: Vec<Lineitem>,
    pub orders: Vec<Orders>,
    pub customer: Vec<Customer>,
    pub dictionaries: cq_load::Dictionaries,
}

mini_linq::interned_relation_loaders! {
    relation Lineitem(l_orderkey: i64, l_linenumber: i64, l_quantity: Decimal);
    relation Orders(o_orderkey: i64, o_custkey: i64, o_orderdate: Date, o_totalprice: Decimal);
    relation Customer(c_custkey: i64, c_name: StringCode);
}

pub fn load(root: &std::path::Path, config: &str) -> Result<Data, Box<dyn std::error::Error>> {
    assert!(CONFIGS.contains(&config), "unknown H18 configuration");
    let mut dictionaries = cq_load::Dictionaries::default();
    Ok(Data {
        lineitem: <relation_loader!(Lineitem)>::parquet_interned(
            root.join("lineitem.parquet"),
            cq_load::ParquetOptions::default(),
            &mut dictionaries,
        )?,
        orders: <relation_loader!(Orders)>::parquet_interned(
            root.join("orders.parquet"),
            cq_load::ParquetOptions::default(),
            &mut dictionaries,
        )?,
        customer: <relation_loader!(Customer)>::parquet_interned(
            root.join("customer.parquet"),
            cq_load::ParquetOptions::default(),
            &mut dictionaries,
        )?,
        dictionaries,
    })
}

crate::tpch_runtime::standalone_configs! { $;
    pub(super) fn run(data: &Data, config: &str) -> Vec<Answer> {
        assert!(CONFIGS.contains(&config), "unknown H18 configuration");
        let Lineitem = data.lineitem.as_slice();
        let quantities: Vec<(i64, i64)> = query! {
            plan(no_reduce, order(Lineitem));
            struct Quantities;
            relation Lineitem(l_orderkey: i64, l_linenumber: i64, l_quantity: Decimal);
            relation answer(order_key: i64, sum_quantity: i64);
            answer(order_key, sum(quantity_units)) :-
                Lineitem(order_key, line_number, quantity),
                let quantity_units: i64 = quantity.scaled();
        }
        .map(|(order_key, quantity)| (*order_key, quantity))
        .collect();
        let Quantity = quantities.as_slice();
        let Orders = data.orders.as_slice();
        let Customer = data.customer.as_slice();
        query! {
            plan(no_reduce, order(Quantity, Orders, Customer));
            struct Query;
            relation Quantity(order_key: i64, sum_quantity: i64);
            relation Orders(o_orderkey: i64, o_custkey: i64, o_orderdate: Date, o_totalprice: Decimal);
            relation Customer(c_custkey: i64, c_name: StringCode);
            relation answer(customer_name: StringCode, customer_key: i64, order_key: i64,
                order_date: Date, total_price: i64, sum_quantity: i64);
            answer(customer_name, customer_key, order_key, order_date, price_units, sum_quantity) :-
                Quantity(order_key, sum_quantity), where sum_quantity > 30000,
                Orders(order_key, customer_key, order_date, total_price),
                Customer(customer_key, customer_name),
                let price_units: i64 = total_price.scaled();
        }
        .map(|(name, customer, order, date, price, quantity)| {
            // DECODE P: publish the customer name in the final answer.
            (data.dictionaries.publish("c_name", *name), *customer, *order, *date, price, *quantity)
        })
        .collect()
    }
}

pub fn canonical(rows: &[Answer]) -> Vec<Vec<String>> {
    rows.iter()
        .map(|(name, customer, order, date, price, quantity)| {
            vec![
                name.clone(),
                customer.to_string(),
                order.to_string(),
                date.days_since_epoch().to_string(),
                price.to_string(),
                quantity.to_string(),
            ]
        })
        .collect()
}
