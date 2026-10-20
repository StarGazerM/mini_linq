use super::*;
use quote::{ToTokens, quote};

fn fixture() -> storage::Program {
    let source = syn::parse_quote! {
        struct Closure;
        relation Edge(x:i32,y:i32) = (edges);
        relation Blocked(x:i32) = (blocked);
        relation Reach(x:i32,y:i32);
        relation Allowed(x:i32,y:i32);
        output Allowed;
        Reach(*x,*y) :- Edge(x,y);
        Reach(*x,*z) :- Reach(x,y), Reach(y,z);
        Allowed(*x,*y) :- Reach(x,y), !Blocked(y);
    };
    let strata = cq_datalog_stratify::compile(&source).unwrap();
    let incremental = cq_datalog_incremental::compile(&strata).unwrap();
    cq_datalog_storage::compile(&incremental).unwrap()
}

#[test]
fn selects_all_applicable_fields_and_restores_base_exactly() {
    let source = fixture();
    assert!(!source.delta_layouts.is_empty());
    let selected = storage(&source, Choice::Fx).unwrap();
    let state: Type = syn::parse_quote!(::std::hash::BuildHasherDefault<::rustc_hash::FxHasher>);
    assert_eq!(
        selected.providers[0].canonical_set,
        syn::parse_quote!(::std::collections::HashSet<R, #state>)
    );
    assert_eq!(selected.providers[0].seen, source.providers[0].seen);
    assert_eq!(
        selected.providers[2].seen,
        syn::parse_quote!(::std::collections::HashSet<(i32, i32,), #state>)
    );
    assert_eq!(
        selected.providers[2].canonical_set,
        source.providers[2].canonical_set
    );
    for layout in selected.layouts.iter().chain(&selected.delta_layouts) {
        assert!(
            layout
                .provider
                .to_token_stream()
                .to_string()
                .contains("FxHasher")
        );
    }
    check_storage(&source, &selected, Choice::Fx).unwrap();
    assert_eq!(storage(&selected, Choice::Fx).unwrap(), selected);
    assert_eq!(storage(&selected, Choice::Default).unwrap(), source);
    assert_eq!(storage(&source, Choice::Default).unwrap(), source);
    let replay: storage::Program = syn::parse2(selected.to_token_stream()).unwrap();
    assert_eq!(replay, selected);
}

#[test]
fn preserves_ordered_custom_and_nested_providers() {
    let mut source = fixture();
    source.layouts[0].provider =
        syn::parse_quote!(::std::collections::BTreeMap<(i32, i32), Vec<()>>);
    source.delta_layouts[0].provider =
        syn::parse_quote!(Custom<::std::collections::HashMap<i32, i32>>);
    source.providers[0].canonical_set = syn::parse_quote!(::std::collections::BTreeSet<R>);
    source.providers[2].seen = syn::parse_quote!(MySeen<(i32, i32)>);
    let selected = storage(&source, Choice::Fx).unwrap();
    assert_eq!(selected.layouts[0], source.layouts[0]);
    assert_eq!(selected.delta_layouts[0], source.delta_layouts[0]);
    assert_eq!(selected.providers[0], source.providers[0]);
    assert_eq!(selected.providers[2], source.providers[2]);
    let mut mutation = selected.clone();
    mutation.providers[2].seen = syn::parse_quote!(::std::collections::BTreeSet<(i32, i32)>);
    assert!(check_storage(&source, &mutation, Choice::Fx).is_err());
}

#[test]
fn accepts_explicit_std_hashers_and_post_sharding_order() {
    let mut source = fixture();
    source.providers[0].canonical_set = syn::parse_quote!(std::collections::HashSet<R>);
    source.providers[2].seen =
        syn::parse_quote!(::cq_datalog_parallel_runtime::ShardedSeen<(i32, i32,)>);
    source.providers[3].seen =
        syn::parse_quote!(cq_datalog_parallel_runtime::ShardedSeen<(i32, i32,), CustomState>);
    source.layouts[0].provider =
        syn::parse_quote!(std::collections::HashMap<(i32, i32,), Vec<()>, CustomState>);
    let selected = storage(&source, Choice::Fx).unwrap();
    let state: Type = syn::parse_quote!(::std::hash::BuildHasherDefault<::rustc_hash::FxHasher>);
    for fields in &selected.providers[2..] {
        assert_eq!(
            fields.seen,
            syn::parse_quote!(::cq_datalog_parallel_runtime::ShardedSeen<(i32, i32,), #state>)
        );
    }
    let restored = storage(&selected, Choice::Default).unwrap();
    for fields in &restored.providers[2..] {
        assert_eq!(
            fields.seen,
            syn::parse_quote!(::cq_datalog_parallel_runtime::ShardedSeen<(i32, i32,)>)
        );
    }
    assert_eq!(
        restored.providers[0].canonical_set,
        syn::parse_quote!(::cq_storage::HashSet<R>)
    );
    assert_eq!(
        restored.layouts[0].provider,
        syn::parse_quote!(::std::collections::HashMap<(i32, i32,), Vec<()>, ::cq_storage::HashState>)
    );
}

#[test]
fn independent_storage_checker_rejects_mutations() {
    let source = fixture();
    let selected = storage(&source, Choice::Fx).unwrap();
    let mutations: &[fn(&mut storage::Program)] = &[
        |p| p.providers[0].canonical_set = syn::parse_quote!(::cq_storage::HashSet<R>),
        |p| p.providers[2].seen = syn::parse_quote!(::cq_storage::HashSet<(i32, i32,)>),
        |p| p.providers[2].seen = syn::parse_quote!(::std::collections::HashSet<(u64,), ::std::hash::BuildHasherDefault<::rustc_hash::FxHasher>>),
        |p| {
            p.layouts[0].provider =
                syn::parse_quote!(::std::collections::HashMap<(), Vec<()>, ::cq_storage::HashState>)
        },
        |p| {
            p.delta_layouts[0].provider =
                syn::parse_quote!(::std::collections::BTreeMap<(), Vec<()>>)
        },
        |p| p.providers[2].all_init = syn::parse_quote!(Vec::with_capacity(42)),
        |p| p.providers[0].canonical_name = syn::parse_quote!(DifferentCanonical),
        |p| p.layouts.push(p.layouts[0].clone()),
        |p| p.reads[0].index = usize::MAX,
        |p| p.layouts[0].maintenance = syn::parse_quote!(replace_each_run),
        |p| p.negations.clear(),
        |p| p.incremental.strata.source.name = syn::parse_quote!(Different),
    ];
    for (index, mutate) in mutations.iter().enumerate() {
        let mut mutation = selected.clone();
        mutate(&mut mutation);
        assert!(
            check_storage(&source, &mutation, Choice::Fx).is_err(),
            "mutation {index}"
        );
    }
    assert!(check_storage(&source, &selected, Choice::Default).is_err());
}

#[test]
fn round_selects_only_negation_key_providers() {
    let source = cq_datalog_fork::compile(&storage(&fixture(), Choice::Fx).unwrap()).unwrap();
    assert_eq!(
        source
            .context
            .strata
            .iter()
            .map(|s| s.keys.len())
            .sum::<usize>(),
        1
    );
    let selected = round(&source, Choice::Fx).unwrap();
    assert_eq!(selected.storage, source.storage);
    assert_eq!(selected.holes, source.holes);
    let key = selected
        .context
        .strata
        .iter()
        .flat_map(|s| &s.keys)
        .next()
        .unwrap();
    assert_eq!(
        key.provider,
        syn::parse_quote!(::std::collections::HashSet<_, ::std::hash::BuildHasherDefault<::rustc_hash::FxHasher>>)
    );
    assert_eq!(round(&selected, Choice::Fx).unwrap(), selected);
    assert_eq!(round(&selected, Choice::Default).unwrap(), source);
    let replay: rounds::Program = syn::parse2(selected.to_token_stream()).unwrap();
    assert_eq!(replay, selected);
    let mut ordered = source.clone();
    ordered
        .context
        .strata
        .iter_mut()
        .flat_map(|s| &mut s.keys)
        .next()
        .unwrap()
        .provider = syn::parse_quote!(::std::collections::BTreeSet<_>);
    assert_eq!(round(&ordered, Choice::Fx).unwrap(), ordered);
}

#[test]
fn independent_round_checker_rejects_mutations() {
    let source = cq_datalog_fork::compile(&fixture()).unwrap();
    let selected = round(&source, Choice::Fx).unwrap();
    let mutations: &[fn(&mut rounds::Program)] = &[
        |p| {
            p.context
                .strata
                .iter_mut()
                .flat_map(|s| &mut s.keys)
                .next()
                .unwrap()
                .provider = syn::parse_quote!(::cq_storage::HashSet<_>)
        },
        |p| {
            p.context
                .strata
                .iter_mut()
                .flat_map(|s| &mut s.keys)
                .next()
                .unwrap()
                .provider = syn::parse_quote!(::std::collections::HashSet<(i32,), ::std::hash::BuildHasherDefault<::rustc_hash::FxHasher>>)
        },
        |p| p.policy.timing = syn::parse_quote!(phases),
        |p| {
            p.context
                .strata
                .iter_mut()
                .flat_map(|s| &mut s.keys)
                .next()
                .unwrap()
                .columns
                .clear()
        },
        |p| {
            p.context
                .strata
                .iter_mut()
                .flat_map(|s| &mut s.keys)
                .next()
                .unwrap()
                .hole = usize::MAX
        },
        |p| p.holes[0].pipeline = syn::parse_quote!(pipeline { custom::emit }),
    ];
    for (index, mutate) in mutations.iter().enumerate() {
        let mut mutation = selected.clone();
        mutate(&mut mutation);
        assert!(
            check_round(&source, &mutation, Choice::Fx).is_err(),
            "mutation {index}"
        );
    }
    assert!(check_round(&source, &selected, Choice::Default).is_err());
}

#[test]
fn reports_bad_arguments_and_malformed_known_providers() {
    for args in [
        quote!(),
        quote!(random),
        quote!(fx, default),
        quote!(fx extra),
        quote!(42),
    ] {
        assert!(syn::parse2::<Choice>(args).is_err());
    }
    for mut ty in [
        syn::parse_quote!(::std::collections::HashMap<i32>),
        syn::parse_quote!(::std::collections::HashSet),
        syn::parse_quote!(::std::collections::HashSet<'a>),
        syn::parse_quote!(::cq_storage::HashSet<i32, CustomState>),
        syn::parse_quote!(::cq_datalog_parallel_runtime::ShardedSeen<i32, S, Extra>),
    ] {
        assert!(select(&mut ty, Choice::Fx).is_err());
    }
    let mut malformed = fixture();
    malformed.providers.clear();
    assert!(storage(&malformed, Choice::Fx).is_err());
}

#[test]
fn stage_protocol_forwards_checked_provider_selections() {
    let source = fixture();
    let output = cq_stage::rewrite::<storage::Program>(
        quote!(arguments { fx }; pipeline { downstream::consume }; #source),
        |source, args| storage(source, syn::parse2(args)?),
    )
    .to_string();
    assert!(output.contains("downstream :: consume"));
    assert!(output.contains("FxHasher"));
    assert!(!output.contains("compile_error"));
    let round_source = cq_datalog_fork::compile(&source).unwrap();
    let output = cq_stage::rewrite::<rounds::Program>(
        quote!(arguments { fx }; pipeline { downstream::consume }; #round_source),
        |source, args| round(source, syn::parse2(args)?),
    )
    .to_string();
    assert!(output.contains("downstream :: consume"));
    assert!(output.contains("FxHasher"));
    assert!(!output.contains("compile_error"));
    let error = cq_stage::rewrite::<storage::Program>(
        quote!(arguments { unsupported }; pipeline { downstream::consume }; #source),
        |source, args| storage(source, syn::parse2(args)?),
    )
    .to_string();
    assert!(error.contains("compile_error"));
    assert!(error.contains("expected hasher choice"));
}
