//! TPC-H H01-style grouped sums/count over distinct projected lineitems.
//! Order/line identity is retained, so equal prices on different lines add twice.
//! Sums are exact scale-two integer coefficients; this derived query omits
//! Q1's averages, discount/tax expressions, and final ordering.
use cq_load::{Date, Decimal, Load};

pub const NAME: &str = "retail_h01";

pub const EXPORTS: &[(&str, &str)] = &[(
    "lineitem",
    "l_orderkey,l_linenumber,l_returnflag,l_linestatus,l_quantity,l_extendedprice,l_shipdate",
)];
pub const SQL: &str = r#"SELECT l_returnflag, l_linestatus,
    CAST(SUM(l_quantity) * 100 AS BIGINT) AS sum_quantity,
    CAST(SUM(l_extendedprice) * 100 AS BIGINT) AS sum_price, COUNT(*) AS count_order
FROM (SELECT DISTINCT l_orderkey, l_linenumber, l_returnflag, l_linestatus,
    l_quantity, l_extendedprice, l_shipdate FROM lineitem
    WHERE l_shipdate <= DATE '1998-09-02') AS matches
GROUP BY l_returnflag, l_linestatus"#;

pub type Lineitem = (i64, i64, String, String, Decimal, Decimal, Date);
pub type Answer = (String, String, i64, i64, usize);
pub struct Data {
    pub lineitem: Vec<Lineitem>,
}

mini_linq::relation_loaders! {
    relation Lineitem(l_orderkey: i64, l_linenumber: i64,
        l_returnflag: String, l_linestatus: String, l_quantity: Decimal,
        l_extendedprice: Decimal, l_shipdate: Date);
}

pub fn load(root: &std::path::Path, config: &str) -> Result<Data, Box<dyn std::error::Error>> {
    assert!(CONFIGS.contains(&config), "unknown H01 configuration");
    Ok(Data {
        lineitem: <relation_loader!(Lineitem)>::parquet(
            root.join("lineitem.parquet"),
            cq_load::ParquetOptions::default(),
        )?,
    })
}

crate::tpch_runtime::standalone_configs! { $;
    pub(super) fn run(data: &Data, config: &str) -> Vec<Answer> {
        assert!(CONFIGS.contains(&config), "unknown H01 configuration");
        let Lineitem = data.lineitem.as_slice();
        let cutoff = Date::load("1998-09-02").expect("authored ISO date");
        query! {
            plan(no_reduce, order(Lineitem));
            struct Query;
            relation Lineitem(l_orderkey: i64, l_linenumber: i64,
                l_returnflag: String, l_linestatus: String, l_quantity: Decimal,
                l_extendedprice: Decimal, l_shipdate: Date);
            relation answer(returnflag: String, linestatus: String,
                sum_quantity: i64, sum_price: i64, count_order: usize);
            answer(returnflag, linestatus, sum(quantity_units), sum(price_units), count()) :-
                Lineitem(order_key, line_number, returnflag, linestatus, quantity, price, shipdate),
                if *shipdate <= cutoff,
                let quantity_units: i64 = quantity.scaled(),
                let price_units: i64 = price.scaled();
        }
        .map(|(flag, status, quantity, price, count)| {
            (flag.clone(), status.clone(), quantity, price, count)
        })
        .collect()
    }
}

pub fn canonical(rows: &[Answer]) -> Vec<Vec<String>> {
    rows.iter()
        .map(|(flag, status, quantity, price, count)| {
            vec![
                flag.clone(),
                status.clone(),
                quantity.to_string(),
                price.to_string(),
                count.to_string(),
            ]
        })
        .collect()
}
