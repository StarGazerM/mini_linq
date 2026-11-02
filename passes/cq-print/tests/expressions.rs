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

#[test]
fn capture_ownership_clones_values_at_every_closure_level() {
    use quote::ToTokens;
    let mut expression: syn::Expr = syn::parse_quote!({
        inspect(c);
        let outer = move || {
            inspect(c);
            move || inspect(c)
        };
        let sibling = move || inspect(c);
        (outer, sibling)
    });
    cq_print::own_captures(
        &mut expression,
        &[],
        &[(syn::parse_quote!(value), syn::parse_quote!(c))],
    );
    let printed = expression.to_token_stream().to_string();
    assert_eq!(printed.matches("Clone :: clone (& value)").count(), 3);
    assert_eq!(printed.matches("let c = & value").count(), 3);
    assert!(!printed.contains("Arc"));
    assert!(!printed.contains("Rc"));
    assert!(printed.starts_with("{ inspect (c) ;"));
}

fn membership_plan() -> cq_lang_structures::Program {
    let source = syn::parse_quote! { physical {
        relation L(a: ::opaque::KeyAlias); relation R(a: ::opaque::KeyAlias,b:i64);
        l = pipe rename L@0 {a -> x};
        r = pipe rename R@1 {a -> x,b -> y};
        selected = let range select r if (y > 1);
        joined = pipe probe first [x] l,selected;
        result = let canon project joined keep {x};
        return result as answer(x);
    }};
    cq_structure_selection::compile_with(&source, quote::quote!(positional(L, R))).unwrap()
}

#[test]
fn explicit_providers_dispatch_build_capabilities_without_type_recognition() {
    use quote::{ToTokens, quote};
    // The declared `build` fact, not the spelling, selects the construction.
    let providers: [(cq_lang_structures::Declaration, syn::Type); 3] = [
        (
            cq_lang_structures::provider::base::declaration("adaptive_membership"),
            syn::parse_quote!(::cq_storage::AdaptiveMembership<_, 16777216>),
        ),
        (
            syn::parse_quote!(selected_provider = ::user::SelectedProvider<_> {
                reads [exact, range, membership], build typed
            };),
            syn::parse_quote!(::user::SelectedProvider<_>),
        ),
        (
            syn::parse_quote!(provider_alias = ::user::ProviderAlias {
                reads [exact, range, membership], build inferred
            };),
            syn::parse_quote!(::user::ProviderAlias),
        ),
    ];
    for (declaration, provider) in providers {
        let mut program = membership_plan();
        for entry in &mut program.structures.entries {
            if entry.name == "selected"
                && let cq_lang_structures::Structure::Built { keyed, .. } = &mut entry.structure
            {
                keyed.provider = cq_lang_structures::Provider {
                    token: declaration.token.clone(),
                    inner: None,
                    input: None,
                    radix: None,
                    ty: provider.clone(),
                };
            }
        }
        program.structures.declare(&[declaration]);
        let facts = cq_lang_structures::contract::check(&program).unwrap();
        assert!(facts.join_filters.is_empty());
        let mut context = cq_print::Context::new(&program, &facts);
        context.register("selected");
        context.materialize("selected", quote!(source_rows));
        let statements = &context.statements;
        let printed = quote!(#(#statements)*).to_string();
        assert!(printed.contains(&provider.to_token_stream().to_string()));
        // Every recorded provider is built by one direct call: a typed build
        // through `Build`, an inferred build through its own constructor.
        if provider == syn::parse_quote!(::user::ProviderAlias) {
            assert!(printed.contains("ProviderAlias :: build"), "{printed}");
        } else {
            assert!(
                printed.contains("as :: cq_storage :: Build < _ > > :: build"),
                "{printed}"
            );
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
    let providers: [cq_lang_structures::Provider; 2] = [
        syn::parse_quote!(@adaptive_membership as ::cq_storage::AdaptiveMembership<_, 16777216>),
        syn::parse_quote!(@exact_set_alias as ::user::ExactSetAlias),
    ];
    for provider in providers {
        let mut program = original.clone();
        for entry in &mut program.structures.entries {
            if entry.name == "r"
                && let cq_lang_structures::Structure::Host { keyed, .. } = &mut entry.structure
                && keyed.membership.is_some()
            {
                keyed.provider = provider.clone();
            }
        }
        program.structures.declare(&[
            cq_lang_structures::provider::base::declaration("adaptive_membership"),
            syn::parse_quote!(exact_set_alias = ::user::ExactSetAlias {
                reads [exact, membership], build inferred
            };),
        ]);
        let facts = cq_lang_structures::contract::check(&program).unwrap();
        let mut context = cq_print::Context::new(&program, &facts);
        context.register("r");
        context.prepare_host("r");
        let statements = &context.statements;
        let printed = quote!(#(#statements)*).to_string();
        // One direct typed build; the fallback cell spells the recorded provider.
        assert!(
            printed.contains("as :: cq_storage :: Build < _ > > :: build"),
            "{printed}"
        );
        let ty = &provider.ty;
        assert!(
            printed.contains(&format!("Lazy < {} , _ >", quote!(#ty))),
            "{printed}"
        );
    }
}

#[test]
fn join_filters_emit_dense_builds_and_provider_independent_identity_predicates() {
    use quote::quote;
    let source = syn::parse_quote! { physical {
        relation L(a:i64); relation R(a:i64,b:i64);
        l = pipe rename L@0 {a -> x};
        r = pipe rename R@1 {a -> x,b -> y};
        selected = let range select r if (y > 1);
        joined = pipe probe first [x] l,selected;
        result = let canon project joined keep {x,y};
        return result as answer(x,y);
    }};
    let program = cq_structure_selection::compile_with(&source, quote!(positional(L, R))).unwrap();
    let facts = cq_lang_structures::contract::check(&program).unwrap();
    assert_eq!(facts.join_filters["l"].len(), 1);
    let mut context = cq_print::Context::new(&program, &facts);
    let statements = &context.statements;
    let identity = quote!(#(#statements)*).to_string();
    assert!(identity.contains("cq_storage :: NoJoinFilter"));
    assert!(!identity.contains("DenseMembership"));
    assert!(!identity.contains("Option :: None"));
    assert!(!identity.contains("conservative"));
    // Task grading can select a provider with independent key and payload
    // holes. An unbuilt typed None would leave its payload unconstrained.
    let mut task_provider = program.clone();
    for entry in &mut task_provider.structures.entries {
        if let cq_lang_structures::Structure::JoinFilter { filter, .. } = &mut entry.structure {
            filter.provider = cq_lang_structures::provider::base::instance(
                "sharded_index",
                syn::parse_quote!(::cq_storage::sharded::Index<_, _>),
            );
        }
    }
    task_provider
        .structures
        .declare(&[cq_lang_structures::provider::base::declaration(
            "sharded_index",
        )]);
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
            "< :: cq_storage :: DenseMembership < _ , 67108864 > as :: cq_storage :: Build < _ > > :: build"
        ),
        "{printed}"
    );
    assert!(printed.contains("distinct_keys"));
    // Every recorded filter is built; the recorded abandon policy is printed.
    assert!(!printed.contains("JoinFilterKey"));
    assert!(
        printed.contains(":: cq_storage :: selective_build"),
        "{printed}"
    );
    assert!(!printed.contains("NoDense"));
}

#[test]
fn routed_builds_dispatch_inner_capabilities_without_changing_routing() {
    use cq_lang_structures::provider::base::{declaration, instance};
    use quote::{ToTokens, quote};
    let providers: [cq_lang_structures::Provider; 3] = [
        instance(
            "adaptive_membership",
            syn::parse_quote!(::cq_storage::AdaptiveMembership<_, 16777216>),
        ),
        instance(
            "sorted_run",
            syn::parse_quote!(::cq_storage::SortedRun<_, _, ::cq_storage::NoDense>),
        ),
        instance(
            "sorted_run",
            syn::parse_quote!(::cq_storage::SortedRun<_, _>),
        ),
    ];
    for inner in providers {
        for borrowed_singleton in [false, true] {
            let mut program = membership_plan();
            for entry in &mut program.structures.entries {
                if entry.name == "selected"
                    && let cq_lang_structures::Structure::Built { keyed, .. } = &mut entry.structure
                {
                    keyed.provider = declaration("sharded").compose(inner.clone()).unwrap();
                }
            }
            program.structures.declare(&[
                declaration("sharded"),
                declaration(&inner.token.to_string()),
            ]);
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
            assert!(printed.contains(&inner.ty.to_token_stream().to_string()));
            assert!(printed.contains("as :: cq_storage :: Build < _ > > :: build"));
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
