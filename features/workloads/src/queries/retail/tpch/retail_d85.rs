// TPC-H-derived re-authoring of the TPC-DS-inspired D85 composite-key core.
// Roles: web_sales -> lineitem/orders; web_returns -> partsupp on (part,supplier),
// web_page -> part; the two demographic aliases -> customer and supplier;
// customer_address/reason -> their two nation aliases; date_dim -> orders' date.
// dbgen has no return/reason relation: those roles are explicitly repurposed as
// supplier availability and supplier geography, not fabricated return records.
use crate::tpch_runtime::date;
use cq_load::Date;

// String-valued input columns stay dictionary-coded throughout execution.

pub const NAME: &str = "retail_d85";
pub const SQL: &str = r#"SELECT DISTINCT o.o_orderkey, p.p_partkey, ns.n_nationkey
FROM lineitem l JOIN partsupp ps ON l.l_partkey = ps.ps_partkey AND l.l_suppkey = ps.ps_suppkey
JOIN part p ON p.p_partkey = l.l_partkey
JOIN orders o ON o.o_orderkey = l.l_orderkey
JOIN customer c ON c.c_custkey = o.o_custkey
JOIN supplier s ON s.s_suppkey = ps.ps_suppkey
JOIN nation nc ON nc.n_nationkey = c.c_nationkey
JOIN nation ns ON ns.n_nationkey = s.s_nationkey
WHERE o.o_orderdate >= DATE '1995-01-01' AND o.o_orderdate < DATE '1995-04-01'
AND nc.n_name = 'UNITED STATES' AND ps.ps_availqty > 0"#;
pub const CONFIGS: &[&str] = &["default", "par", "radix", "radix_retained", "built_hash", "built_radix", "no-reduce", "transfer", "le", "le_runs", "generic_smallest", "advised", "hypergraph", "hypergraph_zone"];
pub const EXPORTS: &[(&str, &str)] = &[
    ("lineitem", "l_orderkey,l_partkey,l_suppkey"),
    ("partsupp", "ps_partkey,ps_suppkey,ps_availqty"),
    ("part", "p_partkey"),
    ("orders", "o_orderkey,o_custkey,o_orderdate"),
    ("customer", "c_custkey,c_nationkey"),
    ("supplier", "s_suppkey,s_nationkey"),
    ("nation", "n_nationkey,n_name"),
];

pub type Lineitem = (i64, i64, i64,);
pub type PartSupp = (i64, i64, i64,);
pub type Part = (i64,);
pub type Orders = (i64, i64, Date,);
pub type Customer = (i64, i32,);
pub type Supplier = (i64, i32,);
pub type Nation = (i32, cq_load::StringCode,);
pub type Answer = (i64, i64, i32,);
pub struct Data {
    pub lineitem: Vec<Lineitem>,
    pub partsupp: Vec<PartSupp>,
    pub part: Vec<Part>,
    pub orders: Vec<Orders>,
    pub customer: Vec<Customer>,
    pub supplier: Vec<Supplier>,
    pub nation: Vec<Nation>,
    pub dictionaries: cq_load::Dictionaries,
    hypergraph: Option<ZoneData>,
}

mini_linq::interned_relation_loaders! {
    relation Lineitem(l_orderkey: i64, l_partkey: i64, l_suppkey: i64);
    relation PartSupp(ps_partkey: i64, ps_suppkey: i64, ps_availqty: i64);
    relation Part(p_partkey: i64);
    relation Orders(o_orderkey: i64, o_custkey: i64, o_orderdate: Date);
    relation Customer(c_custkey: i64, c_nationkey: i32);
    relation Supplier(s_suppkey: i64, s_nationkey: i32);
    relation Nation(n_nationkey: i32, n_name: cq_load::StringCode);
}

pub fn load(root: &std::path::Path, config: &str) -> Result<Data, Box<dyn std::error::Error>> {
    let mut dictionaries = cq_load::Dictionaries::default();
    let mut data = Data {
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
        hypergraph: None,
    };
    if config == "hypergraph_zone" {
        prepare_zones(&mut data);
    }
    Ok(data)
}

macro_rules! runner {
    ($d:tt; $module:ident, {$($stages:tt)*}, [$($extra:path),*]) => {
        crate::tpch_runtime::configuration!($module, $d, {
            $($stages)*
            select = ::mini_linq::passes::select(positional(Lineitem, PartSupp, Part, Orders, Customer, Supplier, Nation));
        }, [$($extra),*], {
            #[allow(non_snake_case)]
            pub(super) fn run(data: &Data) -> Vec<Answer> {
                let Lineitem = data.lineitem.as_slice();
                let PartSupp = data.partsupp.as_slice();
                let Part = data.part.as_slice();
                let Orders = data.orders.as_slice();
                let Customer = data.customer.as_slice();
                let Supplier = data.supplier.as_slice();
                let Nation = data.nation.as_slice();
                let dictionaries = &data.dictionaries;
                let start = date("1995-01-01"); let end = date("1995-04-01");
                let united_states = dictionaries.literal("n_name", "UNITED STATES");
                query! {
                    struct Query;
                    relation Lineitem(l_orderkey: i64, l_partkey: i64, l_suppkey: i64);
                    relation PartSupp(ps_partkey: i64, ps_suppkey: i64, ps_availqty: i64);
                    relation Part(p_partkey: i64);
                    relation Orders(o_orderkey: i64, o_custkey: i64, o_orderdate: Date);
                    relation Customer(c_custkey: i64, c_nationkey: i32);
                    relation Supplier(s_suppkey: i64, s_nationkey: i32);
                    relation Nation(n_nationkey: i32, n_name: cq_load::StringCode);
                    relation answer(order_key: i64, part_key: i64, supplier_nation: i32);
                    answer(order_key, part_key, supplier_nation) :-
                        PartSupp(part_key, supplier_key, available), where available > 0,
                        Lineitem(order_key, part_key, supplier_key),
                        Orders(order_key, customer_key, order_date), where order_date >= start && order_date < end,
                        Customer(customer_key, customer_nation), Nation(customer_nation, { united_states }),
                        Part(part_key), Supplier(supplier_key, supplier_nation), Nation(supplier_nation, _supplier_name);
                }
                .map(|(order_key, part_key, supplier_nation,)| (*order_key, *part_key, *supplier_nation,))
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
runner!($; advised, { plan = ::mini_linq::passes::plan(no_reduce, order(Nation@7, Customer, Orders, Lineitem, Part, PartSupp, Supplier, Nation@10)); }, []);

pub struct ZoneData {
    pub lineitem: cq_load::ZoneRows<Lineitem, (cq_load::MinMax<i64>, cq_load::MinMax<i64>, cq_load::MinMax<i64>,)>,
    pub partsupp: cq_load::ZoneRows<PartSupp, (cq_load::MinMax<i64>, cq_load::MinMax<i64>, cq_load::MinMax<i64>,)>,
    pub part: cq_load::ZoneRows<Part, (cq_load::MinMax<i64>,)>,
    pub orders: cq_load::ZoneRows<Orders, (cq_load::MinMax<i64>, cq_load::MinMax<i64>, cq_load::MinMax<Date>,)>,
    pub customer: cq_load::ZoneRows<Customer, (cq_load::MinMax<i64>, cq_load::MinMax<i32>,)>,
    pub supplier: cq_load::ZoneRows<Supplier, (cq_load::MinMax<i64>, cq_load::MinMax<i32>,)>,
    pub nation: cq_load::ZoneRows<Nation, (cq_load::MinMax<i32>, cq_load::MinMax<cq_load::StringCode>,)>,
}

fn prepare_zones(data: &mut Data) {
    let start = std::time::Instant::now();
    let lineitem = cq_load::ZoneRows::from_rows::<(cq_load::MinMax<i64>, cq_load::MinMax<i64>, cq_load::MinMax<i64>,)>(
        std::mem::take(&mut data.lineitem), std::num::NonZeroUsize::new(2048).unwrap());
    let partsupp = cq_load::ZoneRows::from_rows::<(cq_load::MinMax<i64>, cq_load::MinMax<i64>, cq_load::MinMax<i64>,)>(
        std::mem::take(&mut data.partsupp), std::num::NonZeroUsize::new(2048).unwrap());
    let part = cq_load::ZoneRows::from_rows::<(cq_load::MinMax<i64>,)>(
        std::mem::take(&mut data.part), std::num::NonZeroUsize::new(2048).unwrap());
    let orders = cq_load::ZoneRows::from_rows::<(cq_load::MinMax<i64>, cq_load::MinMax<i64>, cq_load::MinMax<Date>,)>(
        std::mem::take(&mut data.orders), std::num::NonZeroUsize::new(2048).unwrap());
    let customer = cq_load::ZoneRows::from_rows::<(cq_load::MinMax<i64>, cq_load::MinMax<i32>,)>(
        std::mem::take(&mut data.customer), std::num::NonZeroUsize::new(2048).unwrap());
    let supplier = cq_load::ZoneRows::from_rows::<(cq_load::MinMax<i64>, cq_load::MinMax<i32>,)>(
        std::mem::take(&mut data.supplier), std::num::NonZeroUsize::new(2048).unwrap());
    let nation = cq_load::ZoneRows::from_rows::<(cq_load::MinMax<i32>, cq_load::MinMax<cq_load::StringCode>,)>(
        std::mem::take(&mut data.nation), std::num::NonZeroUsize::new(2048).unwrap());
    let bytes = lineitem.metadata_bytes() + partsupp.metadata_bytes() + part.metadata_bytes() + orders.metadata_bytes() + customer.metadata_bytes() + supplier.metadata_bytes() + nation.metadata_bytes();
    data.hypergraph = Some(ZoneData { lineitem, partsupp, part, orders, customer, supplier, nation });
    crate::tpch_runtime::record_zones(start, bytes);
}

runner!($; hypergraph_extra, { plan = ::cq_planners::order(hypergraph); }, []);
crate::tpch_runtime::configuration!(hypergraph_zone, $, {
    plan = ::cq_planners::order(hypergraph);
    select = ::mini_linq::passes::select(positional(Lineitem, PartSupp, Part, Orders, Customer, Supplier, Nation));
}, {
    #[allow(non_snake_case)]
    pub(super) fn run(data: &Data) -> Vec<Answer> {
        let zones = data.hypergraph.as_ref().expect("configuration loaded ZoneRows");
        let Lineitem = &zones.lineitem;
        let PartSupp = &zones.partsupp;
        let Part = &zones.part;
        let Orders = &zones.orders;
        let Customer = &zones.customer;
        let Supplier = &zones.supplier;
        let Nation = &zones.nation;
        let dictionaries = &data.dictionaries;
                let start = date("1995-01-01"); let end = date("1995-04-01");
                let united_states = dictionaries.literal("n_name", "UNITED STATES");
                query! {
                    struct Query;
                    relation Lineitem(l_orderkey: i64, l_partkey: i64, l_suppkey: i64);
                    relation PartSupp(ps_partkey: i64, ps_suppkey: i64, ps_availqty: i64);
                    relation Part(p_partkey: i64);
                    relation Orders(o_orderkey: i64, o_custkey: i64, o_orderdate: Date);
                    relation Customer(c_custkey: i64, c_nationkey: i32);
                    relation Supplier(s_suppkey: i64, s_nationkey: i32);
                    relation Nation(n_nationkey: i32, n_name: cq_load::StringCode);
                    relation answer(order_key: i64, part_key: i64, supplier_nation: i32);
                    answer(order_key, part_key, supplier_nation) :-
                        PartSupp(part_key, supplier_key, available), where available > 0,
                        Lineitem(order_key, part_key, supplier_key),
                        Orders(order_key, customer_key, order_date), where order_date >= start && order_date < end,
                        Customer(customer_key, customer_nation), Nation(customer_nation, { united_states }),
                        Part(part_key), Supplier(supplier_key, supplier_nation), Nation(supplier_nation, _supplier_name);
                }
                .map(|(order_key, part_key, supplier_nation,)| (*order_key, *part_key, *supplier_nation,))
                .collect()
    }
});

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
        "hypergraph" => hypergraph_extra::run(data),
        "hypergraph_zone" => hypergraph_zone::run(data),
        _ => panic!("unknown configuration {}", config),
    }
}

pub fn canonical(rows: &[Answer]) -> Vec<Vec<String>> {
    rows.iter().map(|(order_key, part_key, supplier_nation,)| vec![order_key.to_string(), part_key.to_string(), supplier_nation.to_string()]).collect()
}
