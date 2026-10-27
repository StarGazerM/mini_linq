//! TPC-H Q6 global discounted revenue, without ordering.
//! Inputs are scale-two decimals; revenue is an exact scale-four coefficient.
//! Named order/line identities preserve equal-valued distinct lineitems.
use cq_load::{Date, Decimal, Load};

pub const NAME: &str = "retail_h06";

pub const EXPORTS: &[(&str, &str)] = &[(
    "lineitem",
    "l_orderkey,l_linenumber,l_extendedprice,l_discount,l_quantity,l_shipdate",
)];
pub const SQL: &str = r#"SELECT CAST(COALESCE(SUM(l_extendedprice * l_discount), 0) * 10000 AS BIGINT) AS revenue
FROM (SELECT DISTINCT l_orderkey, l_linenumber, l_extendedprice, l_discount,
    l_quantity, l_shipdate FROM lineitem
    WHERE l_shipdate >= DATE '1994-01-01' AND l_shipdate < DATE '1995-01-01'
      AND l_discount BETWEEN 0.05 AND 0.07 AND l_quantity < 24) AS matches"#;

pub type Lineitem = (i64, i64, Decimal, Decimal, Decimal, Date);
pub type Answer = (i64,);
pub struct Data {
    pub lineitem: Vec<Lineitem>,
}

mini_linq::relation_loaders! {
    relation Lineitem(l_orderkey: i64, l_linenumber: i64,
        l_extendedprice: Decimal, l_discount: Decimal, l_quantity: Decimal,
        l_shipdate: Date);
}

pub fn load(root: &std::path::Path, config: &str) -> Result<Data, Box<dyn std::error::Error>> {
    assert!(CONFIGS.contains(&config), "unknown H06 configuration");
    Ok(Data {
        lineitem: <relation_loader!(Lineitem)>::parquet(
            root.join("lineitem.parquet"),
            cq_load::ParquetOptions::default(),
        )?,
    })
}

crate::tpch_runtime::standalone_configs! { $;
    pub(super) fn run(data: &Data, config: &str) -> Vec<Answer> {
        assert!(CONFIGS.contains(&config), "unknown H06 configuration");
        let Lineitem = data.lineitem.as_slice();
        let start = Date::load("1994-01-01").expect("authored ISO date");
        let end = Date::load("1995-01-01").expect("authored ISO date");
        query! {
            plan(no_reduce, order(Lineitem));
            struct Query;
            relation Lineitem(l_orderkey: i64, l_linenumber: i64,
                l_extendedprice: Decimal, l_discount: Decimal, l_quantity: Decimal,
                l_shipdate: Date);
            relation answer(revenue: i64);
            answer(sum(revenue_units)) :-
                Lineitem(order_key, line_number, price, discount, quantity, shipdate),
                if *shipdate >= start && *shipdate < end,
                if discount.scaled() >= 5 && discount.scaled() <= 7,
                if quantity.scaled() < 2400,
                let revenue_units: i64 = price.scaled() * discount.scaled();
        }
        .collect()
    }
}

pub fn canonical(rows: &[Answer]) -> Vec<Vec<String>> {
    rows.iter()
        .map(|(revenue,)| vec![revenue.to_string()])
        .collect()
}
