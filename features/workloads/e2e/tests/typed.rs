//! Execute typed authored queries against CSV and DuckDB-produced Parquet.

mod wiring {
    pub use cq_state_providers::{
        default_indexes as index_provider, hash_distinct as distinct_state,
    };
    pub use mini_linq::{
        index_demands as choose_index_policy, select_structures, structures_to_rust,
    };
    macro_rules! policy { ($($tokens:tt)*) => { ::mini_linq::lower_physical! { $($tokens)* policy {} } }; }
    pub(crate) use policy as physical_policy;
}

#[derive(Clone, Copy)]
enum Source {
    Csv,
    #[cfg(feature = "parquet")]
    Parquet,
}

macro_rules! execute_typed {
    ($case:ident; [$($cq:tt)*]; [$($relations:ident),*]; relation $relation:ident ($($columns:tt)*) ; $($rest:tt)*) => {
        execute_typed!($case; [$($cq)*]; [$($relations,)* $relation]; $($rest)*);
    };
    ($case:ident; [$($cq:tt)*]; [$($relation:ident),*]; $($query:tt)*) => {
        #[allow(non_snake_case)]
        fn run(source: super::Source) {
            let case = cq_workloads::typed::case(stringify!($case)).unwrap();
            $(
                let $relation = match source {
                    super::Source::Csv => <relation_loader!($relation)>::csv(
                        case.path("data").join(concat!(stringify!($relation), ".csv")),
                        cq_load::CsvOptions { headers: true, batch_size: 2, ..Default::default() },
                    ).unwrap(),
                    #[cfg(feature = "parquet")]
                    super::Source::Parquet => <relation_loader!($relation)>::parquet(
                        case.path("data").join(concat!(stringify!($relation), ".parquet")),
                        cq_load::ParquetOptions { batch_size: 2 },
                    ).unwrap(),
                };
                let $relation = $relation.as_slice();
            )*
            let rows = ::mini_linq::cq_to_ra! { $($cq)* }
                .map(|(a, b, c, d)| vec![a.to_string(), b.to_string(), c.to_string(), d.to_string()])
                .collect();
            cq_workloads::typed::check(stringify!($case), rows);
        }
    };
}

macro_rules! typed_case {
    ($d:tt; $case:ident) => {
        mod $case {
            use cq_load::{Date, Decimal};
            use super::wiring as mini_linq;

            macro_rules! instantiate {
                ($d vis:vis struct $d name:ident; $d($d tail:tt)*) => {
                    ::mini_linq::relation_loaders! { $d vis struct $d name; $d($d tail)* }
                    execute_typed!($case; [$d vis struct $d name; $d($d tail)*]; []; $d($d tail)*);
                };
            }
            cq_workloads::queries::typed::$case::with_query!(instantiate);

            #[test]
            fn csv_matches_duckdb_golden() {
                run(super::Source::Csv);
            }

            #[cfg(feature = "parquet")]
            #[test]
            fn parquet_matches_duckdb_golden() {
                run(super::Source::Parquet);
            }
        }
    };
}

typed_case!($; typed_line_filter);
typed_case!($; typed_customer_orders);
typed_case!($; typed_calendar_shipping);
typed_case!($; typed_priced_products);
typed_case!($; typed_tagged_products);
