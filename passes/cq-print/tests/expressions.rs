use cq_print::expressions::relational_condition_with;
#[test]
fn relational_equalities_require_the_leibniz_operation() {
    let theta = syn::parse_quote!(x = y, z = -5);
    let printed = relational_condition_with(&theta, &Default::default()).to_string();
    assert!(printed.contains("relational_eq (x , y)"));
    assert!(printed.contains("relational_eq (z , & (- 5))"));
}

#[test]
fn column_dependencies_cover_equalities_constraints_and_opaque_rust() {
    let theta = syn::parse_quote!(x = y, z >= @(floor), w in {1, 2});
    assert_eq!(
        cq_print::columns_used(&theta),
        ["w", "x", "y", "z"].map(String::from).into()
    );
    let heading = ["x", "y", "z"].map(String::from);
    let expression = syn::parse_quote!({
        let y = 1;
        x + y
    });
    assert_eq!(
        cq_print::columns_used_rust(&expression, &heading, None),
        ["x"].map(String::from).into()
    );
    let expression = syn::parse_quote!(opaque!(x));
    assert_eq!(
        cq_print::columns_used_rust(&expression, &heading, None),
        heading.into()
    );
}

fn membership_plan() -> cq_lang_structures::Program {
    let source = syn::parse_quote! { physical {
        relation L(a: ::opaque::KeyAlias); relation R(a: ::opaque::KeyAlias,b:i64);
        l = pipe rename L@0 {a -> x};
        r = pipe rename R@1 {a -> x,b -> y};
        selected = let range select r if (y > 1);
        joined = pipe probe first [x] l,selected;
        result = pipe canon project joined keep {x};
        return result as answer(x);
    }};
    cq_structure_selection::compile_with(&source, quote::quote!(positional(L, R))).unwrap()
}

#[test]
fn explicit_providers_dispatch_build_capabilities_without_type_recognition() {
    use quote::{ToTokens, quote};
    let providers: [syn::Type; 3] = [
        syn::parse_quote!(::cq_storage::AdaptiveMembership<_, 16777216>),
        syn::parse_quote!(::user::SelectedProvider<_>),
        syn::parse_quote!(::user::ProviderAlias),
    ];
    for provider in providers {
        let mut program = membership_plan();
        for entry in &mut program.structures.entries {
            if entry.name == "selected"
                && let cq_lang_structures::Structure::Built { keyed, .. } = &mut entry.structure
            {
                keyed.ty = provider.clone();
            }
        }
        let facts = cq_lang_structures::contract::check(&program).unwrap();
        assert!(facts.join_filters.is_empty());
        let mut context = cq_print::Context::new(&program, &facts);
        context.register("selected");
        context.materialize("selected", quote!(source_rows));
        let statements = &context.statements;
        let printed = quote!(#(#statements)*).to_string();
        assert!(printed.contains(&provider.to_token_stream().to_string()));
        if provider == syn::parse_quote!(::user::ProviderAlias) {
            assert!(printed.contains("ProviderAlias :: build"));
        } else {
            assert!(printed.contains("cq_storage :: build !"));
        }
        assert!(!printed.contains("join_filter"));
        assert!(!printed.contains("distinct_keys"));
    }
}

#[test]
fn raw_membership_fallback_dispatches_build_with_inferred_provider_output() {
    use quote::quote;
    let physical = syn::parse_quote! { physical {
        relation L(a: ::opaque::KeyAlias); relation R(a: ::opaque::KeyAlias);
        l = pipe rename L@0 {a -> x};
        r = pipe rename R@1 {a -> x};
        joined = pipe probe first [x] l,r;
        return canon joined as answer(x);
    }};
    let original = cq_structure_selection::compile(&physical).unwrap();
    let providers: [syn::Type; 2] = [
        syn::parse_quote!(::cq_storage::AdaptiveMembership<_, 16777216>),
        syn::parse_quote!(::user::ExactSetAlias),
    ];
    for provider in providers {
        let mut program = original.clone();
        for entry in &mut program.structures.entries {
            if entry.name == "r"
                && let cq_lang_structures::Structure::Host { keyed, .. } = &mut entry.structure
                && keyed.membership.is_some()
            {
                keyed.ty = provider.clone();
            }
        }
        let facts = cq_lang_structures::contract::check(&program).unwrap();
        let mut context = cq_print::Context::new(&program, &facts);
        context.register("r");
        context.prepare_host("r");
        let statements = &context.statements;
        let printed = quote!(#(#statements)*).to_string();
        assert!(printed.contains("cq_storage :: build !"), "{printed}");
        assert!(printed.contains("Lazy < _ , _ >"), "{printed}");
    }
}

#[test]
fn join_filters_emit_adaptive_builds_and_provider_independent_identity_predicates() {
    use quote::quote;
    let source = syn::parse_quote! { physical {
        relation L(a:i64); relation R(a:i64,b:i64);
        l = pipe rename L@0 {a -> x};
        r = pipe rename R@1 {a -> x,b -> y};
        selected = let range select r if (y > 1);
        joined = pipe probe first [x] l,selected;
        result = pipe canon project joined keep {x,y};
        return result as answer(x,y);
    }};
    let program = cq_structure_selection::compile_with(&source, quote!(positional(L, R))).unwrap();
    let facts = cq_lang_structures::contract::check(&program).unwrap();
    assert_eq!(facts.join_filters["l"].len(), 1);
    let mut context = cq_print::Context::new(&program, &facts);
    let statements = &context.statements;
    let identity = quote!(#(#statements)*).to_string();
    assert!(identity.contains("cq_storage :: NoJoinFilter"));
    assert!(!identity.contains("AdaptiveMembership"));
    assert!(!identity.contains("Option :: None"));
    assert!(!identity.contains("conservative"));
    // Task grading can select a provider with independent key and payload
    // holes. An unbuilt typed None would leave its payload unconstrained.
    let mut task_provider = program.clone();
    for entry in &mut task_provider.structures.entries {
        if let cq_lang_structures::Structure::JoinFilter { filter, .. } = &mut entry.structure {
            filter.ty = syn::parse_quote!(::cq_storage::sharded::Index<_, _>);
        }
    }
    let task_facts = cq_lang_structures::contract::check(&task_provider).unwrap();
    let task_context = cq_print::Context::new(&task_provider, &task_facts);
    let task_statements = &task_context.statements;
    assert_eq!(identity, quote!(#(#task_statements)*).to_string());
    context.register("selected");
    context.materialize("selected", quote!(source_rows));
    let statements = &context.statements;
    let printed = quote!(#(#statements)*).to_string();
    assert!(
        printed.contains(
            "cq_storage :: build ! (:: cq_storage :: AdaptiveMembership < _ , 67108864 >"
        )
    );
    assert!(printed.contains("distinct_keys"));
    assert!(printed.contains("JoinFilterKey :: < i64 > :: new"));
    assert!(printed.contains("eligible () && :: cq_storage :: selective_build"));
    assert!(!printed.contains("NoDense"));
}

#[test]
fn routed_builds_dispatch_inner_capabilities_without_changing_routing() {
    use quote::{ToTokens, quote};
    let providers: [syn::Type; 4] = [
        syn::parse_quote!(::cq_storage::AdaptiveMembership<_, 16777216>),
        syn::parse_quote!(::cq_storage::SortedRun<_, _, ::cq_storage::NoDense>),
        syn::parse_quote!(::cq_storage::SortedRun<_, _>),
        syn::parse_quote!(::cq_storage::SortedRun<_, _, ::cq_storage::AutoDense>),
    ];
    for inner in providers {
        for borrowed_singleton in [false, true] {
            let mut program = membership_plan();
            for entry in &mut program.structures.entries {
                if entry.name == "selected"
                    && let cq_lang_structures::Structure::Built { keyed, .. } = &mut entry.structure
                {
                    keyed.ty = syn::parse_quote!(::cq_storage::sharded::Sharded<#inner>);
                }
            }
            let facts = cq_lang_structures::contract::check(&program).unwrap();
            let mut context = cq_print::Context::new(&program, &facts);
            context.register("selected");
            let routed = [(0, inner.clone())].into();
            if borrowed_singleton {
                context.materialize_provider_parts(
                    "selected",
                    quote!(parts),
                    &["x".into()],
                    &routed,
                );
            } else {
                context
                    .materialize_partitioned("selected", quote!(parts), &["x".into()], &routed)
                    .unwrap();
            }
            let statements = &context.statements;
            let printed = quote!(#(#statements)*).to_string();
            assert!(printed.contains("Sharded :: < _ , _ > :: build_from_parts_with_router"));
            assert!(printed.contains("route_index"));
            assert!(printed.contains(&inner.to_token_stream().to_string()));
            assert!(printed.contains("cq_storage :: build !"));
            assert!(!printed.contains("as :: cq_storage :: Build < _ >"));
            assert_eq!(
                printed.matches("let __cq_build_source").count(),
                if borrowed_singleton { 2 } else { 1 },
            );
            if borrowed_singleton {
                assert!(printed.contains("from_parts_with_router"));
                assert!(printed.contains("source ()"));
                assert!(
                    printed.contains(
                        &quote!(
                            |__cq_key: &(&::opaque::KeyAlias,), __cq_count|
                        )
                        .to_string()
                    ),
                    "{printed}",
                );
            } else {
                assert!(
                    printed.contains(&quote!(|__cq_key: &(_,), __cq_count|).to_string()),
                    "non-hoisted routers retain contextual inference: {printed}",
                );
            }
        }
    }
}
