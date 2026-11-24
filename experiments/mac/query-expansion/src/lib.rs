//! Mac experiments over the current authored H03/H09 workload sources.
//! The workload runtime and both query files compile unchanged, with every
//! authored configuration. The default route is additionally replayed once
//! over an explicitly prepared Lineitem host; see [`compiled_h09`].

#[allow(dead_code, unused_imports, unused_macros)]
#[path = "../../../../features/workloads/src/tpch_runtime.rs"]
mod workload_runtime;

/// Workload runtime paths named by the included query sources.
pub mod tpch_runtime {
    pub(crate) use crate::workload_runtime::evidence_query;

    pub fn date(value: &str) -> cq_load::Date {
        crate::workload_runtime::date(value)
    }

    /// The workload configuration macro, unchanged. Its default route also
    /// records a replay of its own stages, Structures rewrites and authored
    /// body, so a prepared host reuses that route instead of a copied query.
    macro_rules! configuration {
        (default, $d:tt, { $($stages:tt)* }, [$($extra:path),*], { $($body:tt)* }) => {
            crate::workload_runtime::configuration!(
                default, $d, { $($stages)* }, [$($extra),*], { $($body)* }
            );
            #[allow(unused_macros)]
            macro_rules! replay_default {
                ($d callback:ident, $d dollar:tt) => {
                    $d callback! { $d dollar { $($stages)* } [$($extra),*] { $($body)* } }
                };
            }
        };
        ($($tokens:tt)*) => {
            crate::workload_runtime::configuration!($($tokens)*);
        };
    }
    pub(crate) use configuration;
}

/// The default route over [`compiled_h09::PreparedData`]. Lineitem additionally
/// declares its native `l_partkey` read; evidence is scanned from the authored
/// declarations, and the pipeline repeats the workload default query's
/// `structures`/`surface` stages.
macro_rules! prepared_default {
    ($d:tt { $($stages:tt)* } [$($extra:path),*] { $($body:tt)* }) => {
        pub(crate) mod prepared {
            use super::*;
            pub(crate) type Data<'a> = crate::compiled_h09::PreparedData<'a>;

            #[allow(unused_macros)]
            macro_rules! query {
                (struct $d query:ident;
                 relation $d lineitem:ident($d order:ident: $d order_ty:ty,
                     $d part:ident: $d part_ty:ty, $d supplier:ident: $d supplier_ty:ty);
                 $d($d rest:tt)*) => {
                    crate::tpch_runtime::evidence_query! {
                        default; {
                            $($stages)*
                            structures = [$($extra,)*];
                            surface = [::cq_ext_aggregate::aggregate, ::cq_ext_correlated::correlated];
                        }
                        struct $d query;
                        relation $d lineitem($d order: $d order_ty, $d part: $d part_ty,
                            $d supplier: $d supplier_ty) host [$d part] reads [exact, membership];
                        $d($d rest)*
                    }
                };
            }

            $($body)*

            pub(crate) fn execute(data: &Data<'_>) -> Vec<Answer> {
                run(data)
            }
        }
    };
}

pub mod h03 {
    include!("../../../../features/workloads/src/queries/retail/tpch/retail_h03.rs");

    /// The authored default route; the ordinary oracle for Mac realizations.
    pub fn compiled(data: &Data) -> Vec<Answer> {
        run(data, "default")
    }
}

pub mod h09 {
    include!("../../../../features/workloads/src/queries/retail/tpch/retail_h09.rs");

    /// The authored default route; the ordinary oracle for Mac realizations.
    pub fn compiled(data: &Data) -> Vec<Answer> {
        run(data, "default")
    }

    #[cfg(target_os = "macos")]
    replay_default!(prepared_default, $);
}

#[cfg(target_os = "macos")]
pub mod compiled_h09;
pub mod fixtures;
pub mod manual_h03;
pub mod manual_h09;
#[cfg(target_os = "macos")]
pub mod metal_h09;

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
        // Customer is a membership read: duplicate BUILDING rows add no witness.
        assert_eq!(first.witnesses, 8);
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
        // PartSupp/Orders/Nation are membership reads; Supplier keeps multiplicity.
        assert_eq!(first.witnesses, 12);
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
        let mut rows = fixtures::h09_boundary_rows();
        rows.lineitem.push((i64::MIN, -1, i64::MAX));
        let data = fixtures::h09_no_green(rows);
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
