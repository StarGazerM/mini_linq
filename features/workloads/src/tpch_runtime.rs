//! Explicit pipelines and the common typed shell for TPC-H-derived CQ kernels.
//! Query collection is timed; answer serialization and comparison are separate.

#[cfg(feature = "counters")]
pub type Dedup<T> = cq_storage::counters::DistinctSet<T>;
#[cfg(not(feature = "counters"))]
pub type Dedup<T> = std::collections::HashSet<T>;

/// Expand the same authored CQ with an explicit sequence of public passes.
macro_rules! configuration {
    ($module:ident, $d:tt, { $($stages:tt)* }, { $($body:tt)* }) => {
        mod $module {
            use super::*;
            #[allow(unused_macros)]
            macro_rules! query {
                ($d($d tokens:tt)*) => { ::mini_linq::compile! {
                    pipeline {
                        $($stages)*
                        structures = [::cq_rewrites::retype(dedup => crate::tpch_runtime::Dedup<_>)];
                        surface = [::cq_ext_aggregate::aggregate, ::cq_ext_correlated::correlated,
                            ::cq_ext_negation::negation];
                    }
                    $d($d tokens)*
                } };
            }
            $($body)*
        }
    };
}
pub(crate) use configuration;

macro_rules! candidate {
    (hypergraph; $d:tt; $($body:tt)*) => {
        crate::tpch_runtime::configuration!(tuned, $d, { plan = ::cq_planners::order(hypergraph); }, { $($body)* });
    };
    (generic_smallest; $d:tt; $($body:tt)*) => {
        crate::tpch_runtime::configuration!(tuned, $d, {
            plan = ::cq_planners::order(written, fine);
            physical = [::cq_rewrites::canon_prefixes, ::cq_rewrites::smallest_intersections];
        }, { $($body)* });
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
        crate::tpch_runtime::configuration!(default, $d, {}, {
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
