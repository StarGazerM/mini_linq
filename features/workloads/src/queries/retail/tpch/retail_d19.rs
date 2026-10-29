// TPC-H-derived re-authoring of the TPC-DS-inspired D19 dimensional join shape.
// Roles: store_sales -> lineitem plus orders (customer/orderdate live on orders),
// date_dim -> orders' real orderdate filter; item -> part; customer -> customer;
// customer_address -> nation; store -> supplier. There are no fabricated TPC-DS
// tables, surrogate dates, or claims that dbgen can supply TPC-DS semantics.
use crate::tpch_runtime::date;
use cq_load::Date;

// String-valued input columns stay dictionary-coded throughout execution.

pub const NAME: &str = "retail_d19";
pub const SQL: &str = r#"SELECT DISTINCT o.o_orderkey, p.p_partkey, c.c_custkey, s.s_suppkey
FROM lineitem l JOIN orders o ON l.l_orderkey = o.o_orderkey
JOIN part p ON l.l_partkey = p.p_partkey
JOIN customer c ON o.o_custkey = c.c_custkey
JOIN nation n ON c.c_nationkey = n.n_nationkey
JOIN supplier s ON l.l_suppkey = s.s_suppkey
WHERE o.o_orderdate >= DATE '1995-01-01' AND o.o_orderdate < DATE '1995-04-01'
AND n.n_name = 'UNITED STATES'"#;
pub const CONFIGS: &[&str] = &["default", "par", "radix", "radix_retained", "built_hash", "built_radix", "no-reduce", "transfer", "le", "le_runs", "hypergraph", "advised"];
pub const EXPORTS: &[(&str, &str)] = &[
    ("lineitem", "l_orderkey,l_partkey,l_suppkey"),
    ("orders", "o_orderkey,o_custkey,o_orderdate"),
    ("part", "p_partkey"),
    ("customer", "c_custkey,c_nationkey"),
    ("nation", "n_nationkey,n_name"),
    ("supplier", "s_suppkey"),
];

pub type Lineitem = (i64, i64, i64,);
pub type Orders = (i64, i64, Date,);
pub type Part = (i64,);
pub type Customer = (i64, i32,);
pub type Nation = (i32, cq_load::StringCode,);
pub type Supplier = (i64,);
pub type Answer = (i64, i64, i64, i64,);
pub struct Data {
    pub lineitem: Vec<Lineitem>,
    pub orders: Vec<Orders>,
    pub part: Vec<Part>,
    pub customer: Vec<Customer>,
    pub nation: Vec<Nation>,
    pub supplier: Vec<Supplier>,
    pub dictionaries: cq_load::Dictionaries,
}

mini_linq::interned_relation_loaders! {
    relation Lineitem(l_orderkey: i64, l_partkey: i64, l_suppkey: i64);
    relation Orders(o_orderkey: i64, o_custkey: i64, o_orderdate: Date);
    relation Part(p_partkey: i64);
    relation Customer(c_custkey: i64, c_nationkey: i32);
    relation Nation(n_nationkey: i32, n_name: cq_load::StringCode);
    relation Supplier(s_suppkey: i64);
}

pub fn load(root: &std::path::Path, config: &str) -> Result<Data, Box<dyn std::error::Error>> {
    let _ = config;
    let mut dictionaries = cq_load::Dictionaries::default();
    let data = Data {
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
        part: <relation_loader!(Part)>::parquet_interned(
            root.join("part.parquet"),
            cq_load::ParquetOptions::default(),
            &mut dictionaries,
        )?,
        customer: <relation_loader!(Customer)>::parquet_interned(
            root.join("customer.parquet"),
            cq_load::ParquetOptions::default(),
            &mut dictionaries,
        )?,
        nation: <relation_loader!(Nation)>::parquet_interned(
            root.join("nation.parquet"),
            cq_load::ParquetOptions::default(),
            &mut dictionaries,
        )?,
        supplier: <relation_loader!(Supplier)>::parquet_interned(
            root.join("supplier.parquet"),
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
            select = ::mini_linq::passes::select(positional(Lineitem, Orders, Part, Customer, Nation, Supplier));
        }, [$($extra),*], {
            #[allow(non_snake_case)]
            pub(super) fn run(data: &Data) -> Vec<Answer> {
                let Lineitem = data.lineitem.as_slice();
                let Orders = data.orders.as_slice();
                let Part = data.part.as_slice();
                let Customer = data.customer.as_slice();
                let Nation = data.nation.as_slice();
                let Supplier = data.supplier.as_slice();
                let dictionaries = &data.dictionaries;
                let start = date("1995-01-01"); let end = date("1995-04-01");
                let united_states = dictionaries.literal("n_name", "UNITED STATES");
                query! {
                    struct Query;
                    relation Lineitem(l_orderkey: i64, l_partkey: i64, l_suppkey: i64);
                    relation Orders(o_orderkey: i64, o_custkey: i64, o_orderdate: Date);
                    relation Part(p_partkey: i64);
                    relation Customer(c_custkey: i64, c_nationkey: i32);
                    relation Nation(n_nationkey: i32, n_name: cq_load::StringCode);
                    relation Supplier(s_suppkey: i64);
                    relation answer(order_key: i64, part_key: i64, customer_key: i64, supplier_key: i64);
                    answer(order_key, part_key, customer_key, supplier_key) :-
                        Lineitem(order_key, part_key, supplier_key),
                        Orders(order_key, customer_key, order_date), where order_date >= start && order_date < end,
                        Part(part_key), Customer(customer_key, nation_key),
                        Nation(nation_key, { united_states }),
                        Supplier(supplier_key);
                }
                .map(|(order_key, part_key, customer_key, supplier_key,)| (*order_key, *part_key, *customer_key, *supplier_key,))
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
runner!($; advised, { plan = ::mini_linq::passes::plan(no_reduce, order(Nation, Customer, Orders, Lineitem, Part, Supplier)); }, []);

pub fn run(data: &Data, config: &str) -> Vec<Answer> {
    match config {
        "default" => default::run(data),
        "radix" => radix::run(data),
        "radix_retained" => radix_retained::run(data),
        "built_hash" => built_hash::run(data),
        "built_radix" => built_radix::run(data),
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
    rows.iter().map(|(order_key, part_key, customer_key, supplier_key,)| vec![order_key.to_string(), part_key.to_string(), customer_key.to_string(), supplier_key.to_string()]).collect()
}
