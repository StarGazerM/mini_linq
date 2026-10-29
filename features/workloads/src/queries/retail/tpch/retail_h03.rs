// TPC-H-derived positive Q3 join core: no revenue, grouping, ordering, or limit.
use crate::tpch_runtime::date;
use cq_load::Date;

// String-valued input columns stay dictionary-coded throughout execution.

pub const NAME: &str = "retail_h03";
pub const SQL: &str = r#"SELECT DISTINCT o.o_orderkey, c.c_custkey
FROM customer c JOIN orders o ON o.o_custkey = c.c_custkey
JOIN lineitem l ON l.l_orderkey = o.o_orderkey
WHERE c.c_mktsegment = 'BUILDING' AND o.o_orderdate < DATE '1995-03-15'
AND l.l_shipdate > DATE '1995-03-15'"#;
pub const CONFIGS: &[&str] = &["default", "par", "no-reduce", "transfer", "le", "le_runs", "hypergraph", "advised"];
pub const EXPORTS: &[(&str, &str)] = &[
    ("customer", "c_custkey,c_mktsegment"),
    ("orders", "o_orderkey,o_custkey,o_orderdate"),
    ("lineitem", "l_orderkey,l_shipdate"),
];

pub type Customer = (i64, cq_load::StringCode,);
pub type Orders = (i64, i64, Date,);
pub type Lineitem = (i64, Date,);
pub type Answer = (i64, i64,);
pub struct Data {
    pub customer: Vec<Customer>,
    pub orders: Vec<Orders>,
    pub lineitem: Vec<Lineitem>,
    pub dictionaries: cq_load::Dictionaries,
}

mini_linq::interned_relation_loaders! {
    relation Customer(c_custkey: i64, c_mktsegment: cq_load::StringCode);
    relation Orders(o_orderkey: i64, o_custkey: i64, o_orderdate: Date);
    relation Lineitem(l_orderkey: i64, l_shipdate: Date);
}

pub fn load(root: &std::path::Path, config: &str) -> Result<Data, Box<dyn std::error::Error>> {
    let _ = config;
    let mut dictionaries = cq_load::Dictionaries::default();
    let data = Data {
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
        lineitem: <relation_loader!(Lineitem)>::parquet_interned(
            root.join("lineitem.parquet"),
            cq_load::ParquetOptions::default(),
            &mut dictionaries,
        )?,
        dictionaries,
    };
    Ok(data)
}

macro_rules! runner {
    ($d:tt; $module:ident, {$($stages:tt)*}, [$($extra:path),*]) => {
        crate::tpch_runtime::configuration!($module, $d, {
            $($stages)*
            select = ::mini_linq::passes::select(positional(Customer, Orders, Lineitem));
        }, [$($extra),*], {
            #[allow(non_snake_case)]
            pub(super) fn run(data: &Data) -> Vec<Answer> {
                let Customer = data.customer.as_slice();
                let Orders = data.orders.as_slice();
                let Lineitem = data.lineitem.as_slice();
                let dictionaries = &data.dictionaries;
                let cutoff = date("1995-03-15");
                let building = dictionaries.literal("c_mktsegment", "BUILDING");
                query! {
                    struct Query;
                    relation Customer(c_custkey: i64, c_mktsegment: cq_load::StringCode);
                    relation Orders(o_orderkey: i64, o_custkey: i64, o_orderdate: Date);
                    relation Lineitem(l_orderkey: i64, l_shipdate: Date);
                    relation answer(order_key: i64, customer_key: i64);
                    answer(order_key, customer_key) :-
                        Lineitem(order_key, ship_date), where ship_date > cutoff,
                        Orders(order_key, customer_key, order_date), where order_date < cutoff,
                        Customer(customer_key, { building });
                }
                .map(|(order_key, customer_key,)| (*order_key, *customer_key,))
                .collect()
            }
        });
    };
}

runner!($; default, {}, []);
runner!($; par, {
    decompose = [::cq_sequence_tasks_macros::plan];
    grade = ::cq_sequence_tasks_macros::tasks;
    print = ::cq_sequence_tasks_macros::translate_tasks;
}, []);
runner!($; no_reduce, { plan = ::mini_linq::passes::plan(no_reduce); }, []);
runner!($; transfer, { plan = ::cq_planners::order(transfer); }, []);
runner!($; le, { plan = ::cq_planners::order(le); }, []);
runner!($; le_runs, { plan = ::cq_planners::order(le); }, [::cq_rewrites::sorted_runs, ::cq_rewrites::retain_lookups]);
runner!($; tuned, { plan = ::cq_planners::order(hypergraph); }, []);
runner!($; advised, { plan = ::mini_linq::passes::plan(no_reduce, order(Customer, Orders, Lineitem)); }, []);

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
    rows.iter().map(|(order_key, customer_key,)| vec![order_key.to_string(), customer_key.to_string()]).collect()
}
