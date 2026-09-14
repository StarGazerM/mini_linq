use super::*;
use crate::compile_physical;
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

fn options() -> PhysicalOptions {
    PhysicalOptions {
        storage: syn::parse_quote!(cq_storage),
        access: syn::parse_quote!(collect_access),
        iterator: syn::parse_quote!(stream_access),
        inputs: BTreeMap::from([
            ("R".into(), syn::parse_quote!(block_rows)),
            ("S".into(), syn::parse_quote!(sorted_rows)),
            ("T".into(), syn::parse_quote!(vec_rows)),
        ]),
        materialize: syn::parse_quote!(hash_rows),
        results: BTreeMap::new(),
        scan: syn::parse_quote!(scan_rows),
        scans: BTreeMap::new(),
        key_provider: syn::parse_quote!(exact_keys),
        key_providers: BTreeMap::new(),
        keys: syn::parse_quote!(::std::collections::HashSet),
        key_sets: BTreeMap::new(),
        join: syn::parse_quote!(hash_witnesses),
        joins: BTreeMap::new(),
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
            syn::ImplItem::Fn(method) if method.sig.ident == "run_with_stats" => {
                Some(&mut method.block.stmts)
            }
            _ => None,
        })
        .unwrap()
}

fn initializer<'a>(target: &'a mut syn::File, name: &str) -> &'a mut syn::Expr {
    body(target).iter_mut().find_map(|statement| match statement {
        syn::Stmt::Local(local) if matches!(&local.pat, syn::Pat::Ident(pat) if pat.ident == name) => {
            Some(local.init.as_mut().unwrap().expr.as_mut())
        }
        _ => None,
    }).unwrap()
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
    options.results.insert(
        "joined".into(),
        syn::parse_quote!(application::ordered_output),
    );
    options
        .joins
        .insert("joined".into(), syn::parse_quote!(application::merge_join));
    let target = compile_physical(&source, &options).unwrap();
    check_physical(&source, &target, &options).unwrap();
    let text = target.to_token_stream().to_string();
    assert!(text.contains("application :: chunked_rows"));
    assert!(text.contains("application :: ordered_output"));
    assert!(text.contains("application :: merge_join"));
}

#[test]
fn resources_share_only_current_version_and_identical_selected_implementations() {
    let source = source();
    let count = |options: &PhysicalOptions| {
        let mut target = compile_physical(&source, options).unwrap();
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
        .scans
        .insert("right".into(), syn::parse_quote!(prune_rows));
    // Candidate traversal consumes the resource; it does not determine its
    // identity. A different scan strategy therefore preserves sharing.
    assert_eq!(count(&selected), 3);
    selected
        .key_providers
        .insert("right".into(), syn::parse_quote!(zone_keys));
    assert_eq!(count(&selected), 4);
    selected.key_providers.clear();
    selected.key_sets.insert(
        "right".into(),
        syn::parse_quote!(::std::collections::BTreeSet),
    );
    assert_eq!(count(&selected), 4);
}

#[test]
fn checker_rejects_stale_versions_wrong_resources_providers_and_key_columns() {
    let source = source();
    let options = options();
    let original = compile_physical(&source, &options).unwrap();
    for (binding, old, new) in [
        ("__ml_resource2", "__ml_view5", "__ml_view2"),
        ("__ml_view6", "__ml_resource2", "__ml_resource0"),
        ("__ml_resource0", "__ml_provider3", "__ml_provider1"),
        ("__ml_view3", "__ml_provider0", "__ml_provider3"),
        ("__ml_view3", "__cq_row [2usize]", "__cq_row [1usize]"),
        ("__ml_view3", "& [2usize , 1usize]", "& [1usize , 2usize]"),
        ("__ml_resource0", "__row [1usize]", "__row [0usize]"),
    ] {
        let mut target = original.clone();
        replace(initializer(&mut target, binding), old, new);
        assert!(
            check_physical(&source, &target, &options).is_err(),
            "accepted mutation {binding}: {old} -> {new}"
        );
    }
}

#[test]
fn checker_rejects_shadowed_and_reordered_generated_bindings() {
    let source = source();
    let options = options();
    let original = compile_physical(&source, &options).unwrap();
    for shadow in [
        syn::parse_quote!(let __ml_view3 = __ml_view0;),
        syn::parse_quote!(let (__ml_view3, _) = (__ml_view0, ());),
        syn::parse_quote!(let __ml_input0 = __ml_input1;),
    ] {
        let mut target = original.clone();
        body(&mut target).insert(0, shadow);
        assert!(check_physical(&source, &target, &options).is_err());
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
    assert!(check_physical(&source, &target, &options).is_err());
}

#[test]
fn checker_rejects_wrong_materialization_provider_and_join_operand() {
    let source = source();
    let options = options();
    let original = compile_physical(&source, &options).unwrap();
    for (old, new) in [("hash_rows", "vec_rows"), ("__ml_view6", "__ml_view0")] {
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
        if old == "hash_rows" {
            statement.mac.path = syn::parse_str(new).unwrap();
        } else {
            let text = statement.mac.tokens.to_string();
            assert!(text.contains(old));
            statement.mac.tokens = text.replace(old, new).parse().unwrap();
        }
        assert!(check_physical(&source, &target, &options).is_err());
    }
}

#[test]
fn checker_rejects_changed_projection_source_and_fields() {
    let source = source();
    let options = options();
    let original = compile_physical(&source, &options).unwrap();
    for (old, new) in [
        ("__ml_view7", "__ml_view0"),
        ("__row [2usize]", "__row [0usize]"),
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
        let text = statement.mac.tokens.to_string();
        assert!(text.contains(old));
        statement.mac.tokens = text.replace(old, new).parse().unwrap();
        assert!(check_physical(&source, &target, &options).is_err());
    }
}

#[test]
fn choice_maps_reject_unbound_inputs_and_overrides_of_inherited_storage() {
    let source = source();
    let valid = options();
    let target = compile_physical(&source, &valid).unwrap();
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
                    .results
                    .insert("left".into(), syn::parse_quote!(vec_rows));
            }
            3 => {
                invalid
                    .scans
                    .insert("joined".into(), syn::parse_quote!(scan_rows));
            }
            4 => {
                invalid.key_sets.insert(
                    "result".into(),
                    syn::parse_quote!(::std::collections::HashSet),
                );
            }
            5 => {
                invalid
                    .joins
                    .insert("left".into(), syn::parse_quote!(hash_witnesses));
            }
            6 => {
                invalid
                    .key_providers
                    .insert("joined".into(), syn::parse_quote!(zone_keys));
            }
            _ => {
                invalid
                    .scans
                    .insert("Missing".into(), syn::parse_quote!(prune_rows));
            }
        }
        assert!(compile_physical(&source, &invalid).is_err());
        assert!(check_physical(&source, &target, &invalid).is_err());
    }
}

#[test]
fn unit_and_zero_column_projection_survive_generic_comma_formatting() {
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
    let original = compile_physical(&source, &options).unwrap();
    for rows in [
        quote::quote!(::std::collections::HashSet::<[::std::primitive::i64; 0]>::from([[]])),
        quote::quote!(::std::collections::HashSet::<[::std::primitive::i64; 0]>::from([[]])),
    ] {
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
            materialize __ml_owner1 __ml_view1 (#rows) (__ml_block_rows)
        };
        check_physical(&source, &target, &options).unwrap();
    }
}

#[test]
fn checker_checks_lexically_bound_configuration_in_both_execution_methods() {
    let source = source();
    let options = options();
    let original = compile_physical(&source, &options).unwrap();
    for method_name in ["run", "run_with_stats"] {
        for (old, new) in [
            ("HashSet", "BTreeSet"),
            ("exact_keys", "zone_keys"),
            ("cq_storage", "wrong_storage"),
            ("stream_access", "wrong_iterator"),
            ("scan_rows", "prune_rows"),
        ] {
            let mut target = original.clone();
            let method = target
                .items
                .iter_mut()
                .filter_map(|i| {
                    if let syn::Item::Impl(i) = i {
                        Some(i)
                    } else {
                        None
                    }
                })
                .flat_map(|i| &mut i.items)
                .find_map(|i| {
                    if let syn::ImplItem::Fn(f) = i {
                        (f.sig.ident == method_name).then_some(f)
                    } else {
                        None
                    }
                })
                .unwrap();
            let item = method
                .block
                .stmts
                .iter_mut()
                .find_map(|stmt| match stmt {
                    syn::Stmt::Item(syn::Item::Macro(m))
                        if m.mac.tokens.to_string().contains(old) =>
                    {
                        Some(m)
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
                check_physical(&source, &target, &options).is_err(),
                "{method_name}: {old}"
            );
        }
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
    let target = compile_physical(&source, &options()).unwrap();
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
    let target = compile_physical(&source(), &options()).unwrap();
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
    let options = options();
    let mut target = compile_physical(&source, &options).unwrap();
    let statements = body(&mut target);
    let position = statements
        .iter()
        .position(|stmt| matches!(stmt, syn::Stmt::Item(syn::Item::Macro(_))))
        .unwrap();
    statements.insert(
        position,
        syn::parse_quote!(let (__ml_work,) = (&cq_storage::Counters::default(),);),
    );
    assert!(check_physical(&source, &target, &options).is_err());
}

#[test]
fn checker_rejects_a_configured_alias_shadowing_its_selected_macro() {
    let source = source();
    for collision in 0..3 {
        let mut options = options();
        if collision == 1 {
            options.access = syn::parse_quote!(__ml_provider0);
        } else if collision == 2 {
            options.iterator = syn::parse_quote!(__ml_provider0);
        } else {
            options
                .inputs
                .insert("R".into(), syn::parse_quote!(__ml_provider0));
        }
        let target = compile_physical(&source, &options).unwrap();
        let text = target.to_token_stream().to_string();
        assert!(text.contains("macro_rules ! __ml_provider1"));
        let target = syn::parse_str(&text.replace("__ml_provider1", "__ml_provider0")).unwrap();
        assert!(check_physical(&source, &target, &options).is_err());
    }
}
