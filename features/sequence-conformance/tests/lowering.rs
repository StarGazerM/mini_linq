use cq_lang_sequence::{LevelNode, Machine, OutputProtocol, Program};
use cq_planners_plan::Planner;
use cq_sequence_lowering::{check, compile};

fn accept(source: &cq_lang_structures::Program) -> Program {
    let target = compile(source).unwrap();
    assert!(target.graded.is_none());
    assert_eq!(target.sequence.output, Some(OutputProtocol::Count));
    cq_lang_sequence::contract::check(&target).unwrap();
    cq_lang_sequence::check_grading(
        &target,
        &Machine {
            layout: None,
            name: syn::parse_quote!(sequential),
            items: 1,
            root: LevelNode {
                shared_bytes: None,
                name: syn::parse_quote!(host),
                collaborative: false,
                dynamic: false,
                width: None,
                children: vec![],
            },
        },
    )
    .unwrap();
    check(source, &target).unwrap();
    target
}

fn lower_cq(source: &cq_testkit::cq::Program, label: &str) {
    for planner in Planner::ALL {
        let result = (|| -> syn::Result<_> {
            let ra = cq_planners_plan::compile(source, planner)?;
            let ix = cq_index_policy::compile(&ra)?;
            let physical = cq_physical_lowering::compile(&ix)?;
            cq_structure_selection::compile(&physical)
        })()
        .unwrap_or_else(|error| panic!("{label} {planner:?}: {error}"));
        accept(&result);
    }
}

#[test]
fn every_authored_scenario_and_scenario_family() {
    for scenario in cq_testkit::SCENARIOS {
        lower_cq(&scenario.parse_cq().unwrap(), scenario.name);
    }
    for case in cq_testkit::CASES.iter().chain(cq_testkit::blocks::CASES) {
        lower_cq(&case.parse_cq().unwrap(), case.name);
    }
}

#[test]
fn every_workload_through_every_planner() {
    for case in cq_workloads::catalog::all() {
        let source = cq_workloads::case(case.name).unwrap().module().unwrap();
        lower_cq(&source, case.name);
    }
}

#[test]
fn rewritten_claw_preserves_ordered_stream_providers() {
    use quote::ToTokens;
    let source = cq_workloads::case("claw").unwrap().module().unwrap();
    for plan in 0..Planner::ALL.len() {
        for choices in 0..16 {
            let pipeline = cq_workloads::pipelines::Pipeline {
                plan,
                merge: choices & 1 != 0,
                canon: choices & 2 != 0,
                smallest: choices & 4 != 0,
                lazy: choices & 8 != 0,
                ..Default::default()
            };
            let physical = pipeline.physical(&source).unwrap();
            let structures = match cq_structure_selection::compile(&physical) {
                Ok(structures) => structures,
                Err(error) => {
                    assert!(
                        error
                            .to_string()
                            .ends_with(cq_workloads::pipelines::SMALLEST_LAYOUT_REJECTION)
                    );
                    continue;
                }
            };
            compile(&structures).unwrap_or_else(|error| {
                panic!("{pipeline:?}: {error}\n{}", structures.to_token_stream())
            });
        }
    }
}
