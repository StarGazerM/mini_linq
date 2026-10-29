// TPC-H-derived re-authoring of the TPC-DS-inspired D27 fact/dimension join.
// Roles: store_sales -> lineitem; date_dim -> orders and its date;
// item -> part; store -> supplier; customer_demographics -> supplier's nation.
// Nation is an actual dbgen relation, not simulated demographic data.
use crate::tpch_runtime::date;
use cq_load::Date;

// String-valued input columns stay dictionary-coded throughout execution.

pub const NAME: &str = "retail_d27";
pub const SQL: &str = r#"SELECT DISTINCT o.o_orderkey, p.p_partkey, s.s_suppkey
FROM lineitem l JOIN orders o ON l.l_orderkey = o.o_orderkey
JOIN part p ON l.l_partkey = p.p_partkey
JOIN supplier s ON l.l_suppkey = s.s_suppkey
JOIN nation n ON s.s_nationkey = n.n_nationkey
WHERE o.o_orderdate >= DATE '1995-01-01' AND o.o_orderdate < DATE '1995-04-01'
AND p.p_brand = 'Brand#12' AND n.n_name = 'UNITED STATES'"#;
pub const CONFIGS: &[&str] = &["default", "par", "no-reduce", "transfer", "le", "le_runs", "hypergraph", "advised"];
pub const EXPORTS: &[(&str, &str)] = &[
    ("lineitem", "l_orderkey,l_partkey,l_suppkey"),
    ("orders", "o_orderkey,o_orderdate"),
    ("part", "p_partkey,p_brand"),
    ("supplier", "s_suppkey,s_nationkey"),
    ("nation", "n_nationkey,n_name"),
];

pub type Lineitem = (i64, i64, i64,);
pub type Orders = (i64, Date,);
pub type Part = (i64, cq_load::StringCode,);
pub type Supplier = (i64, i32,);
pub type Nation = (i32, cq_load::StringCode,);
pub type Answer = (i64, i64, i64,);
pub struct Data {
    pub lineitem: Vec<Lineitem>,
    pub orders: Vec<Orders>,
    pub part: Vec<Part>,
    pub supplier: Vec<Supplier>,
    pub nation: Vec<Nation>,
    pub dictionaries: cq_load::Dictionaries,
}

mini_linq::interned_relation_loaders! {
    relation Lineitem(l_orderkey: i64, l_partkey: i64, l_suppkey: i64);
    relation Orders(o_orderkey: i64, o_orderdate: Date);
    relation Part(p_partkey: i64, p_brand: cq_load::StringCode);
    relation Supplier(s_suppkey: i64, s_nationkey: i32);
    relation Nation(n_nationkey: i32, n_name: cq_load::StringCode);
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
        dictionaries,
    };
    Ok(data)
}

macro_rules! runner {
    ($d:tt; $module:ident, {$($stages:tt)*}, [$($extra:path),*]) => {
        crate::tpch_runtime::configuration!($module, $d, {
            $($stages)*
            select = ::mini_linq::passes::select(positional(Lineitem, Orders, Part, Supplier, Nation));
        }, [$($extra),*], {
            #[allow(non_snake_case)]
            pub(super) fn run(data: &Data) -> Vec<Answer> {
                let Lineitem = data.lineitem.as_slice();
                let Orders = data.orders.as_slice();
                let Part = data.part.as_slice();
                let Supplier = data.supplier.as_slice();
                let Nation = data.nation.as_slice();
                let dictionaries = &data.dictionaries;
                let start = date("1995-01-01"); let end = date("1995-04-01");
                let brand12 = dictionaries.literal("p_brand", "Brand#12");
                let united_states = dictionaries.literal("n_name", "UNITED STATES");
                query! {
                    struct Query;
                    relation Lineitem(l_orderkey: i64, l_partkey: i64, l_suppkey: i64);
                    relation Orders(o_orderkey: i64, o_orderdate: Date);
                    relation Part(p_partkey: i64, p_brand: cq_load::StringCode);
                    relation Supplier(s_suppkey: i64, s_nationkey: i32);
                    relation Nation(n_nationkey: i32, n_name: cq_load::StringCode);
                    relation answer(order_key: i64, part_key: i64, supplier_key: i64);
                    answer(order_key, part_key, supplier_key) :-
                        Lineitem(order_key, part_key, supplier_key),
                        Orders(order_key, order_date), where order_date >= start && order_date < end,
                        Part(part_key, { brand12 }),
                        Supplier(supplier_key, nation_key), Nation(nation_key, { united_states });
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
runner!($; advised, { plan = ::mini_linq::passes::plan(no_reduce, order(Nation, Supplier, Lineitem, Orders, Part), reduce(Supplier with Nation), reduce(Lineitem with Supplier), reduce(Lineitem with Orders), reduce(Lineitem with Part)); }, []);

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
