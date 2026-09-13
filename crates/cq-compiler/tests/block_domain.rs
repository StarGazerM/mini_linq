//! Source-domain failures and checking of explicitly composed physical routes.
use cq_compiler::{
    BlockOptions, PhysicalOptions, compile_blocked, compile_cq_planned, compile_physical,
    compile_zonemap,
};
use cq_ir::cq;
use quote::ToTokens;

fn options() -> BlockOptions {
    BlockOptions {
        storage: syn::parse_quote!(storage_alias),
        access: syn::parse_quote!(user::access),
        keys: syn::parse_quote!(::std::collections::HashSet),
    }
}

#[test]
fn unsupported_sources_are_diagnostics_not_alternative_routes() {
    let sources: &[(&str, &str)] = &[
        (
            "struct P; relation R(a:i64); relation answer(v:String); answer(x) :- R(x);",
            "i64 columns",
        ),
        (
            "struct P; relation R(a:i32); answer(x) :- R(x);",
            "i64 columns",
        ),
        (
            "struct P; relation R(a:i64); answer(x) :- R(x), if x > 0;",
            "supports Unit",
        ),
        (
            "struct P; relation R(a:i64); relation answer(v:i64); answer(x+1) :- R(x);",
            "variable-only output",
        ),
        (
            "struct P; relation R(a:i64); answer(y) :- R(x), let y:i64 = *x+1;",
            "supports Unit",
        ),
    ];
    for (text, expected) in sources {
        let source: cq::Module = syn::parse_str(text).unwrap();
        let ra = compile_cq_planned(&source).unwrap();
        let error = compile_blocked(&ra, &options()).unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
    }
}

#[test]
fn declared_continuations_and_distinct_access_shapes_remain_expansion_boundaries() {
    use syn::visit::{self, Visit};
    let source: cq::Module = syn::parse_quote! {
        struct P; relation R(a:i64,b:i64); relation S(a:i64,b:i64);
        answer(x,z) :- R(x,y), S(y,z);
    };
    let source = compile_cq_planned(&source).unwrap();
    for zone in [false, true] {
        let target = if zone {
            compile_zonemap(&source, &options())
        } else {
            compile_blocked(&source, &options())
        }
        .unwrap();
        let rust = prettyplease::unparse(&target);
        assert!(rust.contains("mini_linq::block_rows!"));
        assert!(rust.contains("access user::access;"));
        assert!(!rust.contains("mini_linq::pull"));
        for removed in [
            "::Relation",
            "::View",
            "Index::build",
            "Execution<",
            "semijoin(",
        ] {
            assert!(
                !rust.contains(removed),
                "unexpected runtime algorithm: {removed}"
            );
        }
        struct Calls {
            options: cq_compiler::RecipeOptions,
            zone: bool,
            semijoins: Vec<cq_ir::rust_access_plan::Plan>,
            builds: usize,
        }
        impl<'ast> Visit<'ast> for Calls {
            fn visit_impl_item_fn(&mut self, method: &'ast syn::ImplItemFn) {
                // The unobserved method has the same relational calls with all
                // instrumentation erased; inspect one generated execution body.
                if method.sig.ident == "run_with_stats" {
                    visit::visit_impl_item_fn(self, method);
                }
            }
            fn visit_macro(&mut self, mac: &'ast syn::Macro) {
                if let Ok(source) =
                    syn::parse2::<cq_ir::physical_context::Context>(mac.tokens.clone())
                {
                    if matches!(
                        source.source.operator,
                        cq_ir::relational_plan::Operator::Project(_)
                            | cq_ir::relational_plan::Operator::Semijoin(_)
                    ) {
                        let compile = if self.zone {
                            cq_compiler::compile_zone_semijoin
                        } else {
                            cq_compiler::compile_exact_semijoin
                        };
                        let expression = compile(&source, &self.options).unwrap();
                        // Test the typed consumer directly. Its configured
                        // wrapper is administrative Rust, not another query IR.
                        let mut nested = CollectAccess::default();
                        nested.visit_expr(&expression);
                        if nested.0.is_empty() {
                            self.builds += 1;
                        }
                        self.semijoins.extend(nested.0);
                        return;
                    }
                }
                visit::visit_macro(self, mac);
            }
        }
        #[derive(Default)]
        struct CollectAccess(Vec<cq_ir::rust_access_plan::Plan>);
        impl<'ast> Visit<'ast> for CollectAccess {
            fn visit_macro(&mut self, mac: &'ast syn::Macro) {
                fn collect(
                    tokens: proc_macro2::TokenStream,
                    plans: &mut Vec<cq_ir::rust_access_plan::Plan>,
                ) {
                    if let Ok(plan) = syn::parse2::<cq_ir::rust_access_plan::Plan>(tokens.clone()) {
                        plans.push(plan);
                        return;
                    }
                    for token in tokens {
                        if let proc_macro2::TokenTree::Group(group) = token {
                            collect(group.stream(), plans);
                        }
                    }
                }
                collect(mac.tokens.clone(), &mut self.0);
            }
        }
        let mut calls = Calls {
            options: cq_compiler::RecipeOptions {
                storage: options().storage,
                access: options().access,
                keys: Some(options().keys),
                work: Some(syn::parse_quote!(&__ml_work)),
            },
            zone,
            semijoins: Vec::new(),
            builds: 0,
        };
        calls.visit_file(&target);
        assert_eq!(calls.builds, 2);
        assert_eq!(calls.semijoins.len(), 2);
        for plan in calls.semijoins {
            cq_ir::rust_access_plan::contract::check(&plan).unwrap();
            let clauses: Vec<_> = plan
                .body
                .iter()
                .map(ToTokens::to_token_stream)
                .map(|t| t.to_string())
                .collect();
            assert_eq!(clauses.len(), if zone { 4 } else { 2 });
            assert!(clauses.last().unwrap().contains("contains"));
            if zone {
                assert!(clauses[0].contains("blocks"));
                assert!(clauses[1].contains("may_match"));
                assert!(clauses[2].contains("block_rows"));
            } else {
                assert!(clauses[0].contains("rows"));
            }
        }
    }
}

#[test]
fn each_input_and_materialized_result_keeps_its_own_provider_path() {
    let options = PhysicalOptions {
        storage: syn::parse_quote!(cq_storage),
        access: syn::parse_quote!(mini_linq::collect_access),
        inputs: std::collections::BTreeMap::from([
            ("R".into(), syn::parse_quote!(providers::blocks)),
            ("S".into(), syn::parse_quote!(providers::hash)),
            ("T".into(), syn::parse_quote!(providers::sorted)),
        ]),
        materialize: syn::parse_quote!(providers::hash),
        results: std::collections::BTreeMap::from([(
            "result".into(),
            syn::parse_quote!(providers::sorted),
        )]),
        semijoin: syn::parse_quote!(recipes::exact),
        semijoins: std::collections::BTreeMap::from([(
            "r1".into(),
            syn::parse_quote!(recipes::zone),
        )]),
        keys: syn::parse_quote!(::std::collections::HashSet),
        key_sets: std::collections::BTreeMap::from([(
            "r2".into(),
            syn::parse_quote!(::std::collections::BTreeSet),
        )]),
        join: syn::parse_quote!(recipes::hash_join),
        joins: Default::default(),
    };
    let source = syn::parse_quote! {
        struct P;
        relation R(k:i64); relation S(k:i64); relation T(k:i64);
        answer(k) :- R(k), S(k), T(k);
        relational {
            r = rename R {k -> k};
            s = rename S {k -> k};
            t = rename T {k -> k};
            r1 = semijoin r with s;
            r2 = semijoin r1 with t;
            result = project r2 keep {k};
            output result as answer(k);
        }
    };
    let target = compile_physical(&source, &options).unwrap();
    let rust = prettyplease::unparse(&target);
    for provider in ["blocks", "hash", "sorted"] {
        assert!(rust.contains(&format!("&providers::{provider}!")), "{rust}");
    }
    let compact: String = rust.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(
        compact.contains("providers::sorted!{storagecq_storage;"),
        "{rust}"
    );
    assert!(rust.contains("recipes::zone!"));
    assert!(rust.contains("recipes::exact!"));
    assert!(compact.contains("keys::std::collections::BTreeSet;"));
    assert!(!rust.contains("Relation"));
    assert!(!rust.contains("View"));
}

#[test]
fn actual_macro_composition_rejects_missing_access_and_incompatible_rust_types() {
    use std::{fs, process::Command};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let dir =
        std::env::temp_dir().join(format!("mini-linq-provider-domain-{}", std::process::id()));
    fs::create_dir_all(dir.join("src")).unwrap();
    let manifest = format!(
        "[package]\nname = \"provider-domain-check\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\
         [workspace]\n[dependencies]\nmini-linq = {{ path = {:?} }}\ncq-storage = {{ path = {:?} }}\n",
        root.join("crates/mini-linq"),
        root.join("crates/cq-storage"),
    );
    fs::write(dir.join("Cargo.toml"), manifest).unwrap();
    // Independent target directory prevents nested Cargo from waiting on the
    // workspace test build that is executing this harness.
    let run_case = |provider: &str, recipe: &str, keys: &str, expected: Option<&str>| {
        let source = format!(
            r#"
            macro_rules! wrong_hash_rows {{
                (storage $storage:path; type $arity:literal) => {{
                    ::std::collections::HashSet<::core::primitive::u64>
                }};
                ($($request:tt)*) => {{ ::mini_linq::hash_rows! {{ $($request)* }} }};
            }}
            // Inherent method names alone do not establish exact membership.
            struct MethodOnlyKeys<T>(Vec<T>);
            impl<T> FromIterator<T> for MethodOnlyKeys<T> {{
                fn from_iter<I: IntoIterator<Item = T>>(rows: I) -> Self {{
                    Self(rows.into_iter().collect())
                }}
            }}
            impl<T: PartialEq> MethodOnlyKeys<T> {{
                fn contains(&self, key: &T) -> bool {{ self.0.contains(key) }}
            }}
            mini_linq::physical_plan! {{
                storage cq_storage;
                access mini_linq::collect_access;
                inputs {{ R => {provider}; S => mini_linq::hash_rows; }}
                materialize mini_linq::hash_rows;
                semijoin {recipe};
                keys {keys};
                join mini_linq::hash_join;
                struct P;
                relation R(k:i64); relation S(k:i64);
                answer(k) :- R(k), S(k);
                relational {{
                    r = rename R {{k -> k}};
                    s = rename S {{k -> k}};
                    reduced = semijoin r with s;
                    output reduced as answer(k);
                }}
            }}
            fn main() {{}}
        "#
        );
        fs::write(dir.join("src/main.rs"), source).unwrap();
        let output = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
            .args(["check", "--offline", "--quiet", "--manifest-path"])
            .arg(dir.join("Cargo.toml"))
            .env("CARGO_TARGET_DIR", dir.join("target"))
            .output()
            .unwrap();
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        if let Some(expected) = expected {
            assert!(
                !output.status.success(),
                "incompatible composition compiled"
            );
            assert!(
                diagnostic.contains(expected),
                "{diagnostic}\nharness: {}",
                dir.display()
            );
        } else {
            assert!(
                output.status.success(),
                "{diagnostic}\nharness: {}",
                dir.display()
            );
        }
    };
    // The same dependency graph and ordinary hash route compile successfully.
    run_case(
        "mini_linq::hash_rows",
        "mini_linq::exact_semijoin",
        "::std::collections::HashSet",
        None,
    );
    // Explicitly selecting zone pruning on native row storage fails at its
    // requested operation. There is no capability-tag precheck or fallback.
    run_case(
        "mini_linq::hash_rows",
        "mini_linq::zone_semijoin",
        "::std::collections::HashSet",
        Some("no block descriptor lowering"),
    );
    // This provider expands every operation, but its chosen element type makes
    // the generated array access ill typed. rustc owns that diagnostic.
    run_case(
        "wrong_hash_rows",
        "mini_linq::exact_semijoin",
        "::std::collections::HashSet",
        Some("cannot index into a value of type"),
    );
    // A familiar method spelling does not silently stand in for the operation
    // trait and its stated exactness law.
    run_case(
        "mini_linq::hash_rows",
        "mini_linq::exact_semijoin",
        "MethodOnlyKeys",
        Some("ExactMembership"),
    );
    fs::remove_dir_all(&dir).unwrap();
}
