use super::*;
use quote::ToTokens;

fn source() -> ra::Module {
    syn::parse_quote! {
        struct Mixed;
        relation R(a:i64,b:i64,c:i64);
        relation S(a:i64,b:i64,c:i64);
        relation T(k:i64,j:i64);
        answer(x,y) :- R(x,k,j), S(j,y,k), T(k,j);
        relational {
            r = rename R {a -> x,b -> k,c -> j};
            s = rename S {a -> j,b -> y,c -> k};
            t = rename T {k -> k,j -> j};
            left = semijoin r with t;
            right = semijoin s with t;
            pruned_t = semijoin t with left;
            again = semijoin left with pruned_t;
            joined = natural_join again with right;
            result = project joined keep {x,y};
            output result as answer(x,y);
        }
    }
}

fn options() -> RelationOptions {
    RelationOptions {
        access: Some(syn::parse_quote!(collect_access)),
        inputs: BTreeMap::from([
            ("R".into(), syn::parse_quote!(array_block_rows)),
            ("S".into(), syn::parse_quote!(array_sorted_rows)),
            ("T".into(), syn::parse_quote!(vec_rows)),
        ]),
        result_rows: Some(syn::parse_quote!(array_hash_rows)),
        result_bindings: BTreeMap::new(),
        candidates: Some(syn::parse_quote!(scan_candidates)),
        candidate_bindings: BTreeMap::new(),
        keys: Some(syn::parse_quote!(exact_keys)),
        key_sets: BTreeMap::new(),
        pairs: Some(syn::parse_quote!(hash_witnesses)),
        pair_bindings: BTreeMap::new(),
    }
}

fn body(target: &mut syn::File) -> &mut Vec<syn::Stmt> {
    target
        .items
        .iter_mut()
        .filter_map(|item| match item {
            syn::Item::Impl(item) => Some(item),
            _ => None,
        })
        .flat_map(|item| &mut item.items)
        .find_map(|item| match item {
            syn::ImplItem::Fn(method) if method.sig.ident == "run" => Some(&mut method.block.stmts),
            _ => None,
        })
        .unwrap()
}

fn initializer<'a>(target: &'a mut syn::File, name: &str) -> &'a mut syn::Expr {
    body(target)
        .iter_mut()
        .find_map(|statement| match statement {
            syn::Stmt::Local(local)
                if cq_ir::cq::rust::bindings(&local.pat)
                    .iter()
                    .any(|id| id == name) =>
            {
                Some(local.init.as_mut().unwrap().expr.as_mut())
            }
            _ => None,
        })
        .unwrap()
}

fn replace(expression: &mut syn::Expr, old: &str, new: &str) {
    let tokens = expression.to_token_stream().to_string();
    assert!(
        tokens.contains(old),
        "mutation must match actual generated syntax: {old} in {tokens}"
    );
    *expression = syn::parse_str(&tokens.replace(old, new)).unwrap();
}

#[test]
fn independent_wiring_check_accepts_heterogeneous_open_paths_and_scoped_results() {
    let source = source();
    let mut options = options();
    // Applicability belongs to expansion/type checking. An unfamiliar provider
    // path is not rejected by a compiler-owned catalogue of capability tags.
    options
        .inputs
        .insert("R".into(), syn::parse_quote!(application::chunked_rows));
    options.result_bindings.insert(
        "joined".into(),
        syn::parse_quote!(application::ordered_output),
    );
    options
        .pair_bindings
        .insert("joined".into(), syn::parse_quote!(application::merge_join));
    let target = compile_program(&source, &options).unwrap();
    check_program(&source, &target, &options).unwrap();
    let text = target.to_token_stream().to_string();
    assert!(text.contains("application :: chunked_rows"));
    assert!(text.contains("application :: ordered_output"));
    assert!(text.contains("application :: merge_join"));
}

#[test]
fn typed_select_extend_and_computed_head_preserve_rust_leaves_and_field_bindings() {
    let source: ra::Module = syn::parse_quote! {
        struct Typed;
        relation R(name: String, number: i32);
        relation answer(label: String, number: i32);
        answer(format!("result-{label}"), number) :-
            R(name, number), if number > 0,
            let label: String = format!("prefix-{name}");
        relational {
            r = rename R {name -> name, number -> number};
            selected = select r if (number > 0);
            extended = extend selected with {label: String} value ((format!("prefix-{name}"),));
            output extended as answer(format!("result-{label}"), number);
        }
    };
    let mut options = options();
    options.inputs = BTreeMap::from([("R".into(), syn::parse_quote!(tuple_hash_rows))]);
    options.result_rows = Some(syn::parse_quote!(tuple_sorted_rows));
    let original = compile_program(&source, &options).unwrap();
    check_program(&source, &original, &options).unwrap();

    let mut target = original.clone();
    replace(
        initializer(&mut target, "__ml_view1"),
        "field (__cq_row) (1)",
        "field (__cq_row) (0)",
    );
    assert!(check_program(&source, &target, &options).is_err());
    let mut target = original.clone();
    let statement = body(&mut target)
        .iter_mut()
        .find_map(|statement| match statement {
            syn::Stmt::Macro(mac) if mac.mac.tokens.to_string().contains("__ml_owner2") => {
                Some(mac)
            }
            _ => None,
        })
        .unwrap();
    let original_leaf = statement.mac.tokens.to_string();
    assert!(original_leaf.contains("prefix-{name}"));
    statement.mac.tokens = original_leaf
        .replace("prefix-{name}", "changed-{name}")
        .parse()
        .unwrap();
    assert!(check_program(&source, &target, &options).is_err());
    let mut target = original;
    replace(
        initializer(&mut target, "__ml_answer"),
        "result-{label}",
        "changed-{label}",
    );
    assert!(check_program(&source, &target, &options).is_err());
}

#[test]
fn resources_share_only_current_version_and_identical_selected_implementations() {
    let source = source();
    let count = |options: &RelationOptions| {
        let mut target = compile_program(&source, options).unwrap();
        body(&mut target)
            .iter()
            .filter(|stmt| {
                matches!(stmt,
            syn::Stmt::Local(local) if matches!(&local.pat,
                syn::Pat::Ident(pat) if pat.ident.to_string().starts_with("__ml_resource")))
            })
            .count()
    };
    let mut selected = options();
    // left/right share T and its sorted separator; pruned_t and again require
    // separate resources over distinct current logical versions.
    assert_eq!(count(&selected), 3);
    selected
        .candidate_bindings
        .insert("right".into(), syn::parse_quote!(prune_block_candidates));
    // Candidate traversal consumes the resource; it does not determine its
    // identity. A different scan strategy therefore preserves sharing.
    assert_eq!(count(&selected), 3);
    selected
        .key_sets
        .insert("right".into(), syn::parse_quote!(summarized_exact_keys));
    assert_eq!(count(&selected), 4);
    selected.key_sets.clear();
    selected.key_sets.insert(
        "right".into(),
        syn::parse_quote!(separately_bound_exact_keys),
    );
    assert_eq!(count(&selected), 4);
}

#[test]
fn scoped_bindings_do_not_require_unused_defaults() {
    let source = source();
    let mut options = options();
    options.result_rows = None;
    options.candidates = None;
    options.keys = None;
    options.pairs = None;
    for definition in &source.plan.definitions {
        let result = symbol_name(&definition.result);
        match &definition.operator {
            ra::Operator::Semijoin(_) => {
                options
                    .candidate_bindings
                    .insert(result.clone(), syn::parse_quote!(scan_candidates));
                options
                    .key_sets
                    .insert(result, syn::parse_quote!(exact_keys));
            }
            ra::Operator::NaturalJoin(_) => {
                options
                    .result_bindings
                    .insert(result.clone(), syn::parse_quote!(array_hash_rows));
                options
                    .pair_bindings
                    .insert(result, syn::parse_quote!(hash_pairs));
            }
            ra::Operator::Project(_) => {
                options
                    .result_bindings
                    .insert(result, syn::parse_quote!(array_hash_rows));
            }
            _ => {}
        }
    }
    let target = compile_program(&source, &options).unwrap();
    check_program(&source, &target, &options).unwrap();
    options.candidate_bindings.remove("left");
    let error = compile_program(&source, &options).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("relation `left` requires a `candidates` implementation")
    );
    assert!(check_program(&source, &target, &options).is_err());
}

#[test]
fn missing_configuration_names_the_operation_that_needs_it() {
    let source: ra::Module = syn::parse_quote! {
        struct Join;
        relation R(x:i64); relation S(x:i64);
        answer(x) :- R(x), S(x);
        relational {
            r = rename R {x -> x}; s = rename S {x -> x};
            joined = natural_join r with s; output joined as answer(x);
        }
    };
    let mut options = RelationOptions {
        inputs: BTreeMap::from([
            ("R".into(), syn::parse_quote!(array_hash_rows)),
            ("S".into(), syn::parse_quote!(array_hash_rows)),
        ]),
        ..RelationOptions::default()
    };
    let error = compile_program(&source, &options).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("relation `joined` requires a `result_rows` implementation")
    );
    options
        .result_bindings
        .insert("joined".into(), syn::parse_quote!(array_hash_rows));
    let error = compile_program(&source, &options).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("relation `joined` requires a `pairs` implementation")
    );
    options
        .pair_bindings
        .insert("joined".into(), syn::parse_quote!(hash_pairs));
    compile_program(&source, &options).unwrap();
}

#[test]
fn generated_type_qualification_preserves_aliases_and_opaque_paths() {
    let actual = crate::rust_bindings::qualify_type(&syn::parse_quote!((
        i64,
        Option<i32>,
        &'static [u8],
        Alias,
        user::i64,
        r#i64
    )));
    let expected: syn::Type = syn::parse_quote!((
        ::core::primitive::i64,
        Option<::core::primitive::i32>,
        &'static [::core::primitive::u8],
        Alias,
        user::i64,
        r#i64
    ));
    assert_eq!(actual, expected);
}

#[test]
fn checker_rejects_stale_versions_wrong_resources_providers_and_key_columns() {
    let source = source();
    let options = options();
    let original = compile_program(&source, &options).unwrap();
    for (binding, old, new) in [
        ("__ml_resource2", "__ml_view5", "__ml_view2"),
        ("__ml_view6", "__ml_resource2", "__ml_resource0"),
        ("__ml_resource0", "vec_rows", "array_sorted_rows"),
        ("__ml_view3", "array_block_rows", "vec_rows"),
        ("__ml_view3", "field (__cq_row) (2)", "field (__cq_row) (1)"),
        ("__ml_view3", "& [2usize , 1usize]", "& [1usize , 2usize]"),
        ("__ml_resource0", "field (__row) (1)", "field (__row) (0)"),
    ] {
        let mut target = original.clone();
        replace(initializer(&mut target, binding), old, new);
        assert!(
            check_program(&source, &target, &options).is_err(),
            "accepted mutation {binding}: {old} -> {new}"
        );
    }
}

#[test]
fn checker_rejects_shadowed_and_reordered_generated_bindings() {
    let source = source();
    let options = options();
    let original = compile_program(&source, &options).unwrap();
    for shadow in [
        syn::parse_quote!(let __ml_view3 = __ml_view0;),
        syn::parse_quote!(let (__ml_view3, _) = (__ml_view0, ());),
        syn::parse_quote!(let __ml_input0 = __ml_input1;),
    ] {
        let mut target = original.clone();
        body(&mut target).insert(0, shadow);
        assert!(check_program(&source, &target, &options).is_err());
    }
    let mut target = original;
    let statements = body(&mut target);
    let position = |statements: &[syn::Stmt], name: &str| {
        statements
            .iter()
            .position(|stmt| {
                matches!(stmt,
        syn::Stmt::Local(local) if matches!(&local.pat, syn::Pat::Ident(pat) if pat.ident == name))
            })
            .unwrap()
    };
    let resource = position(statements, "__ml_resource0");
    let consumer = position(statements, "__ml_view3");
    statements.swap(resource, consumer);
    assert!(check_program(&source, &target, &options).is_err());
}

#[test]
fn checker_rejects_wrong_materialization_provider_and_join_operand() {
    let source = source();
    let options = options();
    let original = compile_program(&source, &options).unwrap();
    for (old, new) in [
        ("array_hash_rows", "vec_rows"),
        ("__ml_view6", "__ml_view0"),
    ] {
        let mut target = original.clone();
        let statement = body(&mut target)
            .iter_mut()
            .find_map(|stmt| match stmt {
                syn::Stmt::Macro(mac) if mac.mac.tokens.to_string().contains("__ml_owner7") => {
                    Some(mac)
                }
                _ => None,
            })
            .unwrap();
        if old == "array_hash_rows" {
            statement.mac.path = syn::parse_str(new).unwrap();
        } else {
            let expression = initializer(&mut target, "__ml_view7_input0");
            replace(expression, old, new);
        }
        assert!(check_program(&source, &target, &options).is_err());
    }
}

#[test]
fn checker_rejects_changed_projection_source_and_fields() {
    let source = source();
    let options = options();
    let original = compile_program(&source, &options).unwrap();
    for (old, new) in [
        ("__ml_view7", "__ml_view0"),
        ("field (__row) (2)", "field (__row) (0)"),
    ] {
        let mut target = original.clone();
        let statement = body(&mut target)
            .iter_mut()
            .find_map(|stmt| match stmt {
                syn::Stmt::Macro(mac) if mac.mac.tokens.to_string().contains("__ml_owner8") => {
                    Some(mac)
                }
                _ => None,
            })
            .unwrap();
        if old == "__ml_view7" {
            replace(initializer(&mut target, "__ml_view8_input0"), old, new);
        } else {
            let text = statement.mac.tokens.to_string();
            assert!(text.contains(old));
            statement.mac.tokens = text.replace(old, new).parse().unwrap();
        }
        assert!(check_program(&source, &target, &options).is_err());
    }
}

#[test]
fn choice_maps_reject_unbound_inputs_and_overrides_of_inherited_storage() {
    let source = source();
    let valid = options();
    let target = compile_program(&source, &valid).unwrap();
    for choice in 0..8 {
        let mut invalid = options();
        match choice {
            0 => {
                invalid.inputs.remove("R");
            }
            1 => {
                invalid
                    .inputs
                    .insert("Missing".into(), syn::parse_quote!(vec_rows));
            }
            2 => {
                invalid
                    .result_bindings
                    .insert("left".into(), syn::parse_quote!(vec_rows));
            }
            3 => {
                invalid
                    .candidate_bindings
                    .insert("joined".into(), syn::parse_quote!(scan_candidates));
            }
            4 => {
                invalid.key_sets.insert(
                    "result".into(),
                    syn::parse_quote!(::std::collections::HashSet),
                );
            }
            5 => {
                invalid
                    .pair_bindings
                    .insert("left".into(), syn::parse_quote!(hash_witnesses));
            }
            6 => {
                invalid
                    .key_sets
                    .insert("joined".into(), syn::parse_quote!(summarized_exact_keys));
            }
            _ => {
                invalid
                    .candidate_bindings
                    .insert("Missing".into(), syn::parse_quote!(prune_block_candidates));
            }
        }
        assert!(compile_program(&source, &invalid).is_err());
        assert!(check_program(&source, &target, &invalid).is_err());
    }
}

#[test]
fn unit_and_zero_column_projection_use_the_selected_record_representation() {
    let source: ra::Module = syn::parse_quote! {
        struct HasRows;
        relation R(a:i64);
        answer(x) :- R(x);
        relational {
            r = rename R {a -> x};
            u = unit;
            joined = natural_join u with r;
            exists = project joined keep {};
            result = natural_join r with exists;
            output result as answer(x);
        }
    };
    let mut options = options();
    options.inputs.retain(|name, _| name == "R");
    let original = compile_program(&source, &options).unwrap();
    check_program(&source, &original, &options).unwrap();
    let mut target = original.clone();
    let statement = body(&mut target)
        .iter_mut()
        .find_map(|statement| match statement {
            syn::Stmt::Macro(mac) if mac.mac.tokens.to_string().contains("__ml_owner1") => {
                Some(mac)
            }
            _ => None,
        })
        .unwrap();
    statement.mac.tokens = quote::quote! {
        materialize __ml_owner1 __ml_view1 (::core::iter::once(array_hash_rows! { record (1) }))
    };
    assert!(check_program(&source, &target, &options).is_err());
}

#[test]
fn checker_checks_observer_forwarding_and_configured_binding_identity() {
    let source = source();
    let options = ObservedOptions {
        relations: options(),
        counter_storage: syn::parse_quote!(cq_storage),
    };
    let original = compile_observed_program(&source, &options).unwrap();
    for (old, new) in [
        ("exact_keys", "summarized_exact_keys"),
        ("scan_candidates", "prune_block_candidates"),
        ("__ml_work", "wrong_counter"),
    ] {
        let mut target = original.clone();
        let item = body(&mut target)
            .iter_mut()
            .find_map(|statement| match statement {
                syn::Stmt::Item(syn::Item::Macro(item))
                    if item.mac.tokens.to_string().contains(old) =>
                {
                    Some(item)
                }
                _ => None,
            })
            .unwrap();
        item.mac.tokens = item
            .mac
            .tokens
            .to_string()
            .replace(old, new)
            .parse()
            .unwrap();
        assert!(
            check_observed_program(&source, &target, &options).is_err(),
            "{old}"
        );
    }
}
#[test]
fn physical_output_contains_access_without_serialized_ra_or_fresh_logical_relations() {
    let mut source = source();
    let tokens = source
        .to_token_stream()
        .to_string()
        .replace("left", "r#__ml_keys0");
    source = syn::parse_str(&tokens).unwrap();
    let target = compile_program(&source, &options()).unwrap();
    let text = target.to_token_stream().to_string();
    for absent in [
        "= project",
        "= semijoin",
        "= natural_join",
        "keep {",
        "bindings {",
        "__ml_keys1",
    ] {
        assert!(!text.contains(absent), "unexpected RA syntax: {absent}");
    }
    assert!(text.contains("build { out"));
    assert!(text.contains("Candidates (__cq_handle) => for"));
}
#[test]
fn plain_execution_contains_no_stats_or_counter_operations() {
    let target = compile_program(&source(), &options()).unwrap();
    let method = target
        .items
        .iter()
        .filter_map(|i| {
            if let syn::Item::Impl(i) = i {
                Some(i)
            } else {
                None
            }
        })
        .flat_map(|i| &i.items)
        .find_map(|i| {
            if let syn::ImplItem::Fn(f) = i {
                (f.sig.ident == "run").then_some(f)
            } else {
                None
            }
        })
        .unwrap();
    let text = method.block.to_token_stream().to_string();
    for absent in ["__ml_work", "__ml_stats", "run_with_stats", "Counters"] {
        assert!(!text.contains(absent), "{absent}");
    }
}

#[test]
fn checker_rejects_instrumentation_shadowing_through_destructuring() {
    let source = source();
    let options = ObservedOptions {
        relations: options(),
        counter_storage: syn::parse_quote!(cq_storage),
    };
    let mut target = compile_observed_program(&source, &options).unwrap();
    let statements = body(&mut target);
    let position = statements
        .iter()
        .position(|stmt| matches!(stmt, syn::Stmt::Item(syn::Item::Macro(_))))
        .unwrap();
    statements.insert(
        position,
        syn::parse_quote!(let (__ml_work,) = (&cq_storage::Counters::default(),);),
    );
    assert!(check_observed_program(&source, &target, &options).is_err());
}

#[test]
fn checker_rejects_a_configured_alias_shadowing_its_selected_macro() {
    let source = source();
    for collision in 0..3 {
        let mut physical = options();
        if collision == 1 {
            physical.access = Some(syn::parse_quote!(__ml_observed0));
        } else if collision == 2 {
            physical.candidates = Some(syn::parse_quote!(__ml_observed0));
        } else {
            physical
                .inputs
                .insert("R".into(), syn::parse_quote!(__ml_observed0));
        }
        let options = ObservedOptions {
            relations: physical,
            counter_storage: syn::parse_quote!(cq_storage),
        };
        let target = compile_observed_program(&source, &options).unwrap();
        let text = target.to_token_stream().to_string();
        assert!(text.contains("macro_rules ! __ml_observed1"));
        let target = syn::parse_str(&text.replace("__ml_observed1", "__ml_observed0")).unwrap();
        assert!(check_observed_program(&source, &target, &options).is_err());
    }
}

#[test]
fn callable_hosts_have_borrowed_parameters_and_owned_answers() {
    let source = source();
    let options = options();
    let plain = compile_query(&source, &options).unwrap();
    check_query(&source, &plain, &options).unwrap();
    let text = plain.to_token_stream().to_string();
    assert!(text.starts_with("move |"));
    assert!(text.contains("-> :: std :: vec :: Vec"));
    assert!(!text.contains("Counters"));
    assert!(!text.contains("array_block_rows :"));
    let observed = ObservedOptions {
        relations: options,
        counter_storage: syn::parse_quote!(cq_storage),
    };
    let target = compile_observed_query(&source, &observed).unwrap();
    check_observed_query(&source, &target, &observed).unwrap();
    let mut wrong = plain;
    let syn::Expr::Closure(closure) = &mut wrong else {
        unreachable!()
    };
    closure.inputs.clear();
    assert!(check_query(&source, &wrong, &observed.relations).is_err());
}

#[test]
fn checker_rejects_changed_owned_output_and_observation_checkpoints() {
    let source = source();
    let options = ObservedOptions {
        relations: options(),
        counter_storage: syn::parse_quote!(cq_storage),
    };
    let original = compile_observed_program(&source, &options).unwrap();
    let mut changed_output = original.clone();
    let answer = body(&mut changed_output)
        .iter_mut()
        .find_map(|statement| match statement {
            syn::Stmt::Local(local)
                if local
                    .to_token_stream()
                    .to_string()
                    .contains("let __ml_answer") =>
            {
                Some(local)
            }
            _ => None,
        })
        .unwrap();
    replace(
        answer.init.as_mut().unwrap().expr.as_mut(),
        "field (__ml_row) (0)",
        "field (__ml_row) (1)",
    );
    assert!(check_observed_program(&source, &changed_output, &options).is_err());
    let mut changed_checkpoint = original;
    let checkpoint = body(&mut changed_checkpoint)
        .iter_mut()
        .find(|statement| {
            statement
                .to_token_stream()
                .to_string()
                .contains("steps . push")
        })
        .unwrap();
    *checkpoint = syn::parse_quote!(__ml_stats.steps.clear(););
    assert!(check_observed_program(&source, &changed_checkpoint, &options).is_err());
}

fn observed_callable_body(target: &mut syn::Expr) -> &mut Vec<syn::Stmt> {
    let syn::Expr::Closure(closure) = target else {
        panic!("expected observed closure")
    };
    let syn::Expr::Block(body) = closure.body.as_mut() else {
        panic!("expected observed body")
    };
    &mut body.block.stmts
}

#[test]
fn observed_hosts_reject_fabricated_initial_statistics_and_rebinding() {
    let source = source();
    let options = ObservedOptions {
        relations: options(),
        counter_storage: syn::parse_quote!(cq_storage),
    };
    let original_item = compile_observed_program(&source, &options).unwrap();
    let original_callable = compile_observed_query(&source, &options).unwrap();
    for mutation in 0..7 {
        let mutate = |statements: &mut Vec<syn::Stmt>| match mutation {
            0 => {
                let syn::Stmt::Local(local) = &mut statements[1] else {
                    unreachable!()
                };
                *local.init.as_mut().unwrap().expr = syn::parse_quote! {
                    __MlStatistics {
                        steps: vec![("invented_step",123,Default::default())],
                        output: Default::default(),
                    }
                };
            }
            1 => {
                let duplicate = statements[1].clone();
                statements.insert(2, duplicate);
            }
            2 => {
                statements.swap(0, 1);
            }
            3 => {
                statements.insert(2, syn::parse_quote!(let (__ml_stats,) = (__ml_stats,);));
            }
            _ => {
                statements.remove(1);
            }
        };
        let mut item = original_item.clone();
        mutate(body(&mut item));
        assert!(
            check_observed_program(&source, &item, &options).is_err(),
            "item mutation {mutation}"
        );
        let mut callable = original_callable.clone();
        mutate(observed_callable_body(&mut callable));
        assert!(
            check_observed_query(&source, &callable, &options).is_err(),
            "callable mutation {mutation}"
        );
    }
}

#[test]
fn observed_hosts_check_their_statistics_definition_and_constructor() {
    let source = source();
    let options = ObservedOptions {
        relations: options(),
        counter_storage: syn::parse_quote!(cq_storage),
    };
    let mut item = compile_observed_program(&source, &options).unwrap();
    let stats = item
        .items
        .iter_mut()
        .find_map(|item| match item {
            syn::Item::Struct(item) if item.ident == "MixedStats" => Some(item),
            _ => None,
        })
        .unwrap();
    stats.attrs.clear();
    assert!(check_observed_program(&source, &item, &options).is_err());
    let mut callable = compile_observed_query(&source, &options).unwrap();
    let syn::Stmt::Local(stats) = &mut observed_callable_body(&mut callable)[1] else {
        unreachable!()
    };
    let initializer = stats.init.as_mut().unwrap().expr.as_mut();
    replace(
        initializer,
        "steps : :: std :: vec :: Vec :: new ()",
        "steps : vec ! [(\"invented_step\",123,output)]",
    );
    assert!(check_observed_query(&source, &callable, &options).is_err());
}
