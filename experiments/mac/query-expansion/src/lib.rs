//! Expand the actual H03/H09 source with its default positional-host pipeline.
//! This shell omits benchmark loaders and alternative planners, not query clauses.

pub mod tpch_runtime {
    pub fn date(value: &str) -> cq_load::Date {
        value.parse().expect("valid authored date")
    }

    macro_rules! retail_kernel {
        (
            $d:tt;
            name $name:literal;
            sql $sql:literal;
            candidate $candidate:ident;
            $(advise plan($($advice:tt)*);)?
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
            #[derive(Clone)]
            pub struct Data { $(pub $table: Vec<($first_ty, $($ty,)*)>,)* }
            pub type Answer = ($($out_ty,)+);
            // Reuse the exact authored clauses and declaration order for ordinary
            // and explicitly prepared host bindings.
            macro_rules! with_hosts {
                ($( $d $relation:expr ),* $d(,)?) => {{
                    $(let $relation = $d $relation;)*
                    $($setup)*
                    mini_linq::compile! {
                        pipeline {
                            select = mini_linq::passes::select(positional($($relation),*));
                        }
                        struct Query;
                        $(relation $relation($first: $first_ty $(, $column: $ty)*);)*
                        answer($($out),+) :- $($body)*
                    }.map(|($($out,)+)| ($(::core::clone::Clone::clone($out),)+)).collect()
                }};
            }
            pub(crate) use with_hosts;

            #[allow(non_snake_case)]
            pub fn compiled(data: &Data) -> Vec<Answer> {
                self::with_hosts!($(data.$table.as_slice()),*)
            }
        };
    }
    pub(crate) use retail_kernel;
}

pub mod h03 {
    include!("../../../../features/workloads/src/queries/retail/tpch/retail_h03.rs");
}

pub mod h09 {
    include!("../../../../features/workloads/src/queries/retail/tpch/retail_h09.rs");
}

pub mod fixtures;
pub mod manual_h03;
pub mod manual_h09;
#[cfg(target_os = "macos")]
pub mod compiled_h09;

/// Execution evidence for the handwritten paths, not a runtime telemetry API.
pub struct ManualResult<T> {
    pub answers: Vec<T>,
    pub index_builds: usize,
    pub indexed_rows: usize,
    pub index_probes: usize,
    pub payload_gathers: usize,
    pub witnesses: usize,
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use cq_mac::MacContext;

    #[test]
    fn h03_reuses_full_orders_and_compacts_only_selected_rows() {
        let context = MacContext::new().unwrap();
        let mut data = fixtures::h03_boundaries();
        // Both wide rows are deliberately unselected for different reasons.
        data.orders
            .push((i64::MAX, 1, tpch_runtime::date("1995-03-15")));
        data.orders
            .push((-1, i64::MIN, tpch_runtime::date("1995-03-14")));
        let prepared = manual_h03::Prepared::new(&context, &data).unwrap();
        assert_eq!(prepared.source_bytes(), data.orders.len() * 24);
        let first = manual_h03::execute(&context, &data, &prepared).unwrap();
        let second = manual_h03::execute(&context, &data, &prepared).unwrap();
        assert_eq!(first.answers, second.answers);
        assert_eq!(first.witnesses, 12);
        assert_eq!(second.witnesses, first.witnesses);
        assert!(first.indexed_rows < data.orders.len());
        let mut expected = h03::compiled(&data);
        expected.sort_unstable();
        let mut actual = first.answers;
        actual.sort_unstable();
        assert_eq!(actual, expected);
        // Ingestion preserves the invalid key; GPU selected projection rejects it.
        data.orders
            .push((i64::MAX, 1, tpch_runtime::date("1995-03-14")));
        let selected = manual_h03::Prepared::new(&context, &data).unwrap();
        assert!(manual_h03::execute(&context, &data, &selected).is_err());
    }

    #[test]
    fn h09_reuses_full_lineitem_and_preserves_wide_payload_witnesses() {
        let context = MacContext::new().unwrap();
        let data = fixtures::h09_boundaries();
        let prepared = manual_h09::Prepared::new(&context, &data).unwrap();
        assert_eq!(prepared.source_bytes(), data.lineitem.len() * 24);
        let first = manual_h09::execute(&context, &data, &prepared, &mut MacContext::build)
            .unwrap()
            .result;
        let second = manual_h09::execute(&context, &data, &prepared, &mut MacContext::build)
            .unwrap()
            .result;
        assert_eq!(first.answers, second.answers);
        assert_eq!(first.witnesses, 68);
        assert_eq!(second.witnesses, first.witnesses);
        assert!(first.answers.contains(&(-1, 3, i64::MAX)));
        let mut expected = h09::compiled(&data);
        expected.sort_unstable();
        let mut actual = first.answers;
        actual.sort_unstable();
        assert_eq!(actual, expected);
    }

    #[test]
    fn no_green_demand_never_ingests_or_constructs_lineitem() {
        let context = MacContext::new().unwrap();
        let mut data = fixtures::h09_boundaries();
        data.part
            .iter_mut()
            .for_each(|(_, name)| *name = "red".into());
        data.lineitem.push((i64::MIN, -1, i64::MAX));
        let prepared = manual_h09::Prepared::new(&context, &data).unwrap();
        assert_eq!(prepared.source_bytes(), 0);
        assert_eq!(prepared.ingestion, std::time::Duration::ZERO);
        let execution = manual_h09::execute(&context, &data, &prepared, &mut |_, _, _, _| {
            panic!("no green demand must not invoke a constructor")
        })
        .unwrap();
        assert!(execution.result.answers.is_empty());
        assert_eq!(execution.result.index_builds, 0);
    }
}
