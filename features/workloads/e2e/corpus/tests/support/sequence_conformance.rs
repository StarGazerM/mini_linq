//! Shared registration, comparison, and reporting for the partitioned sequence corpus.
use cq_testkit::conformance::assert_answers_equal;
use cq_workloads::{catalog, execution, schema::Inputs};

pub type Execute = fn(&mut Inputs) -> Vec<String>;

pub struct Entry {
    pub case: &'static str,
    pub configuration: &'static str,
    pub translations: (Execute, Execute, bool),
}
inventory::collect!(Entry);

// Consume precisely the same configuration invocations as the executable corpus.
// Host choice is preserved; the terminal and its input language are replaced together.
macro_rules! pipeline_configuration {
    ($case:ident, $name:ident; planner: [le_runs]; merge: $merge:tt;
     canon: $canon:tt; intersection: $intersection:tt;
     host: $host:tt $view:ident $advice:tt;
     payload: [$($payload:tt)*]; membership: $membership:tt; $(print_input: $print_input:ident;)? printer: $printer:path;) => {
        pipeline_configuration! { $case, $name; planner: [le]; merge: $merge;
            canon: $canon; intersection: $intersection; host: $host $view $advice;
            payload: [cq_rewrites::sorted_runs, $($payload)*];
            membership: $membership; $(print_input: $print_input;)? printer: $printer;
        }
    };
    ($case:ident, $name:ident; planner: [$($plan:tt)*]; merge: [$($merge:tt)*];
     canon: [$($canon:tt)*]; intersection: [$($intersection:tt)*];
     host: [$load:ident] $view:ident [$($advice:ident)?];
     payload: [$($payload:tt)*];
     membership: [$($membership:tt)*]; $(print_input: $print_input:ident;)? printer: $printer:path;) => {
        inventory::submit! {
            crate::conformance::Entry {
                case: stringify!($case),
                configuration: module_path!(),
                translations: cq_workload_passes::conformance_targets! {
                    $case, [$($advice)?], $load, {
                        plan = cq_planners::order($($plan)*);
                        physical = [$($canon)* $($intersection)* $($membership)* $($merge)*];
                        structures = [$($payload)*];
                    }
                },
            }
        }
    };
}

pub fn run(expected_configurations: usize, expected_cases: &[&str]) {
    let mut entries: Vec<_> = inventory::iter::<Entry>.into_iter().collect();
    entries.sort_by_key(|entry| entry.configuration);
    let configurations = entries.len();
    assert_eq!(configurations, expected_configurations);
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
    let expected = expected_cases
        .iter()
        .map(|case| case.replace('-', "_"))
        .collect();
    assert_eq!(
        registered, expected,
        "every catalog query must be registered"
    );
    let mut agreed = 0;
    let mut failed = Vec::new();
    for entry in entries {
        let stem = entry.case.strip_prefix("run_").expect("corpus macro name");
        let case = catalog::all()
            .find(|case| case.name.replace('-', "_") == stem)
            .expect("registered corpus query");
        for &scenario in execution::scenarios(case.name) {
            let label = format!(
                "{}/{}/{}",
                entry.configuration,
                case.name,
                scenario.as_str()
            );
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let (sequential, tasks, canonical) = entry.translations;
                let expected = sequential(&mut execution::inputs(case.name, scenario));
                let actual = tasks(&mut execution::inputs(case.name, scenario));
                assert_answers_equal(
                    &format!("{label}: Sequential"),
                    expected,
                    &format!("{label}: Tasks"),
                    actual,
                    canonical,
                );
            }));
            if result.is_ok() {
                agreed += 1;
            } else {
                eprintln!("{label}: FAILED (execution or answer divergence)");
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
