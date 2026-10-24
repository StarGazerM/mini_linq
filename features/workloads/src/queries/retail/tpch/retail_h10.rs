//! TPC-H Q10 customer revenue core, without ordering or limit.
//! Revenue is an exact scale-four coefficient; account balance is scale two.
//! Order/line identities remain named witnesses in the grouped fold.
use cq_load::{Date, Decimal, Load};

pub const NAME: &str = "retail_h10";

pub const EXPORTS: &[(&str, &str)] = &[
    (
        "customer",
        "c_custkey,c_name,c_acctbal,c_nationkey,c_address,c_phone,c_comment",
    ),
    ("orders", "o_orderkey,o_custkey,o_orderdate"),
    (
        "lineitem",
        "l_orderkey,l_linenumber,l_extendedprice,l_discount,l_returnflag",
    ),
    ("nation", "n_nationkey,n_name"),
];
pub const SQL: &str = r#"SELECT c_custkey, c_name,
    CAST(SUM(l_extendedprice * (1 - l_discount)) * 10000 AS BIGINT) AS revenue,
    CAST(c_acctbal * 100 AS BIGINT) AS account_balance, n_name, c_address, c_phone, c_comment
FROM (SELECT DISTINCT c.c_custkey, c.c_name, c.c_acctbal, c.c_nationkey,
    c.c_address, c.c_phone, c.c_comment, o.o_orderkey, o.o_orderdate,
    l.l_linenumber, l.l_extendedprice, l.l_discount, l.l_returnflag, n.n_name
    FROM customer c JOIN orders o ON c.c_custkey = o.o_custkey
    JOIN lineitem l ON l.l_orderkey = o.o_orderkey
    JOIN nation n ON n.n_nationkey = c.c_nationkey
    WHERE o.o_orderdate >= DATE '1993-10-01' AND o.o_orderdate < DATE '1994-01-01'
      AND l.l_returnflag = 'R') AS matches
GROUP BY c_custkey, c_name, c_acctbal, n_name, c_address, c_phone, c_comment"#;

pub type Customer = (i64, String, Decimal, i32, String, String, String);
pub type Orders = (i64, i64, Date);
pub type Lineitem = (i64, i64, Decimal, Decimal, String);
pub type Nation = (i32, String);
pub type Answer = (i64, String, i64, i64, String, String, String, String);
pub struct Data {
    pub customer: Vec<Customer>,
    pub orders: Vec<Orders>,
    pub lineitem: Vec<Lineitem>,
    pub nation: Vec<Nation>,
}

mini_linq::relation_loaders! {
    relation Customer(c_custkey: i64, c_name: String, c_acctbal: Decimal,
        c_nationkey: i32, c_address: String, c_phone: String, c_comment: String);
    relation Orders(o_orderkey: i64, o_custkey: i64, o_orderdate: Date);
    relation Lineitem(l_orderkey: i64, l_linenumber: i64,
        l_extendedprice: Decimal, l_discount: Decimal, l_returnflag: String);
    relation Nation(n_nationkey: i32, n_name: String);
}

pub fn load(root: &std::path::Path, config: &str) -> Result<Data, Box<dyn std::error::Error>> {
    assert!(CONFIGS.contains(&config), "unknown H10 configuration");
    Ok(Data {
        customer: <relation_loader!(Customer)>::parquet(
            root.join("customer.parquet"),
            cq_load::ParquetOptions::default(),
        )?,
        orders: <relation_loader!(Orders)>::parquet(
            root.join("orders.parquet"),
            cq_load::ParquetOptions::default(),
        )?,
        lineitem: <relation_loader!(Lineitem)>::parquet(
            root.join("lineitem.parquet"),
            cq_load::ParquetOptions::default(),
        )?,
        nation: <relation_loader!(Nation)>::parquet(
            root.join("nation.parquet"),
            cq_load::ParquetOptions::default(),
        )?,
    })
}

crate::tpch_runtime::standalone_configs! { $;
    pub(super) fn run(data: &Data, config: &str) -> Vec<Answer> {
        assert!(CONFIGS.contains(&config), "unknown H10 configuration");
        let Customer = data.customer.as_slice();
        let Orders = data.orders.as_slice();
        let Lineitem = data.lineitem.as_slice();
        let Nation = data.nation.as_slice();
        let start = Date::load("1993-10-01").expect("authored ISO date");
        let end = Date::load("1994-01-01").expect("authored ISO date");
        query! {
            plan(no_reduce, order(Orders, Lineitem, Customer, Nation));
            struct Query;
            relation Customer(c_custkey: i64, c_name: String, c_acctbal: Decimal,
                c_nationkey: i32, c_address: String, c_phone: String, c_comment: String);
            relation Orders(o_orderkey: i64, o_custkey: i64, o_orderdate: Date);
            relation Lineitem(l_orderkey: i64, l_linenumber: i64,
                l_extendedprice: Decimal, l_discount: Decimal, l_returnflag: String);
            relation Nation(n_nationkey: i32, n_name: String);
            relation answer(customer_key: i64, name: String, revenue: i64,
                account_balance: i64, nation: String, address: String, phone: String, comment: String);
            answer(customer_key, name, sum(revenue_units), balance_units, nation, address, phone, comment) :-
                Orders(order_key, customer_key, order_date),
                if *order_date >= start && *order_date < end,
                Lineitem(order_key, line_number, price, discount, "R"),
                Customer(customer_key, name, balance, nation_key, address, phone, comment),
                Nation(nation_key, nation),
                let revenue_units: i64 = price.scaled() * (100 - discount.scaled()),
                let balance_units: i64 = balance.scaled();
        }
        .map(|(key, name, revenue, balance, nation, address, phone, comment)| {
            (*key, name.clone(), revenue, balance, nation.clone(), address.clone(), phone.clone(), comment.clone())
        })
        .collect()
    }
}

pub fn canonical(rows: &[Answer]) -> Vec<Vec<String>> {
    rows.iter()
        .map(
            |(key, name, revenue, balance, nation, address, phone, comment)| {
                vec![
                    key.to_string(),
                    name.clone(),
                    revenue.to_string(),
                    balance.to_string(),
                    nation.clone(),
                    address.clone(),
                    phone.clone(),
                    comment.clone(),
                ]
            },
        )
        .collect()
}
