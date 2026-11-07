mod common;
use common::Stack as Program;
use cq_lang_sequence::{self as sequence, Guard, Term, structures};
use quote::ToTokens;

fn program() -> Program {
    syn::parse_quote! {
        physical {
            relation L(a:i32,b:i32); relation R(a:i32,b:i32,c:i32);
            l = pipe rename L@0 {a -> x,b -> y};
            r = let rename R@1 {a -> x,b -> y,c -> z};
            j = let canon probe first [x,y] l,r;
            return j as answer(x,y,z);
        }
        structures {
            providers {
                host_scan = HostScan { reads [], build typed };
                index = Index { reads [exact, membership], build typed,
                    statistics [count: rows occurrences exact run_finalize rows_stat host optional build single]
                };
                sort = Sort { reads [distinct], build inferred, shared };
            }
            publications {
                rows_r = build r provider index layout [x,y] columns [] domains []
                    stat count: rows occurrences exact run_finalize rows_stat host optional build single;
            }
            l host scan = @host_scan as HostScan [];
            r host scan = @host_scan as HostScan [];
            r = @index as Index [x,y];
            j rows = @sort as Sort; j canon = rows;
        }
        sequence {
            ready = store rows r;
            answer = choose [
                stat ready (rows_r build r provider index layout [x,y] columns [] domains [] field rows unit occurrences precision exact) < 10
                    => canon flatten lookup r[x,y] rows l,
                else => canon flatten lookup r[x,y] rows l
            ];
        }
    }
}

fn read(program: &mut Program) -> &mut sequence::stats::Guard {
    let Term::Choose(arms) = &mut program.sequence.definitions[1].term else {
        panic!()
    };
    let Guard::Statistic(read) = &mut arms[0].0 else {
        panic!()
    };
    read
}

fn reject(program: &Program) {
    structures::contract::check(&program.physical, &program.structures).unwrap();
    assert!(common::check(program).is_err());
}

#[test]
fn typed_statistic_guard_and_program_roundtrip() {
    let mut program = program();
    common::check(&program).unwrap();
    assert_eq!(
        syn::parse2::<Program>(program.to_token_stream()).unwrap(),
        program
    );
    for threshold in [0, 1, u64::MAX] {
        read(&mut program).threshold = threshold;
        for comparison in [
            sequence::stats::Comparison::Lt,
            sequence::stats::Comparison::Ge,
        ] {
            read(&mut program).comparison = comparison;
            common::check(&program).unwrap();
            let guard = read(&mut program).clone();
            assert_eq!(
                syn::parse2::<sequence::stats::Guard>(guard.to_token_stream()).unwrap(),
                guard
            );
        }
    }
}

#[test]
fn stale_statistic_references_are_rejected_independently_of_physical() {
    let mutations: &[fn(&mut structures::stats::Reference)] = &[
        |r| r.publication = syn::parse_quote!(missing),
        |r| r.build = syn::parse_quote!(l),
        |r| r.provider = syn::parse_quote!(host_scan),
        |r| r.layout = vec![syn::parse_quote!([y, x])],
        |r| r.columns = syn::parse_quote!([x]),
        |r| r.field = structures::stats::Field::Keys,
        |r| r.unit = structures::stats::Unit::DistinctKeys,
        |r| r.precision = structures::stats::Precision::Estimate,
    ];
    for mutation in mutations {
        let mut changed = program();
        mutation(&mut read(&mut changed).reference);
        reject(&changed);
    }
}

#[test]
fn statistic_read_requires_prior_flat_publication_and_explicit_fallback() {
    let mut changed = program();
    changed.sequence.definitions.swap(0, 1);
    reject(&changed);

    let mut changed = program();
    read(&mut changed).ready = syn::parse_quote!(missing);
    reject(&changed);

    let mut changed = program();
    changed.sequence.definitions[0].term = syn::parse_quote!(rows r);
    reject(&changed);

    let mut changed = program();
    changed.sequence.definitions[0].term = syn::parse_quote!(store rows l);
    reject(&changed);

    let mut changed = program();
    let Term::Choose(arms) = &mut changed.sequence.definitions[1].term else {
        panic!()
    };
    arms.pop();
    reject(&changed);
}

#[test]
fn statistic_missing_value_cannot_fall_through_to_another_guard() {
    for intervening in [Guard::WorkersLt(2), Guard::Ge(sequence::Len::Lit(0), 0)] {
        let mut changed = program();
        let Term::Choose(arms) = &mut changed.sequence.definitions[1].term else {
            panic!()
        };
        arms.insert(1, (intervening, arms[0].1.clone()));
        let error = common::check(&changed).unwrap_err().to_string();
        assert!(error.contains("BE-StatsRead"), "{error}");
    }
    let mut changed = program();
    let Term::Choose(arms) = &mut changed.sequence.definitions[1].term else {
        panic!()
    };
    arms.insert(1, arms[0].clone());
    reject(&changed);
}

#[test]
fn estimates_select_checked_shapes_but_are_not_length_or_capacity_operands() {
    let mut program = program();
    program.structures.structures.providers.declarations[1]
        .facts
        .statistics[0]
        .precision = structures::stats::Precision::Estimate;
    program.structures.structures.publications.0[0]
        .statistic
        .precision = structures::stats::Precision::Estimate;
    read(&mut program).reference.precision = structures::stats::Precision::Estimate;
    common::check(&program).unwrap();
    assert!(syn::parse_str::<sequence::Len>("stat ready (rows_r)").is_err());
    assert!(syn::parse_str::<Term>("store capacity stat ready (rows_r) rows r").is_err());
}

#[test]
fn host_statistic_read_cannot_enter_nonuniform_device_scope() {
    let mut program = program();
    let node = |name: &str, children| sequence::LevelNode {
        name: syn::parse_str(name).unwrap(),
        host_rust: true,
        collaborative: false,
        dynamic: true,
        width: None,
        shared_bytes: None,
        children,
    };
    let machine = sequence::Machine {
        gpu: None,
        layout: None,
        name: syn::parse_quote!(target),
        items: 1,
        key_partition_threshold: 10,
        key_partition_cache_threshold: 100,
        root: node("host", vec![node("device", vec![])]),
    };
    program.graded = Some(syn::parse_quote!(graded target { ready at host; answer at host; }));
    common::check_grading(&program, &machine).unwrap();
    program.graded = Some(syn::parse_quote!(graded target { ready at host; answer at device; }));
    common::check(&program).unwrap();
    assert!(common::check_grading(&program, &machine).is_err());
}
