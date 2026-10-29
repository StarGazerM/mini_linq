// TPC-H-derived positive Q7 join core with two aliases of the real nation table.
// Dates and the France/Germany predicate are retained; revenue/grouping are absent.
use crate::tpch_runtime::date;
use cq_load::Date;

// String-valued input columns stay dictionary-coded throughout execution.

pub const NAME: &str = "retail_h07";
pub const SQL: &str = r#"SELECT DISTINCT o.o_orderkey, ns.n_nationkey, nc.n_nationkey
FROM supplier s JOIN lineitem l ON s.s_suppkey = l.l_suppkey
JOIN orders o ON l.l_orderkey = o.o_orderkey
JOIN customer c ON c.c_custkey = o.o_custkey
JOIN nation ns ON s.s_nationkey = ns.n_nationkey
JOIN nation nc ON c.c_nationkey = nc.n_nationkey
WHERE ((ns.n_name = 'FRANCE' AND nc.n_name = 'GERMANY')
OR (ns.n_name = 'GERMANY' AND nc.n_name = 'FRANCE'))
AND l.l_shipdate >= DATE '1995-01-01' AND l.l_shipdate <= DATE '1996-12-31'"#;
pub const CONFIGS: &[&str] = &["default", "par", "radix", "radix_retained", "built_hash", "built_radix", "no-reduce", "transfer", "le", "le_runs", "hypergraph", "advised"];
pub const EXPORTS: &[(&str, &str)] = &[
    ("supplier", "s_suppkey,s_nationkey"),
    ("lineitem", "l_orderkey,l_suppkey,l_shipdate"),
    ("orders", "o_orderkey,o_custkey"),
    ("customer", "c_custkey,c_nationkey"),
    ("nation", "n_nationkey,n_name"),
];

pub type Supplier = (i64, i32,);
pub type Lineitem = (i64, i64, Date,);
pub type Orders = (i64, i64,);
pub type Customer = (i64, i32,);
pub type Nation = (i32, cq_load::StringCode,);
pub type Answer = (i64, i32, i32,);
pub struct Data {
    pub supplier: Vec<Supplier>,
    pub lineitem: Vec<Lineitem>,
    pub orders: Vec<Orders>,
    pub customer: Vec<Customer>,
    pub nation: Vec<Nation>,
    pub dictionaries: cq_load::Dictionaries,
}

mini_linq::interned_relation_loaders! {
    relation Supplier(s_suppkey: i64, s_nationkey: i32);
    relation Lineitem(l_orderkey: i64, l_suppkey: i64, l_shipdate: Date);
    relation Orders(o_orderkey: i64, o_custkey: i64);
    relation Customer(c_custkey: i64, c_nationkey: i32);
    relation Nation(n_nationkey: i32, n_name: cq_load::StringCode);
}

pub fn load(root: &std::path::Path, config: &str) -> Result<Data, Box<dyn std::error::Error>> {
    let _ = config;
    let mut dictionaries = cq_load::Dictionaries::default();
    let data = Data {
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
        nation: <relation_loader!(Nation)>::parquet_interned(
            root.join("nation.parquet"),
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
            select = ::mini_linq::passes::select(positional(Supplier, Lineitem, Orders, Customer, Nation));
        }, [$($extra),*], {
            #[allow(non_snake_case)]
            pub(super) fn run(data: &Data) -> Vec<Answer> {
                let Supplier = data.supplier.as_slice();
                let Lineitem = data.lineitem.as_slice();
                let Orders = data.orders.as_slice();
                let Customer = data.customer.as_slice();
                let Nation = data.nation.as_slice();
                let dictionaries = &data.dictionaries;
                let start = date("1995-01-01"); let end = date("1996-12-31");
                let france = dictionaries.literal("n_name", "FRANCE");
                let germany = dictionaries.literal("n_name", "GERMANY");
                query! {
                    struct Query;
                    relation Supplier(s_suppkey: i64, s_nationkey: i32);
                    relation Lineitem(l_orderkey: i64, l_suppkey: i64, l_shipdate: Date);
                    relation Orders(o_orderkey: i64, o_custkey: i64);
                    relation Customer(c_custkey: i64, c_nationkey: i32);
                    relation Nation(n_nationkey: i32, n_name: cq_load::StringCode);
                    relation answer(order_key: i64, supplier_nation: i32, customer_nation: i32);
                    answer(order_key, supplier_nation, customer_nation) :-
                        Lineitem(order_key, supplier_key, ship_date), where ship_date >= start && ship_date <= end,
                        Supplier(supplier_key, supplier_nation),
                        Orders(order_key, customer_key), Customer(customer_key, customer_nation),
                        Nation(supplier_nation, supplier_name), Nation(customer_nation, customer_name),
                        where (supplier_name == france && customer_name == germany) ||
                            (supplier_name == germany && customer_name == france);
                }
                .map(|(order_key, supplier_nation, customer_nation,)| (*order_key, *supplier_nation, *customer_nation,))
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
runner!($; advised, { plan = ::mini_linq::passes::plan(no_reduce, order(Nation@6, Nation@5, Customer, Orders, Lineitem, Supplier)); }, []);

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
    rows.iter().map(|(order_key, supplier_nation, customer_nation,)| vec![order_key.to_string(), supplier_nation.to_string(), customer_nation.to_string()]).collect()
}
