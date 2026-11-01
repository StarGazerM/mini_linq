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

fn first_key(program: &mut rounds::Program) -> &mut rounds::KeySet {
    program
        .context
        .strata
        .iter_mut()
        .flat_map(|s| &mut s.keys)
        .next()
        .unwrap()
}

#[test]
fn selects_both_directions_and_restores_fx_base_exactly() {
    let source = fixture();
    assert!(!source.delta_layouts.is_empty());
    let keyed = storage(&source, Choice::Keyed).unwrap();
    assert_ne!(keyed, source);
    assert_eq!(storage(&keyed, Choice::Keyed).unwrap(), keyed);
    let canonical = &keyed.layouts[keyed.providers[2].canonical];
    // Full-key layouts are `hash_set`s; the keyed spelling is the cq_storage alias.
    assert_eq!(
        canonical.provider,
        syn::parse_quote!(@hash_set as ::cq_storage::HashSet<(i32, i32,)>)
    );
    assert_eq!(keyed.providers, source.providers);
    assert_eq!(keyed.tokens, source.tokens);
    for layout in keyed.layouts.iter().chain(&keyed.delta_layouts) {
        assert_eq!(layout.hasher, storage::Hasher::Keyed);
    }
    check_storage(&source, &keyed, Choice::Keyed).unwrap();
    let selected = storage(&keyed, Choice::Fx).unwrap();
    for layout in selected.layouts.iter().chain(&selected.delta_layouts) {
        assert_eq!(layout.hasher, storage::Hasher::Fx);
    }
    check_storage(&source, &selected, Choice::Fx).unwrap();
    assert_eq!(storage(&selected, Choice::Fx).unwrap(), selected);
    assert_eq!(selected, source);
    assert_eq!(storage(&selected, Choice::Keyed).unwrap(), keyed);
    let replay: storage::Program = syn::parse2(selected.to_token_stream()).unwrap();
    assert_eq!(replay, selected);
}

#[test]
fn verbatim_tokens_stay_opaque() {
    // An authored token without a spelling function is never respelled,
    // whatever its Rust spelling names.
    let mut source = fixture();
    source.tokens.push(provider::declared(
        "custom_map",
        provider::Spelling::Verbatim,
        Some(storage::LayoutProtocol::GroupedMap),
        provider::Admission::None,
        true,
        provider::TaskHasher::Fx,
    ));
    let index = source
        .layouts
        .iter()
        .position(|layout| layout.protocol == storage::LayoutProtocol::GroupedMap)
        .unwrap();
    source.layouts[index].provider = syn::parse_quote!(
        @custom_map as ::std::collections::HashMap<(i32,), Vec<(i32,)>, Custom>
    );
    storage::contract::check(&source).unwrap();
    let keyed = storage(&source, Choice::Keyed).unwrap();
    assert_eq!(keyed.layouts[index], source.layouts[index]);
    check_storage(&source, &keyed, Choice::Keyed).unwrap();
}

#[test]
fn sharded_tokens_select_hashers_without_changing_key_or_payload() {
    let mut source = fixture();
    source.tokens.push(provider::declared(
        "sharded_layout",
        provider::Spelling::Sharded,
        Some(storage::LayoutProtocol::Bulk),
        provider::Admission::ShardedHash,
        true,
        provider::TaskHasher::Layout,
    ));
    let canonical = source.providers[2].canonical;
    let layout = &mut source.layouts[canonical];
    layout.protocol = storage::LayoutProtocol::Bulk;
    layout.provider = syn::parse_quote!(
        @sharded_layout as ::cq_datalog_parallel_runtime::ShardedLayout<(i32, i32,), (),
            ::std::hash::BuildHasherDefault<::cq_datalog::rustc_hash::FxHasher>>
    );
    storage::contract::check(&source).unwrap();
    let keyed = storage(&source, Choice::Keyed).unwrap();
    assert_eq!(
        keyed.layouts[canonical].provider,
        syn::parse_quote!(
            @sharded_layout as ::cq_datalog_parallel_runtime::ShardedLayout<(i32, i32,), (), ::cq_storage::HashState>
        )
    );
    check_storage(&source, &keyed, Choice::Keyed).unwrap();
    assert_eq!(storage(&keyed, Choice::Fx).unwrap(), source);
}

#[test]
fn independent_storage_checker_rejects_mutations() {
    let source = fixture();
    let selected = storage(&source, Choice::Fx).unwrap();
    let mutations: &[fn(&mut storage::Program)] = &[
        |p| p.providers[2].canonical = 0,
        |p| p.providers[2].next_init = syn::parse_quote!(Vec::with_capacity(42)),
        // A spelling inconsistent with the recorded hasher state.
        |p| {
            p.layouts[0].provider.ty =
                syn::parse_quote!(::std::collections::HashMap<(), Vec<()>, ::cq_storage::HashState>)
        },
        |p| p.delta_layouts[0].provider.token = syn::parse_quote!(hash_set),
        |p| p.layouts.push(p.layouts[0].clone()),
        |p| p.reads[0].index = usize::MAX,
        |p| p.layouts[0].maintenance = syn::parse_quote!(replace_each_run),
        |p| p.negations.clear(),
        |p| {
            p.tokens.pop();
        },
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
    assert!(check_storage(&source, &selected, Choice::Keyed).is_err());
    let keyed = storage(&source, Choice::Keyed).unwrap();
    assert!(check_storage(&source, &keyed, Choice::Fx).is_err());
    for (index, mutate) in mutations.iter().enumerate().skip(2) {
        let mut mutation = keyed.clone();
        mutate(&mut mutation);
        assert!(
            check_storage(&source, &mutation, Choice::Keyed).is_err(),
            "keyed mutation {index}"
        );
    }
    for index in [0, 2] {
        let canonical = source.providers[index].canonical;
        let mut mutation = keyed.clone();
        mutation.layouts[canonical] = source.layouts[canonical].clone();
        assert!(check_storage(&source, &mutation, Choice::Keyed).is_err());
    }
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
    let keyed = round(&source, Choice::Keyed).unwrap();
    assert_ne!(keyed, source);
    assert_eq!(round(&keyed, Choice::Keyed).unwrap(), keyed);
    for key in keyed.context.strata.iter().flat_map(|s| &s.keys) {
        assert_eq!(
            key.provider,
            syn::parse_quote!(@hash_set as ::cq_storage::HashSet<_>)
        );
        assert_eq!(key.hasher, storage::Hasher::Keyed);
    }
    let mut selected = round(&keyed, Choice::Fx).unwrap();
    assert_eq!(selected.storage, source.storage);
    assert_eq!(selected.holes, source.holes);
    assert_eq!(
        first_key(&mut selected).provider,
        syn::parse_quote!(@hash_set as ::std::collections::HashSet<_, ::std::hash::BuildHasherDefault<::cq_datalog::rustc_hash::FxHasher>>)
    );
    assert_eq!(round(&selected, Choice::Fx).unwrap(), selected);
    assert_eq!(selected, source);
    assert_eq!(round(&selected, Choice::Keyed).unwrap(), keyed);
    let replay: rounds::Program = syn::parse2(selected.to_token_stream()).unwrap();
    assert_eq!(replay, selected);
}

#[test]
fn independent_round_checker_rejects_mutations() {
    let source = cq_datalog_fork::compile(&fixture()).unwrap();
    let selected = round(&source, Choice::Fx).unwrap();
    let mutations: &[fn(&mut rounds::Program)] = &[
        |p| first_key(p).hasher = storage::Hasher::Keyed,
        |p| first_key(p).provider.ty = syn::parse_quote!(::std::collections::HashSet<(i32,), ::std::hash::BuildHasherDefault<::cq_datalog::rustc_hash::FxHasher>>),
        |p| p.policy.timing = syn::parse_quote!(phases),
        |p| first_key(p).columns.clear(),
        |p| first_key(p).hole = usize::MAX,
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
    assert!(check_round(&source, &selected, Choice::Keyed).is_err());
    let keyed = round(&source, Choice::Keyed).unwrap();
    assert!(check_round(&source, &keyed, Choice::Fx).is_err());
    for (index, mutate) in mutations.iter().enumerate().skip(1) {
        let mut mutation = keyed.clone();
        mutate(&mut mutation);
        assert!(
            check_round(&source, &mutation, Choice::Keyed).is_err(),
            "keyed mutation {index}"
        );
    }
    let mut mutation = keyed.clone();
    mutation.context = source.context.clone();
    assert!(check_round(&source, &mutation, Choice::Keyed).is_err());
}

#[test]
fn reports_bad_arguments_and_undeclared_tokens() {
    for args in [
        quote!(),
        quote!(random),
        quote!(default),
        quote!(fx, keyed),
        quote!(fx extra),
        quote!(42),
    ] {
        assert!(syn::parse2::<Choice>(args).is_err());
    }
    let mut malformed = fixture();
    malformed.providers.clear();
    assert!(storage(&malformed, Choice::Fx).is_err());
    let mut undeclared = fixture();
    undeclared.layouts[0].provider.token = syn::parse_quote!(unknown);
    assert!(storage(&undeclared, Choice::Fx).is_err());
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
    let output = cq_stage::rewrite::<storage::Program>(
        quote!(arguments { keyed }; pipeline { downstream::consume }; #source),
        |source, args| storage(source, syn::parse2(args)?),
    )
    .to_string();
    assert!(output.contains("downstream :: consume"));
    assert!(output.contains("HashState"));
    assert!(!output.contains("compile_error"));
    let output = cq_stage::rewrite::<rounds::Program>(
        quote!(arguments { keyed }; pipeline { downstream::consume }; #round_source),
        |source, args| round(source, syn::parse2(args)?),
    )
    .to_string();
    assert!(output.contains("downstream :: consume"));
    assert!(output.contains("cq_storage :: HashSet"));
    assert!(!output.contains("compile_error"));
    let error = cq_stage::rewrite::<storage::Program>(
        quote!(arguments { unsupported }; pipeline { downstream::consume }; #source),
        |source, args| storage(source, syn::parse2(args)?),
    )
    .to_string();
    assert!(error.contains("compile_error"));
    assert!(error.contains("expected hasher choice"));
}

#[test]
fn run_layout_tokens_stay_opaque() {
    // Run layouts declare no hasher state; selection leaves them unchanged.
    let mut source = fixture();
    source.tokens.push(provider::declared(
        "run_layout",
        provider::Spelling::Runs,
        Some(storage::LayoutProtocol::Bulk),
        provider::Admission::SortedSubtract,
        false,
        provider::TaskHasher::Fx,
    ));
    let canonical = source.providers[2].canonical;
    let layout = &mut source.layouts[canonical];
    layout.protocol = storage::LayoutProtocol::Bulk;
    layout.hasher = storage::Hasher::None;
    layout.provider = syn::parse_quote!(
        @run_layout as ::cq_datalog_parallel_runtime::RunLayout<(i32, i32,), ()>
    );
    storage::contract::check(&source).unwrap();
    for choice in [Choice::Fx, Choice::Keyed] {
        let target = storage(&source, choice).unwrap();
        assert_eq!(target.layouts[canonical], source.layouts[canonical]);
    }
}
