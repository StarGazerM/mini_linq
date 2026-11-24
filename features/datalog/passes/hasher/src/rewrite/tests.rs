use super::*;
use quote::{ToTokens, quote};

const GROUPED: &[provider::HostRead] = &[
    provider::HostRead::Exact,
    provider::HostRead::Membership,
    provider::HostRead::Candidates,
];

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
    assert_eq!(keyed.negations.len(), 1);
    for negative in &keyed.negations {
        assert_eq!(
            negative.provider,
            syn::parse_quote!(@hash_set as ::cq_storage::HashSet<_>)
        );
        assert_eq!(negative.hasher, storage::Hasher::Keyed);
    }
    check_storage(&source, &keyed, Choice::Keyed).unwrap();
    let selected = storage(&keyed, Choice::Fx).unwrap();
    for layout in selected.layouts.iter().chain(&selected.delta_layouts) {
        assert_eq!(layout.hasher, storage::Hasher::Fx);
    }
    assert_eq!(
        selected.negations[0].provider,
        syn::parse_quote!(@hash_set as ::std::collections::HashSet<_, ::std::hash::BuildHasherDefault<::cq_datalog::rustc_hash::FxHasher>>)
    );
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
        GROUPED,
        &[],
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
        GROUPED,
        &[],
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
        // A negation key set spelled with a hasher other than its record.
        |p| {
            p.negations[0].provider.ty =
                syn::parse_quote!(::std::collections::HashSet<(i32,), ::cq_storage::HashState>)
        },
        |p| p.negations[0].provider.token = syn::parse_quote!(hash_map),
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
    // Leaving a negation key set at Fx under a keyed selection is rejected.
    let mut mutation = keyed.clone();
    mutation.negations[0] = source.negations[0].clone();
    assert!(check_storage(&source, &mutation, Choice::Keyed).is_err());
    // Hasher selection may not change the projection a key set realizes.
    let mut mutation = selected.clone();
    mutation.negations[0].columns.clear();
    assert!(check_storage(&source, &mutation, Choice::Fx).is_err());
}

#[test]
fn verbatim_negation_key_sets_stay_opaque() {
    let mut source = fixture();
    source.tokens.push(provider::declared(
        "custom_set",
        provider::Spelling::Verbatim,
        Some(storage::LayoutProtocol::Set),
        provider::Admission::None,
        true,
        provider::TaskHasher::Fx,
        &[provider::HostRead::Membership],
        &[],
    ));
    source.negations[0].provider =
        syn::parse_quote!(@custom_set as ::std::collections::HashSet<_, Custom>);
    storage::contract::check(&source).unwrap();
    let keyed = storage(&source, Choice::Keyed).unwrap();
    assert_eq!(keyed.negations[0], source.negations[0]);
    check_storage(&source, &keyed, Choice::Keyed).unwrap();
}

#[test]
fn fork_copies_selected_negation_key_sets() {
    for choice in [Choice::Fx, Choice::Keyed] {
        let selected = storage(&fixture(), choice).unwrap();
        let round = cq_datalog_fork::compile(&selected).unwrap();
        let keys: Vec<_> = round.context.strata.iter().flat_map(|s| &s.keys).collect();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].provider, selected.negations[0].provider);
        assert_eq!(keys[0].hasher, selected.negations[0].hasher);
    }
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
    let output = cq_stage::rewrite::<storage::Program>(
        quote!(arguments { keyed }; pipeline { downstream::consume }; #source),
        |source, args| storage(source, syn::parse2(args)?),
    )
    .to_string();
    assert!(output.contains("downstream :: consume"));
    assert!(output.contains("HashState"));
    assert!(output.contains("cq_storage :: HashSet < _ >"));
    assert!(!output.contains("compile_error"));
    let error = cq_stage::rewrite::<storage::Program>(
        quote!(arguments { unsupported }; pipeline { downstream::consume }; #source),
        |source, args| storage(source, syn::parse2(args)?),
    )
    .to_string();
    assert!(error.contains("compile_error"));
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
        GROUPED,
        &[],
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
