//! Replay every pairwise and full-kernel configuration on both CPU targets.
#![recursion_limit = "512"]
#![allow(dead_code)]

use cq_lang_sequence::Program;
use cq_testkit::conformance::Target;
use cq_workloads::{Scenario, catalog, execution};
use quote::quote;

struct Entry {
    case: &'static str,
    configuration: &'static str,
    loader: &'static str,
    program: fn() -> Program,
}
inventory::collect!(Entry);

// Consume precisely the same configuration invocations as the executable corpus.
// Host choice is preserved in the runtime bindings; only the printer is replaced.
macro_rules! pipeline_configuration {
    ($case:ident, $name:ident; planner: [le_runs]; merge: $merge:tt;
     canon: $canon:tt; intersection: $intersection:tt;
     host: $host:tt $view:ident $advice:tt; dedup: $dedup:tt;
     payload: [$($payload:tt)*]; membership: $membership:tt; printer: $printer:path;) => {
        pipeline_configuration! { $case, $name; planner: [le]; merge: $merge;
            canon: $canon; intersection: $intersection; host: $host $view $advice;
            dedup: $dedup; payload: [cq_rewrites::sorted_runs, $($payload)*];
            membership: $membership; printer: $printer;
        }
    };
    ($case:ident, $name:ident; planner: [$($plan:tt)*]; merge: [$($merge:tt)*];
     canon: [$($canon:tt)*]; intersection: [$($intersection:tt)*];
     host: [$load:ident] $view:ident [$($advice:ident)?];
     dedup: [$($dedup:tt)*]; payload: [$($payload:tt)*];
     membership: [$($membership:tt)*]; printer: $printer:path;) => {
        inventory::submit! {
            crate::Entry {
                case: stringify!($case),
                configuration: module_path!(),
                loader: stringify!($load),
                program: || cq_workload_passes::conformance_program! {
                    $case, [$($advice)?], {
                        plan = cq_planners::order($($plan)*);
                        physical = [$($canon)* $($intersection)* $($membership)* $($merge)*];
                        structures = [$($dedup)* $($payload)*];
                    }
                },
            }
        }
    };
}
#[macro_use]
#[path = "support/le_configurations.rs"]
mod le_configurations;
#[macro_use]
#[path = "support/configurations.rs"]
mod configurations;
#[macro_use]
#[path = "support/full_matrix.rs"]
mod full_matrix;

macro_rules! pairwise {
    ($($case:ident => $run:ident),* $(,)?) => {
        $(mod $case { case_configurations!($case, $run); })*
        pub const CONFIGURATION_COUNT: usize = 0 $(+ $case::CONFIGURATIONS.len())*;
    };
}
mod pairwise {
    pairwise! {
        intersection => run_intersection,
        cartesian_product => run_cartesian_product,
        oriented_chain => run_oriented_chain,
        claw => run_claw,
        fact_star => run_fact_star,
        triangle => run_triangle,
        cycle_4 => run_cycle_4,
        spade => run_spade,
        barbell => run_barbell,
        loomis_whitney_4 => run_loomis_whitney_4,
        retail_h03 => run_retail_h03,
        retail_h05 => run_retail_h05,
        retail_h07 => run_retail_h07,
        retail_h08 => run_retail_h08,
        retail_h09 => run_retail_h09,
        retail_h21_positive => run_retail_h21_positive,
        retail_d19 => run_retail_d19,
        retail_d27 => run_retail_d27,
        retail_d72_inner => run_retail_d72_inner,
        retail_d85 => run_retail_d85,
    }
}
mod kernel {
    mod written_coarse {
        full_matrix!([written, coarse]; merge_hosts: ordered);
    }
    mod written_fine {
        full_matrix!([written, fine]; merge_hosts: all);
    }
    mod prefix_connected_coarse {
        full_matrix!([prefix connected, coarse]; merge_hosts: ordered);
    }
    mod prefix_connected_fine {
        full_matrix!([prefix connected, fine]; merge_hosts: all);
    }
    mod prefix_greedy_coarse {
        full_matrix!([prefix greedy, coarse]; merge_hosts: ordered);
    }
    mod prefix_greedy_fine {
        full_matrix!([prefix greedy, fine]; merge_hosts: all);
    }
    mod elimination_coarse {
        full_matrix!([elimination, coarse]; merge_hosts: ordered);
    }
    mod elimination_fine {
        full_matrix!([elimination, fine]; merge_hosts: all);
    }
    mod hypergraph {
        full_matrix!([hypergraph]; merge_hosts: ordered);
    }
    mod le {
        full_matrix!([le]; merge_hosts: ordered);
    }
    mod le_runs {
        full_matrix!([le_runs]; merge_hosts: ordered);
    }
}

fn bindings(
    case: &cq_workloads::QueryCase,
    scenario: Scenario,
    loader: &str,
) -> proc_macro2::TokenStream {
    let name = case.name;
    let scenario = match scenario {
        Scenario::Coverage => quote!(Coverage),
        Scenario::EmptyInput => quote!(EmptyInput),
        Scenario::NoMatch => quote!(NoMatch),
    };
    let loader = syn::Ident::new(loader, proc_macro2::Span::call_site());
    let inputs = case.inputs.iter().map(|input| {
        let name = input.name;
        let ident = syn::Ident::new(name, proc_macro2::Span::call_site());
        let types = input.columns.iter().map(|_| quote!(i32));
        if loader == "take" {
            quote!(let #ident = &inputs.#loader::<(#(#types,)*)>(#name)[..];)
        } else {
            quote!(let #ident = &inputs.#loader::<(#(#types,)*)>(#name);)
        }
    });
    quote! {
        let mut inputs = ::cq_workloads::execution::inputs(#name, ::cq_workloads::Scenario::#scenario);
        #(#inputs)*
    }
}

#[test]
fn sequential_and_tasks_agree_on_every_corpus_configuration_and_scenario() {
    use cq_testkit::rustc::{Runtime, linked_dependency};
    let runtime = Runtime::with_dependencies(vec![
        linked_dependency::<cq_storage::HashIndex<'static, (i32,), (i32,)>>(
            "cq_storage",
            "cq_storage::HashIndex<'static, (i32,), (i32,)>",
        ),
        linked_dependency::<cq_workloads::Scenario>("cq_workloads", "cq_workloads::Scenario"),
    ]);
    // This provider belongs to the workload, not to either target crate.
    let catalog = cq_lang_sequence::Catalog {
        providers: vec![cq_lang_sequence::ProviderToken {
            ty: syn::parse_quote!(::cq_workloads::SortedIndex<_, _>),
            elements: None,
            capabilities: vec![
                cq_lang_sequence::Capability::Exact,
                cq_lang_sequence::Capability::Membership,
            ],
            levels: vec![
                vec!["host".into()],
                vec!["host".into(), "pool".into()],
                vec!["host".into(), "pool".into(), "task".into()],
            ],
        }],
        ..Default::default()
    };
    let mut entries: Vec<_> = inventory::iter::<Entry>.into_iter().collect();
    entries.sort_by_key(|entry| entry.configuration);
    let configurations = entries.len();
    assert_eq!(configurations, 2_848 + pairwise::CONFIGURATION_COUNT);
    let registered: std::collections::BTreeSet<_> = entries
        .iter()
        .map(|entry| {
            entry
                .case
                .strip_prefix("run_")
                .expect("corpus macro name")
                .to_owned()
        })
        .collect();
    let expected = catalog::all()
        .map(|case| case.name.replace('-', "_"))
        .collect();
    assert_eq!(
        registered, expected,
        "every catalog query must be registered"
    );
    // Diagnose unsupported authored tokens before compiling the executable cases.
    for entry in &entries {
        for target in [Target::Sequential, Target::Tasks] {
            cq_testkit::conformance::plan_with_catalog(&(entry.program)(), target, &catalog)
                .unwrap_or_else(|error| {
                    panic!(
                        "{}: {target:?} planning rejected: {error}",
                        entry.configuration
                    )
                });
        }
    }
    let mut agreed = 0;
    let mut failed = Vec::new();
    for entry in entries {
        let stem = entry.case.strip_prefix("run_").expect("corpus macro name");
        let case = catalog::all()
            .find(|case| case.name.replace('-', "_") == stem)
            .expect("registered corpus query");
        let sequence = (entry.program)();
        for &scenario in execution::scenarios(case.name) {
            let label = format!(
                "{}/{}/{}",
                entry.configuration,
                case.name,
                scenario.as_str()
            );
            // Both targets plan this unmodified fixture using their own catalog.
            let _ = execution::inputs(case.name, scenario);
            eprintln!("{label}: comparing Sequential vs Tasks");
            let bindings = bindings(case, scenario, entry.loader);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                cq_testkit::conformance::assert_conformant_with_runtime(
                    &sequence,
                    &[Target::Sequential, Target::Tasks],
                    &bindings,
                    &runtime,
                    &catalog,
                );
            }));
            if result.is_ok() {
                agreed += 1;
            } else {
                eprintln!("{label}: FAILED (planning, translation or answer divergence)");
                failed.push(label);
            }
        }
    }
    eprintln!(
        "Sequence corpus conformance: {configurations} configurations; {agreed} agreed / {} failed",
        failed.len()
    );
    assert!(failed.is_empty(), "conformance failures: {failed:#?}");
}
