//! Counter providers and the common typed shell for TPC-H-derived CQ kernels.
//! Query collection is timed; answer serialization and comparison are separate.

macro_rules! distinct_state {
    (operator $op:ident; new) => {
        ::cq_storage::counters::DistinctSet::new()
    };
    (operator $op:ident; insert($state:expr, $row:expr)) => {
        $state.insert(::core::clone::Clone::clone($row))
    };
}
macro_rules! index_provider {
    (probe) => { ::cq_storage::HashIndex<'_, _, _> };
    (semi) => { ::cq_storage::counters::HashSet<_> };
    (ordered) => { ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> };
    (trie $inner:ty) => { ::cq_storage::Trie<'_, _, $inner> };
}
pub(crate) use {distinct_state, index_provider};

/// One configuration expands the same typed CQ through the selected planner.
/// Providers are the existing deterministic counters, including deduplication.
macro_rules! configuration {
    ($module:ident, $d:tt, $planner:ident, { $($policy:tt)* }, { $($body:tt)* }) => {
        mod $module {
            use super::*;
            macro_rules! physical_policy {
                ($d($d tokens:tt)*) => {
                    ::mini_linq::lower_physical! { $d($d tokens)* policy { $($policy)* } }
                };
            }
            pub(crate) use physical_policy;
            mod wiring {
                pub(crate) use super::physical_policy;
                pub(crate) use crate::tpch_runtime::{distinct_state, index_provider};
                pub(crate) use ::mini_linq::{index_demands as choose_index_policy, select_structures, structures_to_rust};
                #[allow(unused_imports)]
                pub(crate) use ::mini_linq::{mini_linq as aggregate_hole, $planner as cq_to_ra};
            }
            use wiring as mini_linq;
            #[allow(unused_macros)]
            macro_rules! query {
                ($d($d tokens:tt)*) => { ::mini_linq::$planner! { $d($d tokens)* } };
            }
            $($body)*
        }
    };
}
pub(crate) use configuration;

macro_rules! candidate {
    (hypergraph; $d:tt; $($body:tt)*) => {
        crate::tpch_runtime::configuration!(tuned, $d, cq_to_ra_hypergraph, {}, { $($body)* });
    };
    (generic_smallest; $d:tt; $($body:tt)*) => {
        crate::tpch_runtime::configuration!(tuned, $d, cq_to_ra_generic,
            { project = canon; intersect = smallest; }, { $($body)* });
    };
}
pub(crate) use candidate;

/// Declares each loading schema once, then reuses those exact declarations in
/// both CQ configurations and in `relation_loaders!`.
macro_rules! retail_kernel {
    (
        $d:tt;
        name $name:literal;
        sql $sql:literal;
        candidate $candidate:ident;
        setup { $($setup:tt)* }
        relations {
            $(relation $relation:ident => $table:ident (
                $first:ident : $first_ty:ty $(, $column:ident : $ty:ty)* $(,)?
            );)*
        }
        answer($($out:ident : $out_ty:ty),+ $(,)?) :- $($body:tt)*
    ) => {
        pub const NAME: &str = $name;
        pub const SQL: &str = $sql;
        pub const CONFIGS: &[&str] = &["default", stringify!($candidate)];
        pub const EXPORTS: &[(&str, &str)] = &[$(
            (stringify!($table), concat!(stringify!($first) $(, ",", stringify!($column))*)),
        )*];
        pub struct Data { $(pub $table: Vec<($first_ty, $($ty,)*)>,)* }
        pub type Answer = ($($out_ty,)+);
        ::mini_linq::relation_loaders! {
            $(relation $relation($first: $first_ty $(, $column: $ty)*);)*
        }
        pub fn load(root: &std::path::Path) -> Result<Data, Box<dyn std::error::Error>> {
            Ok(Data { $($table: <relation_loader!($relation)>::parquet(
                root.join(concat!(stringify!($table), ".parquet")),
                ::cq_load::ParquetOptions::default(),
            )?,)* })
        }
        crate::tpch_runtime::configuration!(default, $d, cq_to_ra, {}, {
            #[allow(non_snake_case)]
            pub(super) fn run(data: &Data) -> Vec<Answer> {
                $(let $relation = data.$table.as_slice();)*
                $($setup)*
                query! {
                    struct Query;
                    $(relation $relation($first: $first_ty $(, $column: $ty)*);)*
                    answer($($out),+) :- $($body)*
                }.map(|($($out,)+)| ($(::core::clone::Clone::clone($out),)+)).collect()
            }
        });
        crate::tpch_runtime::candidate!($candidate; $d;
            #[allow(non_snake_case)]
            pub(super) fn run(data: &Data) -> Vec<Answer> {
                $(let $relation = data.$table.as_slice();)*
                $($setup)*
                query! {
                    struct Query;
                    $(relation $relation($first: $first_ty $(, $column: $ty)*);)*
                    answer($($out),+) :- $($body)*
                }.map(|($($out,)+)| ($(::core::clone::Clone::clone($out),)+)).collect()
            }
        );
        pub fn run(data: &Data, config: &str) -> Vec<Answer> {
            match config {
                "default" => default::run(data),
                stringify!($candidate) => tuned::run(data),
                _ => panic!("unknown configuration {config}"),
            }
        }
        pub fn canonical(rows: &[Answer]) -> Vec<Vec<String>> {
            rows.iter().map(|($($out,)+)| vec![$($out.to_string(),)+]).collect()
        }
    };
}
pub(crate) use retail_kernel;

pub(crate) fn date(value: &str) -> cq_load::Date {
    <cq_load::Date as cq_load::Load>::load(value).expect("authored ISO date")
}
