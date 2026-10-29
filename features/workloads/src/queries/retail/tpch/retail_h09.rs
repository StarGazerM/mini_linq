// TPC-H-derived positive Q9 join core with the composite partsupp key.
// The canonical '%green%' pattern is exactly the literal substring predicate;
// profit arithmetic, grouping, and ordering remain outside this inspired core.
// Loading evaluates the string predicate once per distinct Part name and retains
// the resulting code relation to filter Part before the remaining joins.
// SF1: filtered Part has 10,664 rows; its Lineitem join has 319,404.
// This connected order keeps the large Lineitem build keyed only by part_key.
// The PartSupp-first candidate has smaller prefix work (777,440 vs 1,607,684)
// but builds Lineitem on a composite key; both are recorded in the Mac comparison.
// Orders before Lineitem would form 10,664 * 1,500,000 independent pairs.

// String-valued input columns stay dictionary-coded throughout execution.

pub const NAME: &str = "retail_h09";
pub const SQL: &str = r#"SELECT DISTINCT o.o_orderkey, p.p_partkey, s.s_suppkey
FROM lineitem l JOIN partsupp ps ON l.l_partkey = ps.ps_partkey AND l.l_suppkey = ps.ps_suppkey
JOIN part p ON p.p_partkey = l.l_partkey
JOIN supplier s ON s.s_suppkey = l.l_suppkey
JOIN orders o ON o.o_orderkey = l.l_orderkey
JOIN nation n ON n.n_nationkey = s.s_nationkey
WHERE p.p_name LIKE '%green%'"#;
pub const CONFIGS: &[&str] = &["default", "par", "radix", "radix_retained", "built_hash", "built_radix", "no-reduce", "transfer", "le", "le_runs", "generic_smallest", "advised"];
pub const EXPORTS: &[(&str, &str)] = &[
    ("lineitem", "l_orderkey,l_partkey,l_suppkey"),
    ("partsupp", "ps_partkey,ps_suppkey"),
    ("part", "p_partkey,p_name"),
    ("supplier", "s_suppkey,s_nationkey"),
    ("orders", "o_orderkey"),
    ("nation", "n_nationkey"),
];

pub type Lineitem = (i64, i64, i64,);
pub type PartSupp = (i64, i64,);
pub type Part = (i64, cq_load::StringCode,);
pub type Supplier = (i64, i32,);
pub type Orders = (i64,);
pub type Nation = (i32,);
pub type Answer = (i64, i64, i64,);
pub struct Data {
    pub lineitem: Vec<Lineitem>,
    pub partsupp: Vec<PartSupp>,
    pub part: Vec<Part>,
    pub supplier: Vec<Supplier>,
    pub orders: Vec<Orders>,
    pub nation: Vec<Nation>,
    pub green_names: Vec<(cq_load::StringCode,)>,
    pub dictionaries: cq_load::Dictionaries,
}

mini_linq::interned_relation_loaders! {
    relation Lineitem(l_orderkey: i64, l_partkey: i64, l_suppkey: i64);
    relation PartSupp(ps_partkey: i64, ps_suppkey: i64);
    relation Part(p_partkey: i64, p_name: cq_load::StringCode);
    relation Supplier(s_suppkey: i64, s_nationkey: i32);
    relation Orders(o_orderkey: i64);
    relation Nation(n_nationkey: i32);
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
        partsupp: <relation_loader!(PartSupp)>::parquet_interned(
            root.join("partsupp.parquet"),
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
        nation: <relation_loader!(Nation)>::parquet_interned(
            root.join("nation.parquet"),
            cq_load::ParquetOptions::default(),
            &mut dictionaries,
        )?,
        green_names: cq_load::dictionary_where(
            &dictionaries, "p_name", |value: &str| value.contains("green")),
        dictionaries,
    };
    Ok(data)
}

macro_rules! runner {
    ($d:tt; $module:ident, {$($stages:tt)*}, [$($extra:path),*]) => {
        crate::tpch_runtime::configuration!($module, $d, {
            $($stages)*
            select = ::mini_linq::passes::select(positional(Lineitem, PartSupp, Part, Supplier, Orders, Nation));
        }, [$($extra),*], {
            #[allow(non_snake_case)]
            pub(super) fn run(data: &Data) -> Vec<Answer> {
                let Lineitem = data.lineitem.as_slice();
                let PartSupp = data.partsupp.as_slice();
                let Part = data.part.as_slice();
                let Supplier = data.supplier.as_slice();
                let Orders = data.orders.as_slice();
                let Nation = data.nation.as_slice();
                let GreenNames = data.green_names.as_slice();
                query! {
                    struct Query;
                    relation Lineitem(l_orderkey: i64, l_partkey: i64, l_suppkey: i64);
                    relation PartSupp(ps_partkey: i64, ps_suppkey: i64);
                    relation Part(p_partkey: i64, p_name: cq_load::StringCode);
                    relation Supplier(s_suppkey: i64, s_nationkey: i32);
                    relation Orders(o_orderkey: i64);
                    relation Nation(n_nationkey: i32);
                    relation GreenNames(name: cq_load::StringCode);
                    relation answer(order_key: i64, part_key: i64, supplier_key: i64);
                    answer(order_key, part_key, supplier_key) :-
                        Part(part_key, part_name), GreenNames(part_name),
                        Lineitem(order_key, part_key, supplier_key), PartSupp(part_key, supplier_key),
                        Orders(order_key), Supplier(supplier_key, nation_key), Nation(nation_key);
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
runner!($; tuned, {
    plan = ::cq_planners::order(written, fine);
    physical = [::cq_rewrites::canon_prefixes, ::cq_rewrites::smallest_intersections];
}, []);
runner!($; advised, { plan = ::mini_linq::passes::plan(no_reduce, order(Part, GreenNames, Lineitem, PartSupp, Orders, Supplier, Nation)); }, []);

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
        _ => panic!("unknown configuration {}", config),
    }
}

pub fn canonical(rows: &[Answer]) -> Vec<Vec<String>> {
    rows.iter().map(|(order_key, part_key, supplier_key,)| vec![order_key.to_string(), part_key.to_string(), supplier_key.to_string()]).collect()
}
