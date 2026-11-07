//! STR-BuildStats mutations keep Physical valid: only publication contracts fail.
use cq_lang_structures::{self as structures, stats::*};
use quote::ToTokens;

fn program() -> structures::Program {
    syn::parse_quote! {
        physical {
            relation L(a:i32,b:i32); relation R(a:i32,b:i32,c:i32);
            l = pipe rename L {a -> x,b -> y};
            r = let rename R {a -> x,b -> y,c -> z};
            j = let canon probe first [x,y] l,r;
            return j as answer(x,y,z);
        }
        structures {
            providers {
                host_scan = HostScan { reads [], build typed };
                index = Index { reads [exact, membership, ordered], build typed,
                    statistics [
                        count: rows occurrences exact run_finalize rows_stat host required build single,
                        distinct: keys distinct_keys exact run_finalize keys_stat host optional build single,
                        extrema: bounds code_bounds exact run_finalize bounds_stat host optional build single
                    ]
                };
                sort = Sort { reads [distinct], build inferred, shared };
            }
            publications {
                rows_r = build r provider index layout [x,y] columns [] domains []
                    stat count: rows occurrences exact run_finalize rows_stat host required build single;
                keys_r = build r provider index layout [x,y] columns [x,y]
                    domains [x: identity integers 0,y: dictionary names 7]
                    stat distinct: keys distinct_keys exact run_finalize keys_stat host optional build single;
            }
            l host scan = @host_scan as HostScan [];
            r host scan = @host_scan as HostScan [];
            r = @index as Index [x,y];
            j rows = @sort as Sort;
            j canon = rows;
        }
    }
}

fn rejected(program: &structures::Program) {
    structures::physical::contract::check(&program.physical).unwrap();
    assert!(structures::contract::check(program).is_err());
}

#[test]
fn publication_and_consumer_identity_roundtrip() {
    let program = program();
    let checked = structures::contract::check(&program).unwrap();
    assert_eq!(checked.publications.len(), 2);
    assert_eq!(
        syn::parse2::<structures::Program>(program.to_token_stream()).unwrap(),
        program
    );
    for publication in checked.publications.values() {
        let reference = publication.reference();
        assert_eq!(
            syn::parse2::<Reference>(reference.to_token_stream()).unwrap(),
            reference
        );
        assert_eq!(
            syn::parse2::<Publication>(publication.to_token_stream()).unwrap(),
            *publication
        );
    }
    assert_eq!(
        checked.publications["keys_r"].statistic.availability,
        Availability::Optional
    );
    assert_eq!(
        checked.publications["rows_r"].statistic.unit,
        Unit::Occurrences
    );
}

#[test]
fn unselected_statistics_default_empty_and_emit_no_publications() {
    let mut program = program();
    program.structures.publications.0.clear();
    for provider in &mut program.structures.providers.declarations {
        provider.facts.statistics.clear();
    }
    let tokens = program.to_token_stream();
    assert!(!tokens.to_string().contains("publications"));
    assert!(!tokens.to_string().contains("statistics"));
    let parsed: structures::Program = syn::parse2(tokens).unwrap();
    assert!(
        structures::contract::check(&parsed)
            .unwrap()
            .publications
            .is_empty()
    );
    assert!(
        parsed
            .structures
            .providers
            .declarations
            .iter()
            .all(|d| d.facts.statistics.is_empty())
    );
}

#[test]
fn declarations_alone_do_not_select_statistics() {
    let mut program = program();
    program.structures.publications.0.clear();
    let checked = structures::contract::check(&program).unwrap();
    assert!(checked.publications.is_empty());
    assert_eq!(
        program.structures.providers.declarations[1]
            .facts
            .statistics
            .len(),
        3
    );
}

fn build_provider(program: &mut structures::Program) -> &mut structures::Provider {
    program.structures.entries.iter_mut().find_map(|entry| match &mut entry.structure {
        structures::Structure::Built { keyed, .. } if entry.name == "r" => Some(&mut keyed.provider),
        _ => None,
    }).unwrap()
}

#[test]
fn construction_is_selected_from_declared_support_and_publication_demand() {
    for (rows, max_group) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut selected = program();
        let declaration = &mut selected.structures.providers.declarations[1];
        declaration.facts.statistics_build = BuildDemand { rows: true, max_group: true };
        let maximum: StatDeclaration = syn::parse_quote!(
            maximum: max_group max_bucket_occurrences exact run_finalize max_stat host optional build single
        );
        declaration.facts.statistics.push(maximum.clone());
        if !rows {
            selected.structures.publications.0.retain(|publication| publication.statistic.field != Field::Rows);
        }
        if max_group {
            selected.structures.publications.0.push(syn::parse_quote!(
                maximum_r = build r provider index layout [x,y] columns [x,y]
                    domains [x: identity integers 0,y: dictionary names 7] stat #maximum;
            ));
        }
        if rows || max_group {
            rejected(&selected);
        }
        record_constructions(&mut selected).unwrap();
        let expected = if rows || max_group {
            Construction::Statistics(BuildDemand { rows, max_group })
        } else {
            Construction::Ordinary
        };
        assert_eq!(build_provider(&mut selected).construction, expected);
        assert_eq!(syn::parse2::<structures::Program>(selected.to_token_stream()).unwrap(), selected);
        let mut stale = selected.clone();
        build_provider(&mut stale).construction = Construction::Statistics(BuildDemand {
            rows: !rows, max_group,
        });
        rejected(&stale);
        if rows || max_group {
            let mut unsupported = selected.clone();
            unsupported.structures.providers.declarations[1].facts.statistics_build = BuildDemand::default();
            rejected(&unsupported);
            let mut missing = selected.clone();
            missing.structures.publications.0.clear();
            rejected(&missing);
        }
    }
}

#[test]
fn ordinary_protocol_does_not_require_a_statistic_builder_for_cheap_accessors() {
    let mut selected = program();
    record_constructions(&mut selected).unwrap();
    assert_eq!(build_provider(&mut selected).construction, Construction::Ordinary);
    let provider: structures::Provider = syn::parse_quote!(@index construct ordinary as Index);
    assert_eq!(provider.construction, Construction::Ordinary);
    assert!(syn::parse_str::<BuildDemand>("[rows, rows]").is_err());
    assert!(syn::parse_str::<BuildDemand>("[keys]").is_err());
    let mut empty = program();
    build_provider(&mut empty).construction = Construction::Statistics(BuildDemand::default());
    rejected(&empty);
}

#[test]
fn publication_rejects_wrong_build_provider_and_ordered_layout() {
    for mutation in 0..4 {
        let mut program = program();
        let publication = &mut program.structures.publications.0[0];
        match mutation {
            0 => publication.build = syn::parse_quote!(j),
            1 => publication.build = syn::parse_quote!(old_round),
            2 => publication.provider = syn::parse_quote!(sort),
            _ => publication.layout = vec![syn::parse_quote!([y, x])],
        }
        rejected(&program);
    }
}

#[test]
fn publication_rejects_wrong_key_component_and_missing_domain_epoch() {
    let mut wrong = program();
    wrong.structures.publications.0[1].columns = syn::parse_quote!([x]);
    rejected(&wrong);
    let mut wrong = program();
    wrong.structures.publications.0[1].columns = syn::parse_quote!([x, z]);
    rejected(&wrong);
    let mut wrong = program();
    wrong.structures.publications.0[1].domains.pop();
    rejected(&wrong);
    assert!(syn::parse_str::<Domain>("x: dictionary names").is_err());
    assert!(syn::parse_str::<Domain>("x: dictionary names -1").is_err());
}

#[test]
fn publication_cannot_relabel_unit_precision_abi_or_producer() {
    for mutation in 0..6 {
        let mut program = program();
        let stat = &mut program.structures.publications.0[0].statistic;
        match mutation {
            0 => stat.unit = Unit::DistinctKeys,
            1 => stat.precision = Precision::Estimate,
            2 => stat.accessor = syn::parse_quote!(recount_rows),
            3 => stat.producer = Producer::HashFinalize,
            4 => stat.placement = Placement::Cuda,
            _ => stat.computation = Computation::Additional,
        }
        rejected(&program);
    }
}

#[test]
fn declaration_checks_units_duplicate_slots_and_overlapping_exact_claims() {
    let mut wrong = program();
    wrong.structures.providers.declarations[1].facts.statistics[0].unit = Unit::DistinctKeys;
    rejected(&wrong);
    let mut wrong = program();
    let duplicate = wrong.structures.providers.declarations[1].facts.statistics[0].clone();
    wrong.structures.providers.declarations[1]
        .facts
        .statistics
        .push(duplicate);
    rejected(&wrong);
    for field in [Field::Keys, Field::MaxGroup] {
        let mut wrong = program();
        let declaration = &mut wrong.structures.providers.declarations[1].facts.statistics[1];
        declaration.field = field;
        declaration.unit = if field == Field::Keys {
            Unit::DistinctKeys
        } else {
            Unit::MaxBucketOccurrences
        };
        declaration.aggregation = Aggregation::Overlapping;
        wrong.structures.publications.0[1].statistic = declaration.clone();
        rejected(&wrong);
    }
}

#[test]
fn overlapping_keys_require_bound_or_recorded_computation() {
    for realization in 0..3 {
        let mut program = program();
        let declaration = &mut program.structures.providers.declarations[1]
            .facts
            .statistics[1];
        declaration.aggregation = Aggregation::Overlapping;
        match realization {
            0 => declaration.precision = Precision::UpperBound,
            1 => declaration.computation = Computation::Additional,
            _ => {
                declaration.aggregation = Aggregation::KeyMerge;
                declaration.producer = Producer::MergeFinalize;
            }
        }
        program.structures.publications.0[1].statistic = declaration.clone();
        structures::contract::check(&program).unwrap();
    }
}

#[test]
fn duplicate_names_fields_and_conflicting_epochs_are_rejected() {
    let mut wrong = program();
    wrong.structures.publications.0[1].name = syn::parse_quote!(rows_r);
    rejected(&wrong);
    let mut wrong = program();
    let mut duplicate = wrong.structures.publications.0[1].clone();
    duplicate.name = syn::parse_quote!(another_name);
    wrong.structures.publications.0.push(duplicate);
    rejected(&wrong);
    let mut wrong = program();
    wrong.structures.publications.0.push(syn::parse_quote!(
        bounds_r = build r provider index layout [x,y] columns [y]
            domains [y: dictionary names 6]
            stat extrema: bounds code_bounds exact run_finalize bounds_stat host optional build single;
    ));
    rejected(&wrong);
}

#[test]
fn reference_identity_detects_stale_version_domain_layout_precision_and_units() {
    let publication = program().structures.publications.0.remove(1);
    let expected = publication.reference();
    for mutation in 0..7 {
        let mut reference = expected.clone();
        match mutation {
            0 => reference.build = syn::parse_quote!(next_round),
            1 => reference.provider = syn::parse_quote!(other_index),
            2 => reference.layout = vec![syn::parse_quote!([y, x])],
            3 => reference.domains[1].epoch += 1,
            4 => reference.domains[1].encoding = syn::parse_quote!(other_encoding),
            5 => reference.precision = Precision::Estimate,
            _ => reference.unit = Unit::Occurrences,
        }
        assert_ne!(expected, reference);
    }
}

#[test]
fn selected_statistics_are_a_physical_preserving_refinement() {
    let target = program();
    let mut source = target.clone();
    source.structures.publications.0.clear();
    structures::check_rewrite(&source, &target).unwrap();
    assert_eq!(source.physical, target.physical);
}

#[test]
fn declaration_parser_covers_all_units_targets_and_statuses() {
    for spelling in [
        "input: input_rows occurrences estimate group_finalize input_stat host optional build single",
        "groups: groups groups exact group_finalize groups_stat host required build single",
        "maximum: max_group max_bucket_occurrences lower_bound hash_finalize max_stat cuda optional build disjoint",
        "blocks: blocks blocks exact scan_encode blocks_stat metal required build single",
        "runs: runs runs exact merge_finalize runs_stat host required build overlapping",
    ] {
        let declaration: StatDeclaration = syn::parse_str(spelling).unwrap();
        assert_eq!(
            syn::parse2::<StatDeclaration>(declaration.to_token_stream()).unwrap(),
            declaration
        );
        assert!(declaration.is_scalar());
    }
    assert!(
        syn::parse_str::<StatDeclaration>(
            "n: rows rows exact run_finalize count host required build single"
        )
        .is_err()
    );
}

#[test]
fn equality_only_provider_cannot_declare_code_bounds() {
    let mut wrong = program();
    wrong.structures.providers.declarations[1]
        .facts
        .reads
        .retain(|capability| *capability != structures::Capability::Ordered);
    rejected(&wrong);
    // Point membership and count publication remain valid without an ordered law.
    wrong.structures.providers.declarations[1]
        .facts
        .statistics
        .retain(|statistic| statistic.field != Field::Bounds);
    structures::contract::check(&wrong).unwrap();
}
