//! Explicit pipelines and the common typed shell for TPC-H-derived CQ kernels.
//! Query collection is timed; answer serialization and comparison are separate.

#[cfg(feature = "counters")]
pub type Dedup<T> = cq_storage::counters::DistinctSet<T>;
// Keep the recorded measurement alias aligned with the default provider.
// The counter build retains its explicit deterministic instrumentation state.
#[cfg(not(feature = "counters"))]
pub type Dedup<T> = cq_storage::HashSet<T>;

/// Expand the same authored CQ with an explicit sequence of public passes.
macro_rules! configuration {
    ($module:ident, $d:tt, { $($stages:tt)* }, { $($body:tt)* }) => {
        mod $module {
            use super::*;
            #[allow(unused_macros)]
            macro_rules! query {
                (positional($d($d host:ident),*); $d($d tokens:tt)*) => { ::mini_linq::compile! {
                    pipeline {
                        $($stages)*
                        select = ::mini_linq::passes::select(positional($d($d host),*));
                        structures = [::cq_rewrites::retype(dedup => crate::tpch_runtime::Dedup<_>)];
                        surface = [::cq_ext_aggregate::aggregate, ::cq_ext_correlated::correlated,
                            ::cq_ext_negation::negation];
                    }
                    $d($d tokens)*
                } };
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
    (hypergraph; $d:tt; positional($($host:ident),*); $($body:tt)*) => {
        crate::tpch_runtime::configuration!(tuned, $d, { plan = ::cq_planners::order(hypergraph); select = ::mini_linq::passes::select(positional($($host),*)); }, { $($body)* });
    };
    (generic_smallest; $d:tt; positional($($host:ident),*); $($body:tt)*) => {
        crate::tpch_runtime::configuration!(tuned, $d, {
            plan = ::cq_planners::order(written, fine);
            select = ::mini_linq::passes::select(positional($($host),*));
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
        $(extra $extra:ident;)?
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
        pub const CONFIGS: &[&str] = &["default", stringify!($candidate) $(, stringify!($extra), concat!(stringify!($extra), "_zone"))?];
        pub const EXPORTS: &[(&str, &str)] = &[$(
            (stringify!($table), concat!(stringify!($first) $(, ",", stringify!($column))*)),
        )*];
        pub struct Data { $(pub $table: Vec<($first_ty, $($ty,)*)>,)* $($extra: Option<ZoneData>,)? }
        pub type Answer = ($($out_ty,)+);
        ::mini_linq::relation_loaders! {
            $(relation $relation($first: $first_ty $(, $column: $ty)*);)*
        }
        pub fn load(root: &std::path::Path, config: &str) -> Result<Data, Box<dyn std::error::Error>> {
            let data = Data { $($table: <relation_loader!($relation)>::parquet(
                root.join(concat!(stringify!($table), ".parquet")),
                ::cq_load::ParquetOptions::default(),
            )?,)* $($extra: None,)? };
            Ok(crate::tpch_runtime::extra_prepare!([$($extra)?]; data, config))
        }
        crate::tpch_runtime::configuration!(default, $d, { select = ::mini_linq::passes::select(positional($($relation),*)); }, {
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
        crate::tpch_runtime::candidate!($candidate; $d; positional($($relation),*);
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
        crate::tpch_runtime::extra_zone!([$($extra)?]; $d;
            setup { $($setup)* }
            relations { $(relation $relation => $table ($first:$first_ty $(,$column:$ty)*);)* }
            answer($($out),+) :- $($body)*
        );
        pub fn run(data: &Data, config: &str) -> Vec<Answer> {
            match config {
                "default" => default::run(data),
                stringify!($candidate) => tuned::run(data),
                _ => crate::tpch_runtime::extra_run!([$($extra)?]; data, config),
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

thread_local! {
    static ZONE_COST: std::cell::Cell<(f64, usize)> = const { std::cell::Cell::new((0.0, 0)) };
}
pub fn zone_metrics() -> (f64, usize) { ZONE_COST.with(std::cell::Cell::get) }
pub fn record_zones(start: std::time::Instant, bytes: usize) {
    ZONE_COST.with(|c| c.set((start.elapsed().as_secs_f64() * 1000.0, bytes)));
}
macro_rules! extra_run {
    ([]; $data:expr, $config:expr) => { panic!("unknown configuration {}", $config) };
    ([hypergraph]; $data:expr, $config:expr) => { match $config {
        "hypergraph" => hypergraph_extra::run($data),
        "hypergraph_zone" => hypergraph_zone::run($data),
        _ => panic!("unknown configuration {}", $config),
    } };
}
pub(crate) use extra_run;
macro_rules! extra_zone {
    ([]; $($rest:tt)*) => {};
    ([hypergraph]; $d:tt;
     setup { $($setup:tt)* }
     relations { $(relation $relation:ident => $table:ident ($first:ident:$first_ty:ty $(,$column:ident:$ty:ty)*);)* }
     answer($($out:ident),+) :- $($body:tt)*
    ) => {
        pub struct ZoneData { $(pub $table: ::cq_load::ZoneRows<($first_ty, $($ty,)*), (::cq_load::MinMax<$first_ty>, $(::cq_load::MinMax<$ty>,)*)>,)* }
        #[allow(non_snake_case)]
        fn prepare_zones(data: &mut Data) {
            let start = std::time::Instant::now();
            $(let $relation = ::cq_load::ZoneRows::from_rows::<(::cq_load::MinMax<$first_ty>, $(::cq_load::MinMax<$ty>,)*)>(std::mem::take(&mut data.$table), std::num::NonZeroUsize::new(2048).unwrap());)*
            let bytes = 0 $(+ $relation.metadata_bytes())*;
            data.hypergraph = Some(ZoneData { $($table: $relation,)* });
            crate::tpch_runtime::record_zones(start, bytes);
        }
        crate::tpch_runtime::configuration!(hypergraph_extra, $d, {
            plan = ::cq_planners::order(hypergraph);
            select = ::mini_linq::passes::select(positional($($relation),*));
        }, {
            #[allow(non_snake_case)]
            pub(super) fn run(data: &Data) -> Vec<Answer> {
                $(let $relation = data.$table.as_slice();)*
                $($setup)*
                query! { struct Query;
                    $(relation $relation($first:$first_ty $(,$column:$ty)*);)*
                    answer($($out),+) :- $($body)*
                }.map(|($($out,)+)| ($(::core::clone::Clone::clone($out),)+)).collect()
            }
        });
        crate::tpch_runtime::configuration!(hypergraph_zone, $d, {
            plan = ::cq_planners::order(hypergraph);
            select = ::mini_linq::passes::select(positional($($relation),*));
        }, {
            #[allow(non_snake_case)]
            pub(super) fn run(data: &Data) -> Vec<Answer> {
                let data = data.hypergraph.as_ref().expect("configuration loaded ZoneRows");
                $(let $relation = &data.$table;)*
                $($setup)*
                query! { struct Query;
                    $(relation $relation($first:$first_ty $(,$column:$ty)*);)*
                    answer($($out),+) :- $($body)*
                }.map(|($($out,)+)| ($(::core::clone::Clone::clone($out),)+)).collect()
            }
        });
    };
}
pub(crate) use extra_zone;

// Workers load one configuration per process. Move decoded buffers into the
// explicitly selected host once; ordinary configurations retain their Vecs.
macro_rules! extra_prepare {
    ([]; $data:expr, $config:expr) => {{ let _ = $config; $data }};
    ([hypergraph]; $data:expr, $config:expr) => {{
        let mut data = $data;
        if $config == "hypergraph_zone" { prepare_zones(&mut data); }
        data
    }};
}
pub(crate) use extra_prepare;
