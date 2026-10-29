//! TPC-H Q16 part/supplier core, without grouped supplier counts or ordering.
//! dbgen keys are non-null, so SQL NOT IN equals the CQ anti-join below.
//! Dictionary predicates select kinds/comments; a CQ join derives BadSupplier.
//! SQL is the independent DuckDB oracle; LIKE preserves substring order.
//! String predicates are prepared against sorted dictionaries at load time.
use cq_load::StringCode;

pub const NAME: &str = "retail_h16";
pub const CONFIGS: &[&str] = &[
    "default", "par", "no-reduce", "transfer", "le", "le_runs", "hypergraph", "advised",
];
pub const EXPORTS: &[(&str, &str)] = &[
    ("part", "p_partkey,p_brand,p_type,p_size"),
    ("partsupp", "ps_partkey,ps_suppkey"),
    ("supplier", "s_suppkey,s_comment"),
];
pub const SQL: &str = r#"SELECT DISTINCT p.p_partkey, ps.ps_suppkey
FROM part p JOIN partsupp ps ON ps.ps_partkey = p.p_partkey
WHERE p.p_brand <> 'Brand#45' AND p.p_type NOT LIKE 'MEDIUM POLISHED%'
AND p.p_size IN (49, 14, 23, 45, 19, 3, 36, 9)
AND ps.ps_suppkey NOT IN (SELECT s.s_suppkey FROM supplier s
WHERE s.s_comment LIKE '%Customer%Complaints%')"#;

pub type Part = (i64, StringCode, StringCode, i32);
pub type Partsupp = (i64, i64);
pub type Supplier = (i64, StringCode);
pub type Answer = (i64, i64);
pub struct Data {
    pub part: Vec<Part>,
    pub partsupp: Vec<Partsupp>,
    pub supplier: Vec<Supplier>,
    pub allowed_kinds: Vec<(StringCode,)>,
    pub bad_comments: Vec<(StringCode,)>,
    pub dictionaries: cq_load::Dictionaries,
}

mini_linq::interned_relation_loaders! {
    relation Part(p_partkey: i64, p_brand: StringCode, p_type: StringCode, p_size: i32);
    relation Partsupp(ps_partkey: i64, ps_suppkey: i64);
    relation Supplier(s_suppkey: i64, s_comment: StringCode);
}

pub fn load(root: &std::path::Path, _config: &str) -> Result<Data, Box<dyn std::error::Error>> {
    let mut dictionaries = cq_load::Dictionaries::default();
    Ok(Data {
        part: <relation_loader!(Part)>::parquet_interned(
            root.join("part.parquet"),
            cq_load::ParquetOptions::default(),
            &mut dictionaries,
        )?,
        partsupp: <relation_loader!(Partsupp)>::parquet_interned(
            root.join("partsupp.parquet"),
            cq_load::ParquetOptions::default(),
            &mut dictionaries,
        )?,
        supplier: <relation_loader!(Supplier)>::parquet_interned(
            root.join("supplier.parquet"),
            cq_load::ParquetOptions::default(),
            &mut dictionaries,
        )?,
        allowed_kinds: cq_load::dictionary_where(
            &dictionaries, "p_type", |value: &str| !value.starts_with("MEDIUM POLISHED")),
        bad_comments: cq_load::dictionary_where(
            &dictionaries, "s_comment", |value: &str| value.split_once("Customer")
                .is_some_and(|(_, rest)| rest.contains("Complaints"))),
        dictionaries,
    })
}

macro_rules! query_body {
    () => {
    pub(super) fn run(data: &Data) -> Vec<Answer> {
        let Part = data.part.as_slice();
        let Partsupp = data.partsupp.as_slice();
        let Supplier = data.supplier.as_slice();
        let AllowedKinds = data.allowed_kinds.as_slice();
        let BadComments = data.bad_comments.as_slice();
        let excluded_brand = data.dictionaries.literal("p_brand", "Brand#45");
        let BadSupplier: Vec<(i64,)> = query! {
            plan(no_reduce, order(Supplier, BadComments));
            struct BadSuppliers;
            relation Supplier(s_suppkey: i64, s_comment: StringCode);
            relation BadComments(comment: StringCode);
            relation answer(suppkey: i64);
            answer(suppkey) :- Supplier(suppkey, comment), BadComments(comment);
        }.map(|(suppkey,)| (*suppkey,)).collect();
        let BadSupplier = BadSupplier.as_slice();
        query! {
            plan(no_reduce, order(Part, AllowedKinds, Partsupp));
            struct Query;
            relation Part(p_partkey: i64, p_brand: StringCode, p_type: StringCode, p_size: i32);
            relation Partsupp(ps_partkey: i64, ps_suppkey: i64);
            relation AllowedKinds(kind: StringCode);
            relation BadSupplier(suppkey: i64);
            relation answer(part_key: i64, suppkey: i64);
            answer(part_key, suppkey) :-
                Part(part_key, brand, kind, size), AllowedKinds(kind),
                where brand != excluded_brand,
                if [49, 14, 23, 45, 19, 3, 36, 9].contains(size),
                Partsupp(part_key, suppkey), !BadSupplier(suppkey);
        }.map(|(part_key, suppkey)| (*part_key, *suppkey)).collect()
    }
    };
}

// Keep positional selection stage-local: dictionary membership and derived
// supplier keys are not scan hosts. The authored query syntax also remains
// available unchanged to importers.
macro_rules! h16_configuration {
    ($module:ident, $d:tt, { $($stages:tt)* }, [$($extra:path),*], { $($body:tt)* }) => {
        #[allow(non_snake_case)]
        mod $module {
            use super::*;
            macro_rules! query {
                (plan($d($d advice:tt)*); struct BadSuppliers; $d($d tokens:tt)*) => {
                    crate::tpch_runtime::standalone_query! {
                        $module; [$d($d advice)*]; {
                            $($stages)*
                            select = ::mini_linq::passes::select(positional(Supplier));
                        }; [$($extra),*];
                        struct BadSuppliers;
                        $d($d tokens)*
                    }
                };
                (plan($d($d advice:tt)*); struct Query; $d($d tokens:tt)*) => {
                    crate::tpch_runtime::standalone_query! {
                        $module; [$d($d advice)*]; {
                            $($stages)*
                            select = ::mini_linq::passes::select(positional(Part, Partsupp));
                        }; [$($extra),*];
                        struct Query;
                        $d($d tokens)*
                    }
                };
            }
            $($body)*
        }
    };
}

h16_configuration!(default, $, {}, [], { query_body!(); });
h16_configuration!(par, $, {
    decompose = [::cq_sequence_tasks_macros::plan];
    grade = ::cq_sequence_tasks_macros::tasks;
    print = ::cq_sequence_tasks_macros::translate_tasks;
}, [], { query_body!(); });
h16_configuration!(no_reduce, $, {
    plan = ::mini_linq::passes::plan(no_reduce);
}, [], { query_body!(); });
h16_configuration!(transfer, $, {
    plan = ::cq_planners::order(transfer);
}, [], { query_body!(); });
h16_configuration!(le, $, {
    plan = ::cq_planners::order(le);
}, [], { query_body!(); });
h16_configuration!(le_runs, $, {
    plan = ::cq_planners::order(le);
}, [::cq_rewrites::sorted_runs, ::cq_rewrites::retain_lookups], { query_body!(); });
h16_configuration!(tuned, $, {
    plan = ::cq_planners::order(hypergraph);
}, [], { query_body!(); });
h16_configuration!(advised, $, {}, [], { query_body!(); });

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
    rows.iter()
        .map(|(part_key, suppkey)| vec![part_key.to_string(), suppkey.to_string()])
        .collect()
}
