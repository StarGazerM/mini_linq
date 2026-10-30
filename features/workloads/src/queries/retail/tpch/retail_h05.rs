// TPC-H-derived positive Q5 join core, including the customer/supplier nation cycle.
// Revenue and grouping are deliberately outside this inspired kernel.
use crate::tpch_runtime::date;
use cq_load::Date;

// String-valued input columns stay dictionary-coded throughout execution.

pub const NAME: &str = "retail_h05";
pub const SQL: &str = r#"SELECT DISTINCT o.o_orderkey, s.s_suppkey, n.n_nationkey
FROM customer c JOIN orders o ON c.c_custkey = o.o_custkey
JOIN lineitem l ON l.l_orderkey = o.o_orderkey
JOIN supplier s ON s.s_suppkey = l.l_suppkey AND s.s_nationkey = c.c_nationkey
JOIN nation n ON n.n_nationkey = s.s_nationkey
JOIN region r ON r.r_regionkey = n.n_regionkey
WHERE r.r_name = 'ASIA' AND o.o_orderdate >= DATE '1994-01-01'
AND o.o_orderdate < DATE '1995-01-01'"#;
pub const CONFIGS: &[&str] = &["default", "par", "radix", "radix_retained", "built_hash", "built_radix", "no-reduce", "transfer", "le", "le_runs", "generic_smallest", "advised", "step_r", "step_r_lookups", "step_r_retained"];
pub const EXPORTS: &[(&str, &str)] = &[
    ("customer", "c_custkey,c_nationkey"),
    ("orders", "o_orderkey,o_custkey,o_orderdate"),
    ("lineitem", "l_orderkey,l_suppkey"),
    ("supplier", "s_suppkey,s_nationkey"),
    ("nation", "n_nationkey,n_regionkey"),
    ("region", "r_regionkey,r_name"),
];

pub type Customer = (i64, i32,);
pub type Orders = (i64, i64, Date,);
pub type Lineitem = (i64, i64,);
pub type Supplier = (i64, i32,);
pub type Nation = (i32, i32,);
pub type Region = (i32, cq_load::StringCode,);
pub type Answer = (i64, i64, i32,);
pub struct Data {
    pub customer: Vec<Customer>,
    pub orders: Vec<Orders>,
    pub lineitem: Vec<Lineitem>,
    pub supplier: Vec<Supplier>,
    pub nation: Vec<Nation>,
    pub region: Vec<Region>,
    pub dictionaries: cq_load::Dictionaries,
}

mini_linq::interned_relation_loaders! {
    relation Customer(c_custkey: i64, c_nationkey: i32);
    relation Orders(o_orderkey: i64, o_custkey: i64, o_orderdate: Date);
    relation Lineitem(l_orderkey: i64, l_suppkey: i64);
    relation Supplier(s_suppkey: i64, s_nationkey: i32);
    relation Nation(n_nationkey: i32, n_regionkey: i32);
    relation Region(r_regionkey: i32, r_name: cq_load::StringCode);
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
        supplier: <relation_loader!(Supplier)>::parquet_interned(
            root.join("supplier.parquet"),
            cq_load::ParquetOptions::default(),
            &mut dictionaries,
        )?,
        nation: <relation_loader!(Nation)>::parquet_interned(
            root.join("nation.parquet"),
            cq_load::ParquetOptions::default(),
            &mut dictionaries,
        )?,
        region: <relation_loader!(Region)>::parquet_interned(
            root.join("region.parquet"),
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
            select = ::mini_linq::passes::select(positional(Customer, Orders, Lineitem, Supplier, Nation, Region));
        }, [$($extra),*], {
            #[allow(non_snake_case)]
            pub(super) fn run(data: &Data) -> Vec<Answer> {
                let Customer = data.customer.as_slice();
                let Orders = data.orders.as_slice();
                let Lineitem = data.lineitem.as_slice();
                let Supplier = data.supplier.as_slice();
                let Nation = data.nation.as_slice();
                let Region = data.region.as_slice();
                let dictionaries = &data.dictionaries;
                let start = date("1994-01-01"); let end = date("1995-01-01");
                let asia = dictionaries.literal("r_name", "ASIA");
                query! {
                    struct Query;
                    relation Customer(c_custkey: i64, c_nationkey: i32);
                    relation Orders(o_orderkey: i64, o_custkey: i64, o_orderdate: Date);
                    relation Lineitem(l_orderkey: i64, l_suppkey: i64);
                    relation Supplier(s_suppkey: i64, s_nationkey: i32);
                    relation Nation(n_nationkey: i32, n_regionkey: i32);
                    relation Region(r_regionkey: i32, r_name: cq_load::StringCode);
                    relation answer(order_key: i64, supplier_key: i64, nation_key: i32);
                    answer(order_key, supplier_key, nation_key) :-
                        Lineitem(order_key, supplier_key),
                        Orders(order_key, customer_key, order_date), where order_date >= start && order_date < end,
                        Customer(customer_key, nation_key), Nation(nation_key, region_key), Region(region_key, { asia }),
                        Supplier(supplier_key, nation_key);
                }
                .map(|(order_key, supplier_key, nation_key,)| (*order_key, *supplier_key, *nation_key,))
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
runner!($; tuned, {
    plan = ::cq_planners::order(written, fine);
    physical = [::cq_rewrites::canon_prefixes, ::cq_rewrites::smallest_intersections];
}, []);
// Close the Supplier/customer-nation cycle before expanding Nation/Region.
// Checked order advice keeps the written date-guard prerequisites intact.
runner!($; advised, { plan = ::mini_linq::passes::plan(order(Lineitem, Orders, Customer, Supplier, Nation, Region)); }, []);

// P3.2: explicit CQ steps for the ASIA-first R order. The final Supplier
// read binds no variable and therefore closes the nation/supplier cycle.
runner!($; step_r, {
    plan = ::mini_linq::passes::plan(no_reduce, order(Region, Nation, Customer, Orders, Lineitem, Supplier));
}, []);
runner!($; step_r_lookups, {
    plan = ::mini_linq::passes::plan(no_reduce, order(Region, Nation, Customer(nation_key), Supplier(nation_key), Customer, Orders, Lineitem, Supplier));
}, []);
runner!($; step_r_retained, {
    plan = ::mini_linq::passes::plan(no_reduce, order(Region, Nation, Customer, Orders, Lineitem, Supplier));
}, [::cq_rewrites::retain_lookups]);

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
        "generic_smallest" => tuned::run(data),
        "advised" => advised::run(data),
        "step_r" => step_r::run(data),
        "step_r_lookups" => step_r_lookups::run(data),
        "step_r_retained" => step_r_retained::run(data),
        _ => panic!("unknown configuration {}", config),
    }
}

pub fn canonical(rows: &[Answer]) -> Vec<Vec<String>> {
    rows.iter().map(|(order_key, supplier_key, nation_key,)| vec![order_key.to_string(), supplier_key.to_string(), nation_key.to_string()]).collect()
}
