// TPC-H-derived positive Q21 join core: a late Saudi supplier and a different
// supplier on a finalized order. The NOT EXISTS condition and counts of Q21
// are deliberately absent, so this query is only the named positive core.
use cq_load::Date;

// String-valued input columns stay dictionary-coded throughout execution.

pub const NAME: &str = "retail_h21_positive";
pub const SQL: &str = r#"SELECT DISTINCT l1.l_orderkey, s.s_suppkey, l2.l_suppkey
FROM supplier s JOIN lineitem l1 ON s.s_suppkey = l1.l_suppkey
JOIN orders o ON o.o_orderkey = l1.l_orderkey
JOIN nation n ON n.n_nationkey = s.s_nationkey
JOIN lineitem l2 ON l1.l_orderkey = l2.l_orderkey
WHERE n.n_name = 'SAUDI ARABIA' AND o.o_orderstatus = 'F'
AND l1.l_receiptdate > l1.l_commitdate AND l2.l_suppkey <> l1.l_suppkey"#;
pub const CONFIGS: &[&str] = &["default", "par", "radix", "radix_retained", "built_hash", "built_radix", "no-reduce", "transfer", "le", "le_runs", "generic_smallest", "advised"];
pub const EXPORTS: &[(&str, &str)] = &[
    ("supplier", "s_suppkey,s_nationkey"),
    ("lineitem", "l_orderkey,l_suppkey,l_receiptdate,l_commitdate"),
    ("orders", "o_orderkey,o_orderstatus"),
    ("nation", "n_nationkey,n_name"),
];

pub type Supplier = (i64, i32,);
pub type Lineitem = (i64, i64, Date, Date,);
pub type Orders = (i64, cq_load::StringCode,);
pub type Nation = (i32, cq_load::StringCode,);
pub type Answer = (i64, i64, i64,);
pub struct Data {
    pub supplier: Vec<Supplier>,
    pub lineitem: Vec<Lineitem>,
    pub orders: Vec<Orders>,
    pub nation: Vec<Nation>,
    pub dictionaries: cq_load::Dictionaries,
}

mini_linq::interned_relation_loaders! {
    relation Supplier(s_suppkey: i64, s_nationkey: i32);
    relation Lineitem(l_orderkey: i64, l_suppkey: i64, l_receiptdate: Date, l_commitdate: Date);
    relation Orders(o_orderkey: i64, o_orderstatus: cq_load::StringCode);
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
            select = ::mini_linq::passes::select(positional(Supplier, Lineitem, Orders, Nation));
        }, [$($extra),*], {
            #[allow(non_snake_case)]
            pub(super) fn run(data: &Data) -> Vec<Answer> {
                let Supplier = data.supplier.as_slice();
                let Lineitem = data.lineitem.as_slice();
                let Orders = data.orders.as_slice();
                let Nation = data.nation.as_slice();
                let dictionaries = &data.dictionaries;
                let saudi_arabia = dictionaries.literal("n_name", "SAUDI ARABIA");
                let finalized = dictionaries.literal("o_orderstatus", "F");
                query! {
                    struct Query;
                    relation Supplier(s_suppkey: i64, s_nationkey: i32);
                    relation Lineitem(l_orderkey: i64, l_suppkey: i64, l_receiptdate: Date, l_commitdate: Date);
                    relation Orders(o_orderkey: i64, o_orderstatus: cq_load::StringCode);
                    relation Nation(n_nationkey: i32, n_name: cq_load::StringCode);
                    relation answer(order_key: i64, waiting_supplier: i64, other_supplier: i64);
                    answer(order_key, waiting_supplier, other_supplier) :-
                        Lineitem(order_key, other_supplier, _other_receipt, _other_commit),
                        Lineitem(order_key, waiting_supplier, receipt_date, commit_date), where receipt_date > commit_date,
                        Supplier(waiting_supplier, nation_key),
                        Nation(nation_key, { saudi_arabia }), Orders(order_key, { finalized }),
                        where waiting_supplier != other_supplier;
                }
                .map(|(order_key, waiting_supplier, other_supplier,)| (*order_key, *waiting_supplier, *other_supplier,))
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
runner!($; advised, { plan = ::mini_linq::passes::plan(no_reduce, order(Nation, Supplier, Lineitem@1, Orders, Lineitem@0), reduce(Supplier with Nation), reduce(Lineitem@1 with Supplier)); }, []);

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
    rows.iter().map(|(order_key, waiting_supplier, other_supplier,)| vec![order_key.to_string(), waiting_supplier.to_string(), other_supplier.to_string()]).collect()
}
