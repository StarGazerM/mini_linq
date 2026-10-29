// TPC-H-derived positive Q8 join core. The market-share ratio is not this kernel.
use crate::tpch_runtime::date;
use cq_load::Date;

// String-valued input columns stay dictionary-coded throughout execution.

pub const NAME: &str = "retail_h08";
pub const SQL: &str = r#"SELECT DISTINCT o.o_orderkey, p.p_partkey, s.s_suppkey
FROM lineitem l JOIN part p ON l.l_partkey = p.p_partkey
JOIN supplier s ON l.l_suppkey = s.s_suppkey
JOIN orders o ON l.l_orderkey = o.o_orderkey
JOIN customer c ON o.o_custkey = c.c_custkey
JOIN nation nc ON c.c_nationkey = nc.n_nationkey
JOIN region r ON nc.n_regionkey = r.r_regionkey
JOIN nation ns ON s.s_nationkey = ns.n_nationkey
WHERE r.r_name = 'AMERICA' AND p.p_type = 'ECONOMY ANODIZED STEEL'
AND o.o_orderdate >= DATE '1995-01-01' AND o.o_orderdate <= DATE '1996-12-31'"#;
pub const CONFIGS: &[&str] = &["default", "par", "no-reduce", "transfer", "le", "le_runs", "hypergraph", "advised"];
pub const EXPORTS: &[(&str, &str)] = &[
    ("lineitem", "l_orderkey,l_partkey,l_suppkey"),
    ("part", "p_partkey,p_type"),
    ("supplier", "s_suppkey,s_nationkey"),
    ("orders", "o_orderkey,o_custkey,o_orderdate"),
    ("customer", "c_custkey,c_nationkey"),
    ("nation", "n_nationkey,n_regionkey"),
    ("region", "r_regionkey,r_name"),
];

pub type Lineitem = (i64, i64, i64,);
pub type Part = (i64, cq_load::StringCode,);
pub type Supplier = (i64, i32,);
pub type Orders = (i64, i64, Date,);
pub type Customer = (i64, i32,);
pub type Nation = (i32, i32,);
pub type Region = (i32, cq_load::StringCode,);
pub type Answer = (i64, i64, i64,);
pub struct Data {
    pub lineitem: Vec<Lineitem>,
    pub part: Vec<Part>,
    pub supplier: Vec<Supplier>,
    pub orders: Vec<Orders>,
    pub customer: Vec<Customer>,
    pub nation: Vec<Nation>,
    pub region: Vec<Region>,
    pub dictionaries: cq_load::Dictionaries,
}

mini_linq::interned_relation_loaders! {
    relation Lineitem(l_orderkey: i64, l_partkey: i64, l_suppkey: i64);
    relation Part(p_partkey: i64, p_type: cq_load::StringCode);
    relation Supplier(s_suppkey: i64, s_nationkey: i32);
    relation Orders(o_orderkey: i64, o_custkey: i64, o_orderdate: Date);
    relation Customer(c_custkey: i64, c_nationkey: i32);
    relation Nation(n_nationkey: i32, n_regionkey: i32);
    relation Region(r_regionkey: i32, r_name: cq_load::StringCode);
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
        part: <relation_loader!(Part)>::parquet_interned(
            root.join("part.parquet"),
            cq_load::ParquetOptions::default(),
            &mut dictionaries,
        )?,
        supplier: <relation_loader!(Supplier)>::parquet_interned(
            root.join("supplier.parquet"),
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
            select = ::mini_linq::passes::select(positional(Lineitem, Part, Supplier, Orders, Customer, Nation, Region));
        }, [$($extra),*], {
            #[allow(non_snake_case)]
            pub(super) fn run(data: &Data) -> Vec<Answer> {
                let Lineitem = data.lineitem.as_slice();
                let Part = data.part.as_slice();
                let Supplier = data.supplier.as_slice();
                let Orders = data.orders.as_slice();
                let Customer = data.customer.as_slice();
                let Nation = data.nation.as_slice();
                let Region = data.region.as_slice();
                let dictionaries = &data.dictionaries;
                let start = date("1995-01-01"); let end = date("1996-12-31");
                let america = dictionaries.literal("r_name", "AMERICA");
                let economy_anodized_steel = dictionaries.literal("p_type", "ECONOMY ANODIZED STEEL");
                query! {
                    struct Query;
                    relation Lineitem(l_orderkey: i64, l_partkey: i64, l_suppkey: i64);
                    relation Part(p_partkey: i64, p_type: cq_load::StringCode);
                    relation Supplier(s_suppkey: i64, s_nationkey: i32);
                    relation Orders(o_orderkey: i64, o_custkey: i64, o_orderdate: Date);
                    relation Customer(c_custkey: i64, c_nationkey: i32);
                    relation Nation(n_nationkey: i32, n_regionkey: i32);
                    relation Region(r_regionkey: i32, r_name: cq_load::StringCode);
                    relation answer(order_key: i64, part_key: i64, supplier_key: i64);
                    answer(order_key, part_key, supplier_key) :-
                        Lineitem(order_key, part_key, supplier_key),
                        Part(part_key, { economy_anodized_steel }),
                        Supplier(supplier_key, supplier_nation),
                        Orders(order_key, customer_key, order_date), where order_date >= start && order_date <= end,
                        Customer(customer_key, customer_nation), Nation(customer_nation, customer_region),
                        Region(customer_region, { america }),
                        Nation(supplier_nation, _supplier_region);
                }
                .map(|(order_key, part_key, supplier_key,)| (*order_key, *part_key, *supplier_key,))
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
runner!($; advised, { plan = ::mini_linq::passes::plan(no_reduce, order(Part, Lineitem, Orders, Customer, Nation@6, Region, Supplier, Nation@8)); }, []);

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
    rows.iter().map(|(order_key, part_key, supplier_key,)| vec![order_key.to_string(), part_key.to_string(), supplier_key.to_string()]).collect()
}
