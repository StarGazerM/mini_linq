//! Explicit pipelines and the common typed shell for TPC-H-derived CQ kernels.
//! Query collection is timed; answer serialization and comparison are separate.

/// Expand the same authored CQ with an explicit sequence of public passes.
macro_rules! configuration {
    (default, $d:tt, { $($stages:tt)* }, { $($body:tt)* }) => {
        crate::tpch_runtime::configuration!(default, $d, { $($stages)* }, [], { $($body)* });
        crate::tpch_runtime::configuration!(@named par, $d, {
            $($stages)*
            decompose = [::cq_sequence_tasks_macros::plan];
            grade = ::cq_sequence_tasks_macros::tasks;
            print = ::cq_sequence_tasks_macros::translate_tasks;
        }, { $($body)* });
        crate::tpch_runtime::configuration!(@named no_reduce, $d, {
            plan = ::mini_linq::passes::plan(no_reduce);
            $($stages)*
        }, { $($body)* });
    };
    (default, $d:tt, { $($stages:tt)* }, [$($extra:path),*], { $($body:tt)* }) => {
        crate::tpch_runtime::configuration!(@named default, $d, { $($stages)* }, [$($extra),*], { $($body)* });
        crate::tpch_runtime::configuration!(@named radix, $d, { $($stages)* }, [$($extra,)* ::cq_rewrites::radix_runs], { $($body)* });
        crate::tpch_runtime::configuration!(@named radix_retained, $d, { $($stages)* }, [$($extra,)* ::cq_rewrites::radix_runs, ::cq_rewrites::retain_lookups], { $($body)* });
        crate::tpch_runtime::configuration!(@named built_hash, $d, {
            $($stages)*
            physical = [::cq_rewrites::build_host_reads];
        }, [$($extra),*], { $($body)* });
        crate::tpch_runtime::configuration!(@named built_radix, $d, {
            $($stages)*
            physical = [::cq_rewrites::build_host_reads];
        }, [$($extra,)* ::cq_rewrites::radix_runs], { $($body)* });
    };
    ($module:ident, $d:tt, { $($stages:tt)* }, { $($body:tt)* }) => {
        crate::tpch_runtime::configuration!(@named $module, $d, { $($stages)* }, [], { $($body)* });
    };
    ($module:ident, $d:tt, { $($stages:tt)* }, [ $($extra:path),* ], { $($body:tt)* }) => {
        crate::tpch_runtime::configuration!(@named $module, $d, { $($stages)* }, [ $($extra),* ], { $($body)* });
    };
    (@named $module:ident, $d:tt, { $($stages:tt)* }, { $($body:tt)* }) => {
        crate::tpch_runtime::configuration!(@named $module, $d, { $($stages)* }, [], { $($body)* });
    };
    // Extra Structures rewrites run before the dedup retype shared by every configuration.
    (@named $module:ident, $d:tt, { $($stages:tt)* }, [ $($extra:path),* ], { $($body:tt)* }) => {
        mod $module {
            use super::*;
            #[allow(unused_macros)]
            macro_rules! query {
                (positional($d($d host:ident),*); $d($d tokens:tt)*) => { ::mini_linq::compile! {
                    pipeline {
                        $($stages)*
                        select = ::mini_linq::passes::select(positional($d($d host),*));
                        structures = [$($extra,)* ];
                        surface = [::cq_ext_aggregate::aggregate, ::cq_ext_correlated::correlated];
                    }
                    $d($d tokens)*
                } };
                ($d($d tokens:tt)*) => { ::mini_linq::compile! {
                    pipeline {
                        $($stages)*
                        structures = [$($extra,)* ];
                        surface = [::cq_ext_aggregate::aggregate, ::cq_ext_correlated::correlated];
                    }
                    $d($d tokens)*
                } };
            }
            $($body)*
        }
    };
}
pub(crate) use configuration;

/// Standalone (including staged aggregate) queries share one authored runner.
/// Each `query!` carries stage-local advice; only the advised variant applies it.
macro_rules! standalone_configs {
    ($d:tt; $($body:tt)*) => {
        pub const CONFIGS: &[&str] = &["default", "par", "radix", "radix_retained", "no-reduce", "transfer", "le", "le_runs", "advised"];
        crate::tpch_runtime::standalone_configuration!(default, $d, {}, [], { $($body)* });
        crate::tpch_runtime::standalone_configuration!(radix, $d, {}, [::cq_rewrites::radix_runs], { $($body)* });
        crate::tpch_runtime::standalone_configuration!(radix_retained, $d, {}, [::cq_rewrites::radix_runs, ::cq_rewrites::retain_lookups], { $($body)* });
        crate::tpch_runtime::standalone_configuration!(par, $d, {
            decompose = [::cq_sequence_tasks_macros::plan];
            grade = ::cq_sequence_tasks_macros::tasks;
            print = ::cq_sequence_tasks_macros::translate_tasks;
        }, [], { $($body)* });
        crate::tpch_runtime::standalone_configuration!(no_reduce, $d, {
            plan = ::mini_linq::passes::plan(no_reduce);
        }, [], { $($body)* });
        crate::tpch_runtime::standalone_configuration!(transfer, $d, {
            plan = ::cq_planners::order(transfer);
        }, [], { $($body)* });
        crate::tpch_runtime::standalone_configuration!(le, $d, {
            plan = ::cq_planners::order(le);
        }, [], { $($body)* });
        crate::tpch_runtime::standalone_configuration!(le_runs, $d, {
            plan = ::cq_planners::order(le);
        }, [::cq_rewrites::sorted_runs, ::cq_rewrites::retain_lookups], { $($body)* });
        crate::tpch_runtime::standalone_configuration!(advised, $d, {}, [], { $($body)* });
        pub fn run(data: &Data, config: &str) -> Vec<Answer> {
            match config {
                "default" => default::run(data, config),
                "par" => par::run(data, config),
                "radix" => radix::run(data, config),
                "radix_retained" => radix_retained::run(data, config),
                "no-reduce" => no_reduce::run(data, config),
                "transfer" => transfer::run(data, config),
                "le" => le::run(data, config),
                "le_runs" => le_runs::run(data, config),
                "advised" => advised::run(data, config),
                _ => panic!("unknown configuration {}", config),
            }
        }
    };
}
pub(crate) use standalone_configs;

macro_rules! standalone_configuration {
    ($module:ident, $d:tt, { $($stages:tt)* }, [$($extra:path),*], { $($body:tt)* }) => {
        #[allow(non_snake_case)]
        mod $module {
            use super::*;
            macro_rules! query {
                (plan($d($d advice:tt)*); evidence { $d($d facts:tt)* } $d($d tokens:tt)*) => {
                    crate::tpch_runtime::standalone_query! {
                        $module; [$d($d advice)*]; {
                            $($stages)*
                            sequence = ::mini_linq::passes::sequence(evidence { $d($d facts)* });
                        };
                        [$($extra),*]; $d($d tokens)*
                    }
                };
                (plan($d($d advice:tt)*); $d($d tokens:tt)*) => {
                    crate::tpch_runtime::standalone_query! {
                        $module; [$d($d advice)*]; { $($stages)* };
                        [$($extra),*]; $d($d tokens)*
                    }
                };
            }
            $($body)*
        }
    };
}

pub(crate) use standalone_configuration;

macro_rules! standalone_query {
    (advised; [$($advice:tt)*]; {$($stages:tt)*}; []; $($tokens:tt)*) => {
        crate::tpch_runtime::standalone_query! {
            default; []; { $($stages)* plan = ::mini_linq::passes::plan($($advice)*); }; [];
            $($tokens)*
        }
    };
    ($module:ident; [$($advice:tt)*]; {$($stages:tt)*}; [$($extra:path),*]; $($tokens:tt)*) => {
        ::mini_linq::compile! {
            pipeline {
                $($stages)*
                structures = [$($extra,)* ];
                surface = [::cq_ext_aggregate::aggregate, ::cq_ext_correlated::correlated];
            }
            $($tokens)*
        }
    };
}
pub(crate) use standalone_query;

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
        $(advise plan($($advice:tt)*);)?
        $(extra $extra:ident;)?
        setup { $($setup:tt)* }
        relations {
            $(relation $relation:ident => $table:ident (
                $first:ident : $first_ty:ty $(, $column:ident : $ty:ty)* $(,)?
            );)*
        }
        $(derived { $($derived:tt)* })?
        answer($($out:ident : $out_ty:ty),+ $(,)?) :- $($body:tt)*
    ) => {
        pub const NAME: &str = $name;
        pub const SQL: &str = $sql;
        pub const CONFIGS: &[&str] = &["default", "par", "radix", "radix_retained", "built_hash", "built_radix", "no-reduce", "transfer", "le", "le_runs", stringify!($candidate) $(, crate::tpch_runtime::advised_name!($($advice)*))? $(, stringify!($extra), concat!(stringify!($extra), "_zone"))?];
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
                    $($($derived)*)?
                    answer($($out),+) :- $($body)*
                }.map(|($($out,)+)| ($(::core::clone::Clone::clone($out),)+)).collect()
            }
        });
        // Predicate transfer uses the same authored query and Structures,
        // connecting atom runs before forward/backward semijoin transfer.
        crate::tpch_runtime::configuration!(transfer, $d, {
            plan = ::cq_planners::order(transfer);
            select = ::mini_linq::passes::select(positional($($relation),*));
        }, {
            #[allow(non_snake_case)]
            pub(super) fn run(data: &Data) -> Vec<Answer> {
                $(let $relation = data.$table.as_slice();)*
                $($setup)*
                query! {
                    struct Query;
                    $(relation $relation($first: $first_ty $(, $column: $ty)*);)*
                    $($($derived)*)?
                    answer($($out),+) :- $($body)*
                }.map(|($($out,)+)| ($(::core::clone::Clone::clone($out),)+)).collect()
            }
        });
        crate::tpch_runtime::configuration!(le, $d, {
            plan = ::cq_planners::order(le);
            select = ::mini_linq::passes::select(positional($($relation),*));
        }, {
            #[allow(non_snake_case)]
            pub(super) fn run(data: &Data) -> Vec<Answer> {
                $(let $relation = data.$table.as_slice();)*
                $($setup)*
                query! {
                    struct Query;
                    $(relation $relation($first: $first_ty $(, $column: $ty)*);)*
                    $($($derived)*)?
                    answer($($out),+) :- $($body)*
                }.map(|($($out,)+)| ($(::core::clone::Clone::clone($out),)+)).collect()
            }
        });
        // Lookup & Expand with sorted runs and shared lookups: integral-key
        // builds become sorted clustered runs with rank directories, and each
        // early lookup's cursor is the witness its later expansion forces.
        crate::tpch_runtime::configuration!(le_runs, $d, {
            plan = ::cq_planners::order(le);
            select = ::mini_linq::passes::select(positional($($relation),*));
        }, [::cq_rewrites::sorted_runs, ::cq_rewrites::retain_lookups], {
            #[allow(non_snake_case)]
            pub(super) fn run(data: &Data) -> Vec<Answer> {
                $(let $relation = data.$table.as_slice();)*
                $($setup)*
                query! {
                    struct Query;
                    $(relation $relation($first: $first_ty $(, $column: $ty)*);)*
                    $($($derived)*)?
                    answer($($out),+) :- $($body)*
                }.map(|($($out,)+)| ($(::core::clone::Clone::clone($out),)+)).collect()
            }
        });
        // The default plan with the kernel author's recorded plan advice.
        crate::tpch_runtime::advised_config!([$($($advice)*)?]; $d; positional($($relation),*);
            #[allow(non_snake_case)]
            pub(super) fn run(data: &Data) -> Vec<Answer> {
                $(let $relation = data.$table.as_slice();)*
                $($setup)*
                query! {
                    struct Query;
                    $(relation $relation($first: $first_ty $(, $column: $ty)*);)*
                    $($($derived)*)?
                    answer($($out),+) :- $($body)*
                }.map(|($($out,)+)| ($(::core::clone::Clone::clone($out),)+)).collect()
            }
        );
        crate::tpch_runtime::candidate!($candidate; $d; positional($($relation),*);
            #[allow(non_snake_case)]
            pub(super) fn run(data: &Data) -> Vec<Answer> {
                $(let $relation = data.$table.as_slice();)*
                $($setup)*
                query! {
                    struct Query;
                    $(relation $relation($first: $first_ty $(, $column: $ty)*);)*
                    $($($derived)*)?
                    answer($($out),+) :- $($body)*
                }.map(|($($out,)+)| ($(::core::clone::Clone::clone($out),)+)).collect()
            }
        );
        crate::tpch_runtime::extra_zone!([$($extra)?]; $d;
            setup { $($setup)* }
            relations { $(relation $relation => $table ($first:$first_ty $(,$column:$ty)*);)* }
            derived { $($($derived)*)? }
            answer($($out),+) :- $($body)*
        );
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
                stringify!($candidate) => tuned::run(data),
                $(crate::tpch_runtime::advised_name!($($advice)*) => advised::run(data),)?
                _ => crate::tpch_runtime::extra_run!([$($extra)?]; data, config),
            }
        }
        pub fn canonical(rows: &[Answer]) -> Vec<Vec<String>> {
            rows.iter().map(|($($out,)+)| vec![$($out.to_string(),)+]).collect()
        }
    };
}
pub(crate) use retail_kernel;

/// Configuration name of a kernel's advised plan.
macro_rules! advised_name {
    ($($advice:tt)*) => {
        "advised"
    };
}
pub(crate) use advised_name;

macro_rules! advised_config {
    ([]; $d:tt; positional($($host:ident),*); $($body:tt)*) => {};
    ([$($advice:tt)+]; $d:tt; positional($($host:ident),*); $($body:tt)*) => {
        crate::tpch_runtime::configuration!(advised, $d, {
            plan = ::mini_linq::passes::plan($($advice)+);
            select = ::mini_linq::passes::select(positional($($host),*));
        }, { $($body)* });
    };
}
pub(crate) use advised_config;

pub(crate) fn date(value: &str) -> cq_load::Date {
    <cq_load::Date as cq_load::Load>::load(value).expect("authored ISO date")
}

thread_local! {
    static ZONE_COST: std::cell::Cell<(f64, usize)> = const { std::cell::Cell::new((0.0, 0)) };
}
pub fn zone_metrics() -> (f64, usize) {
    ZONE_COST.with(std::cell::Cell::get)
}
pub fn record_zones(start: std::time::Instant, bytes: usize) {
    ZONE_COST.with(|c| c.set((start.elapsed().as_secs_f64() * 1000.0, bytes)));
}
macro_rules! extra_run {
    ([]; $data:expr, $config:expr) => {
        panic!("unknown configuration {}", $config)
    };
    ([hypergraph]; $data:expr, $config:expr) => {
        match $config {
            "hypergraph" => hypergraph_extra::run($data),
            "hypergraph_zone" => hypergraph_zone::run($data),
            _ => panic!("unknown configuration {}", $config),
        }
    };
}
pub(crate) use extra_run;
macro_rules! extra_zone {
    ([]; $($rest:tt)*) => {};
    ([hypergraph]; $d:tt;
     setup { $($setup:tt)* }
     relations { $(relation $relation:ident => $table:ident ($first:ident:$first_ty:ty $(,$column:ident:$ty:ty)*);)* }
     derived { $($derived:tt)* }
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
                    $($derived)*
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
                    $($derived)*
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
    ([]; $data:expr, $config:expr) => {{
        let _ = $config;
        $data
    }};
    ([hypergraph]; $data:expr, $config:expr) => {{
        let mut data = $data;
        if $config == "hypergraph_zone" {
            prepare_zones(&mut data);
        }
        data
    }};
}
pub(crate) use extra_prepare;

/// Check complete canonical rows under the same plan and alternate indexes.
#[cfg(test)]
pub(crate) fn same_plan_answers<T>(
    run: impl Fn(&str) -> Vec<T>,
    canonical: impl Fn(&[T]) -> Vec<Vec<String>>,
) -> Vec<T> {
    let baseline = run("default");
    let mut expected = canonical(&baseline);
    expected.sort();
    for config in ["radix", "radix_retained"] {
        let mut actual = canonical(&run(config));
        actual.sort();
        assert_eq!(actual, expected, "{config} differs from the default query");
    }
    baseline
}
