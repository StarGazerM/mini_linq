//! Exact erasure, authored expected CQ, and independent mutation rejection.
use cq_datalog_partition::{check, compile};
use cq_lang_datalog_round::{self as round, InputSource, Program, cq::BodyItem};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};

fn lower(tokens: TokenStream) -> Program {
    let source = syn::parse2(tokens).unwrap();
    let strata = cq_datalog_stratify::compile(&source).unwrap();
    let incremental = cq_datalog_incremental::compile(&strata).unwrap();
    let storage = cq_datalog_storage::compile(&incremental).unwrap();
    cq_datalog_fork::compile(&storage).unwrap()
}
fn authored() -> Program {
    lower(quote! {
        struct Closure;
        relation Edge(x:i32,y:i32) = (edges);
        relation Blocked(x:i32,y:i32) = (blocked);
        relation Reach(x:i32,y:i32);
        output Reach;
        Reach(*x,*y) :- Edge(x,y), !Blocked(y,_);
        Reach(*x,*z) :- Reach(x,y), Edge(y,z), !Blocked(z,_);
    })
}
fn rejected(change: impl FnOnce(&mut Program)) {
    let source = authored();
    let mut target = compile(&source).unwrap();
    change(&mut target);
    assert!(check(&source, &target).is_err());
}

#[test]
fn partition_metadata_selects_flat_slices_and_preserves_schemas() {
    let source = authored();
    let target = compile(&source).unwrap();
    check(&source, &target).unwrap();
    assert_eq!(round::erase(&target).unwrap(), source);
    assert_eq!(
        syn::parse2::<Program>(target.to_token_stream()).unwrap(),
        target
    );
    let stratum = &target.context.strata[0];
    for (rule, expected_source) in [
        (&stratum.seeds[0], InputSource::Layout(0)),
        (&stratum.steps[0], InputSource::Delta(2)),
    ] {
        let scan = rule.scan.as_ref().unwrap();
        assert_eq!(scan.source, expected_source);
        let execution = target
            .execution
            .iter()
            .find(|entry| entry.hole == rule.hole)
            .unwrap();
        let hole = &target.holes[rule.hole].cq;
        let declaration = &hole.declarations[hole.declarations.len() - 2];
        assert_eq!(declaration.name, execution.binding);
        assert_eq!(
            declaration.columns,
            source.context.relations[0].declaration.columns
        );
        let chunk = &execution.binding;
        assert_eq!(declaration.value, Some(syn::parse_quote!(= (#chunk))));
        let BodyItem::Positive { atom } = &hole.query.body[0] else {
            panic!("positive scan")
        };
        let BodyItem::Positive { atom: original } = &source.holes[rule.hole].cq.query.body[0]
        else {
            unreachable!()
        };
        assert_eq!(atom.relation, execution.binding);
        assert_eq!(atom.args, original.args);
    }
    // Key projections are neither scheduled for workers nor rewritten.
    for key in &stratum.keys {
        assert_eq!(target.holes[key.hole], source.holes[key.hole]);
        assert!(!target.execution.iter().any(|entry| entry.hole == key.hole));
    }
}

#[test]
fn checker_rejects_metadata_binding_and_occurrence_mutations() {
    rejected(|target| {
        target.execution.pop();
    });
    rejected(|target| target.execution.push(target.execution[0].clone()));
    rejected(|target| target.execution[0].hole = usize::MAX);
    rejected(|target| target.execution[0].occurrence = usize::MAX);
    rejected(|target| target.execution[0].strategy = syn::parse_quote!(unknown));
    rejected(|target| target.execution[0].binding = syn::parse_quote!(Edge));
    rejected(|target| target.context.strata[0].steps[0].scan = None);
    rejected(|target| {
        target.context.strata[0].steps[0]
            .scan
            .as_mut()
            .unwrap()
            .source = InputSource::Delta(usize::MAX)
    });
    rejected(|target| {
        target.context.strata[0].seeds[0]
            .scan
            .as_mut()
            .unwrap()
            .source = InputSource::Layout(1)
    });
    rejected(|target| {
        let entry = &target.execution[0];
        let chunk = &entry.binding;
        target.holes[entry.hole]
            .cq
            .declarations
            .iter_mut()
            .find(|declaration| declaration.name == *chunk)
            .unwrap()
            .value = Some(syn::parse_quote!(= (&#chunk)));
    });
    rejected(|target| {
        let entry = &target.execution[0];
        if let BodyItem::Positive { atom } = &mut target.holes[entry.hole].cq.query.body[0] {
            atom.relation = syn::parse_quote!(Edge);
        }
    });
    rejected(|target| {
        target.holes[0].pipeline.passes[0].arguments = quote!(no_reduce);
    });
    rejected(|target| target.policy.timing = syn::parse_quote!(phases));
}

#[test]
fn partition_rewrites_only_first_occurrence_of_a_self_join() {
    let source = lower(quote! {
        struct SelfJoin;
        relation E(x:i32,y:i32) = (edges);
        relation R(x:i32,y:i32);
        output R;
        R(*x,*z) :- E(x,y), E(a,z);
    });
    let target = compile(&source).unwrap();
    let rule = &target.context.strata[0].seeds[0];
    let chunk = &target.execution[0].binding;
    let layout = &target.context.layouts[0].name;
    let first: BodyItem = syn::parse_quote!(#chunk(x,y));
    let second: BodyItem = syn::parse_quote!(#layout(a,z));
    assert_eq!(
        target.holes[rule.hole]
            .cq
            .query
            .body
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        vec![first, second]
    );
    let mut changed = target.clone();
    changed.holes[rule.hole].cq.query.body[1] = syn::parse_quote!(#chunk(a,z));
    assert!(check(&source, &changed).is_err());
}

#[test]
fn a_leading_guard_is_not_moved_to_enable_partitioning() {
    let source = lower(quote! {
        struct GuardFirst;
        relation E(x:i32) = (edges);
        relation R(x:i32);
        output R;
        R(*x) :- if enabled, E(x);
    });
    let target = compile(&source).unwrap();
    assert!(target.execution.is_empty());
    assert_eq!(target, source);
}

#[test]
fn keyed_initial_scans_and_key_holes_are_not_partitioned() {
    let source = lower(quote! {
        struct Selection;
        relation E(x:i32,y:i32) = (edges);
        relation Blocked(x:i32) = (blocked);
        relation R(y:i32);
        output R;
        R(*y) :- E(7,y), !Blocked(y);
    });
    let target = compile(&source).unwrap();
    assert!(target.execution.is_empty());
    assert_eq!(target, source);
}

#[test]
fn bindings_are_fresh_per_rule_and_avoid_source_identifiers() {
    let mut source = authored();
    // Reconstruct the source so all embedded observer copies remain coherent.
    let mut input = source.storage.incremental.strata.source.clone();
    input.observer = Some(syn::parse_quote!(__cq_datalog_parallel_chunk_0));
    source = lower(input.to_token_stream());
    let target = compile(&source).unwrap();
    assert_eq!(target.execution.len(), 2);
    assert_ne!(target.execution[0].binding, target.execution[1].binding);
    for entry in &target.execution {
        assert_ne!(entry.binding, "__cq_datalog_parallel_chunk_0");
    }
    assert!(compile(&target).is_err());
    assert!(cq_datalog_partition::compile_with(&source, quote!(unexpected)).is_err());
}

#[test]
fn empty_output_only_program_is_an_identity_rewrite() {
    let source = lower(quote! {
        struct Input;
        relation E(x:i32) = (edges);
        output E;
    });
    assert_eq!(compile(&source).unwrap(), source);
}

#[test]
fn partition_requires_slice_backed_delta_and_layout_buckets() {
    let mutations: &[fn(&mut Program)] = &[
        |program| {
            let provider = &mut program.storage.providers[2];
            provider.delta = syn::parse_quote!(::std::collections::VecDeque<(i32, i32,)>);
            provider.next = provider.delta.clone();
            provider.delta_init = syn::parse_quote!(::std::collections::VecDeque::new());
            provider.next_init = provider.delta_init.clone();
            program.context.relations[2].providers = provider.clone();
        },
        |program| {
            let provider = syn::parse_quote!(
                ::std::collections::BTreeMap<(), ::std::collections::VecDeque<(i32, i32,)>>
            );
            program.storage.layouts[0].provider = provider;
            program.context.layouts[0].provider = program.storage.layouts[0].provider.clone();
        },
        |program| {
            program.storage.layouts[0].provider = syn::parse_quote!(custom::GroupedRows);
            program.context.layouts[0].provider = program.storage.layouts[0].provider.clone();
        },
    ];
    for mutate in mutations {
        let mut source = authored();
        let mut target = compile(&source).unwrap();
        mutate(&mut source);
        mutate(&mut target);
        round::contract::check(&source).unwrap();
        round::contract::check(&target).unwrap();
        let error = check(&source, &target).unwrap_err();
        assert!(error.to_string().contains("slice partition requires"));
        assert!(compile(&source).is_err());
    }
}

#[test]
fn ordered_maps_with_vector_buckets_remain_partitionable() {
    let mut source = authored();
    source.storage.layouts[0].provider = syn::parse_quote!(
        ::std::collections::BTreeMap<(), ::std::vec::Vec<(i32, i32,)>>
    );
    source.context.layouts[0].provider = source.storage.layouts[0].provider.clone();
    let target = compile(&source).unwrap();
    check(&source, &target).unwrap();
}
