//! The direct RA edge and unsupported native access compositions.
use cq_compose::compile_cq_planned;
use cq_ir::{cq, relational_plan as ra};
use cq_physical::direct::PhysicalOptions;
use cq_physical::direct::compile_physical;

fn options(source: &ra::Module) -> PhysicalOptions {
    PhysicalOptions {
        access: syn::parse_quote!(user::access),
        inputs: source
            .program
            .inputs()
            .map(|r| {
                (
                    cq_ir::symbol_name(&r.name),
                    syn::parse_quote!(providers::rows),
                )
            })
            .collect(),
        materialize: syn::parse_quote!(providers::rows),
        results: Default::default(),
        scan: syn::parse_quote!(accesses::scan),
        scans: Default::default(),
        keys: syn::parse_quote!(accesses::keys),
        key_sets: Default::default(),
        join: syn::parse_quote!(accesses::witnesses),
        joins: Default::default(),
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
        let error = compile_physical(&ra, &options(&ra)).unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
    }
}

#[test]
fn direct_lowering_emits_existing_access_regions_and_native_operations() {
    use syn::visit::{self, Visit};
    let source: cq::Module = syn::parse_quote! {
        struct P; relation R(a:i64,b:i64); relation S(a:i64,b:i64);
        answer(x,z) :- R(x,y), S(y,z);
    };
    let source = compile_cq_planned(&source).unwrap();
    let options = options(&source);
    let target = compile_physical(&source, &options).unwrap();
    cq_physical::direct::check_physical(&source, &target, &options).unwrap();
    let rust = prettyplease::unparse(&target);
    for selected in [
        "user::access!",
        "accesses::scan!",
        "accesses::keys!",
        "accesses::witnesses!",
    ] {
        assert!(rust.contains(selected), "missing {selected}: {rust}");
    }
    for removed in [
        "indexes {",
        "= semijoin",
        "= natural_join",
        "bindings {",
        "::Relation",
        "::View",
        "Execution<",
    ] {
        assert!(!rust.contains(removed), "unexpected {removed}");
    }
    #[derive(Default)]
    struct Regions(Vec<cq_ir::rust_access_plan::Plan>);
    impl<'ast> Visit<'ast> for Regions {
        fn visit_impl_item_fn(&mut self, method: &'ast syn::ImplItemFn) {
            if method.sig.ident == "run" {
                visit::visit_impl_item_fn(self, method);
            }
        }
        fn visit_macro(&mut self, mac: &'ast syn::Macro) {
            fn collect(
                tokens: proc_macro2::TokenStream,
                plans: &mut Vec<cq_ir::rust_access_plan::Plan>,
            ) {
                if let Ok(plan) = syn::parse2::<cq_ir::rust_access_plan::Plan>(tokens.clone()) {
                    cq_ir::rust_access_plan::contract::check(&plan).unwrap();
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
    let mut regions = Regions::default();
    regions.visit_file(&target);
    assert!(
        regions.0.len() >= 4,
        "all relational work should already be Access regions"
    );
    assert!(!rust.contains("Counters::default"));
}

#[test]
fn each_binding_and_access_choice_keeps_its_own_macro_path() {
    let source: ra::Module = syn::parse_quote! {
        struct P;
        relation R(k:i64); relation S(k:i64); relation T(k:i64);
        answer(k) :- R(k), S(k), T(k);
        relational {
            r = rename R {k -> k}; s = rename S {k -> k}; t = rename T {k -> k};
            r1 = semijoin r with s; r2 = semijoin r1 with t;
            result = project r2 keep {k}; output result as answer(k);
        }
    };
    let mut options = options(&source);
    options.inputs = std::collections::BTreeMap::from([
        ("R".into(), syn::parse_quote!(providers::blocks)),
        ("S".into(), syn::parse_quote!(providers::hash)),
        ("T".into(), syn::parse_quote!(providers::sorted)),
    ]);
    options
        .results
        .insert("result".into(), syn::parse_quote!(providers::sorted));
    options
        .scans
        .insert("r1".into(), syn::parse_quote!(accesses::prune));
    options
        .key_sets
        .insert("r1".into(), syn::parse_quote!(accesses::zone));
    options.key_sets.insert(
        "r2".into(),
        syn::parse_quote!(accesses::separately_bound_keys),
    );
    let target = compile_physical(&source, &options).unwrap();
    let rust = prettyplease::unparse(&target);
    for selected in [
        "providers::blocks!",
        "providers::hash!",
        "providers::sorted!",
        "accesses::prune!",
        "accesses::zone!",
        "accesses::scan!",
        "accesses::separately_bound_keys!",
    ] {
        assert!(rust.contains(selected), "missing {selected}: {rust}");
    }
    let compact: String = rust.chars().filter(|ch| !ch.is_whitespace()).collect();
    assert!(compact.contains("accesses::separately_bound_keys!"));
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
    let run_case = |provider: &str,
                    scan: &str,
                    resource: &str,
                    keys: &str,
                    expected: Option<&str>| {
        let source = format!(
            r#"
            macro_rules! wrong_hash_rows {{
                (type $arity:literal) => {{
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
            macro_rules! blocks {{
                ($($op:tt)*) => {{ mini_linq::block_rows! {{ storage cq_storage; $($op)* }} }};
            }}
            macro_rules! prune {{
                ($($op:tt)*) => {{ mini_linq::prune_rows! {{ iterator mini_linq::stream_access; $($op)* }} }};
            }}
            macro_rules! keys {{
                ($($op:tt)*) => {{ {resource}! {{ storage cq_storage; iterator mini_linq::stream_access; keys {keys}; $($op)* }} }};
            }}
            mini_linq::physical_plan! {{
                access mini_linq::stream_access;
                inputs {{ R => {provider}; S => mini_linq::hash_rows; }}
                materialize mini_linq::hash_rows;
                scan {scan};
                keys keys;
                join mini_linq::hash_witnesses;
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
        "mini_linq::scan_rows",
        "mini_linq::exact_keys",
        "::std::collections::HashSet",
        None,
    );
    // Explicitly selecting zone pruning on native row storage fails at its
    // requested operation. There is no capability-tag precheck or fallback.
    run_case(
        "mini_linq::hash_rows",
        "prune",
        "mini_linq::zone_keys",
        "::std::collections::HashSet",
        Some("no block descriptor lowering"),
    );
    // Block-capable rows do not manufacture metadata missing from exact keys.
    run_case(
        "blocks",
        "prune",
        "mini_linq::exact_keys",
        "::std::collections::HashSet",
        Some("metadata rejection requires a summary implementation"),
    );
    // This provider expands every operation, but its chosen element type makes
    // the generated array access ill typed. rustc owns that diagnostic.
    run_case(
        "wrong_hash_rows",
        "mini_linq::scan_rows",
        "mini_linq::exact_keys",
        "::std::collections::HashSet",
        Some("cannot index into a value of type"),
    );
    // A familiar method spelling does not silently stand in for the operation
    // trait and its stated exactness law.
    run_case(
        "mini_linq::hash_rows",
        "mini_linq::scan_rows",
        "mini_linq::exact_keys",
        "MethodOnlyKeys",
        Some("ExactMembership"),
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn native_access_fragments_reject_the_removed_relational_envelope() {
    for source in [
        "q = project r keep {k}; { r(k) => rows { current }; }",
        "t = semijoin l with q; { l(k) => rows { current }; q(k) { keys }; }",
        "storage data; access collect; work (&counter); bindings {}",
    ] {
        assert!(syn::parse_str::<cq_ir::physical_provider::Operation>(source).is_err());
        assert!(syn::parse_str::<cq_ir::rust_access_plan::Plan>(source).is_err());
    }
}
