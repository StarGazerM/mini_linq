use cq_lang_sequence::{LevelNode, Machine, OutputProtocol, Program};
use cq_planners_plan::Planner;
use cq_sequence_lowering::{check, compile};

fn accept(physical: &cq_lang_structures::physical::Program, source: &cq_lang_structures::Program) -> Program {
    let target = compile(physical, source).unwrap();
    assert!(target.graded.is_none());
    assert_eq!(target.sequence.output, Some(OutputProtocol::Count));
    cq_lang_sequence::contract::check(physical, source, &target).unwrap();
    cq_lang_sequence::check_grading(
        physical,
        source,
        &target,
        &Machine {
            gpu: None,
            layout: None,
            name: syn::parse_quote!(sequential),
            items: 1,
            key_partition_threshold: 65536,
            key_partition_cache_threshold: 6 * 1024 * 1024,
            root: LevelNode {
                shared_bytes: None,
                name: syn::parse_quote!(host),
                host_rust: true,
                collaborative: false,
                dynamic: false,
                width: None,
                children: vec![],
            },
        },
    )
    .unwrap();
    check(physical, source, &target).unwrap();
    target
}

fn lower_cq(source: &cq_testkit::cq::Program, label: &str) {
    for planner in Planner::ALL {
        let result = (|| -> syn::Result<_> {
            let ra = cq_planners_plan::compile(source, planner)?;
            let ix = cq_index_policy::compile(&ra)?;
            let physical = cq_physical_lowering::compile(&ix)?;
            let structures = cq_structure_selection::compile(&physical)?;
            Ok((physical, structures))
        })()
        .unwrap_or_else(|error| panic!("{label} {planner:?}: {error}"));
        accept(&result.0, &result.1);
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
    use cq_lang_structures::physical::{self, ModeKind};
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
            let facts = physical::contract::check(&physical).unwrap();
            let candidates = physical::candidate_prefixes(&physical, &facts);
            // A retained side needs [prefix][candidate], with exactly one
            // remaining column. Host pipes use their declared host adapters;
            // only Let sides can receive a built candidate directory.
            let unsupported: Vec<_> = candidates
                .iter()
                .flat_map(|candidate| {
                    let facts = &facts;
                    candidate.sides.iter().filter_map(move |(side, prefix)| {
                        let host = facts.hosts.contains_key(side)
                            && facts.modes[side] == ModeKind::Pipe;
                        let tail: Vec<_> = facts.headings[side]
                            .iter()
                            .filter(|column| !prefix.contains(column))
                            .collect();
                        (!host
                            && (facts.modes[side] != ModeKind::Let
                                || tail != [&candidate.column]))
                            .then_some((candidate.reader, side, prefix, tail))
                    })
                })
                .collect();
            let structures = match cq_structure_selection::compile(&physical) {
                Ok(structures) => {
                    assert!(
                        unsupported.is_empty(),
                        "{pipeline:?}: accepted unsupported candidate facts {unsupported:?}"
                    );
                    let selected = cq_lang_structures::contract::check(&physical, &structures).unwrap();
                    for candidate in &candidates {
                        for (side, prefix) in &candidate.sides {
                            assert!(
                                selected.host_read(side)
                                    || selected
                                        .candidate_on(side, prefix, &candidate.column)
                                        .is_some(),
                                "{pipeline:?}: missing candidate directory for {side}"
                            );
                        }
                    }
                    structures
                }
                Err(error) => {
                    assert!(
                        !unsupported.is_empty(),
                        "{pipeline:?}: unexpected Structures rejection: {error}"
                    );
                    let diagnostics: Vec<_> =
                        error.into_iter().map(|error| error.to_string()).collect();
                    let expected = format!(
                        "{}: {}",
                        unsupported[0].0,
                        cq_workloads::pipelines::SMALLEST_LAYOUT_REJECTION
                    );
                    assert_eq!(
                        diagnostics,
                        [expected],
                        "{pipeline:?}: Structures must reject the first unsupported candidate"
                    );
                    continue;
                }
            };
            compile(&physical, &structures).unwrap_or_else(|error| {
                panic!("{pipeline:?}: {error}\n{}", structures.to_token_stream())
            });
        }
    }
}
