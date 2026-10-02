//! Structures → Rust: authored modules print the statements and the pull
//! iterator this pass owns — one build and guard per structure, host maps
//! read in place, merges over native or sorted cursors, point reads,
//! opaque binds/guards, and the distinct wrapper — with answers in head order.
use cq_lang_structures as structures;
use cq_physical_realization::compile;
use cq_testkit::{cases, corpus as scenarios};
use quote::{ToTokens, quote};

fn text(tokens: impl ToTokens) -> String {
    tokens.to_token_stream().to_string()
}

fn program(module: &structures::Program) -> String {
    let complete = cq_structure_selection::compile(&module.physical).unwrap();
    let expr = compile(&complete).unwrap();
    text(expr)
}

/// `answer(c, a) :- L(a), R(b, a), S(b, c)`: the head reverses the sorted
/// heading `[a, c]`, and `R` declares `b` before `a`.
#[test]
fn complete_scenarios_compile_and_execute_against_authored_answers() {
    use std::process::Command;
    let directory = std::env::temp_dir().join(format!("cq-realization-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let mut checks = Vec::new();
    for case in scenarios::SCENARIOS {
        let source = case.parse_structures().unwrap();
        let expression = compile(&source).unwrap();
        let data_name = if case.name == "fine_triangle" {
            "triangle"
        } else {
            case.name.trim_end_matches("_reduced")
        };
        let data = cases::CASES.iter().find(|c| c.name == data_name).unwrap();
        let bindings: Vec<_> = source
            .physical
            .inputs()
            .map(|d| {
                let name = &d.name;
                let rows = data.inputs.iter().find(|(n, _)| name == *n).unwrap().1;
                let tuples = rows.iter().map(|r| quote!((#(#r,)*)));
                let types = d.columns.iter().map(|c| &c.ty);
                quote!(let #name: &[(#(#types,)*)] = &[#(#tuples),*];)
            })
            .collect();
        let variables: Vec<_> = (0..source.physical.program.head.args.len())
            .map(|i| quote::format_ident!("v{i}"))
            .collect();
        let copied = variables.iter().map(|v| quote!(*#v));
        let expected = data.expected.iter().map(|r| quote!((#(#r,)*)));
        let label = case.name;
        checks.push(quote!({
            #(#bindings)*
            let actual: ::std::collections::BTreeSet<_> = (#expression).map(|(#(#variables,)*)| (#(#copied,)*)).collect();
            let expected: ::std::collections::BTreeSet<_> = [#(#expected),*].into_iter().collect();
            assert_eq!(actual, expected, #label);
        }));
    }
    let source = quote! {
        #![allow(non_snake_case, unused_variables)]
        fn main() { #(#checks)* }
    };
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let runtime = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("../../runtime/cq-storage/src/lib.rs");
    let compiled = Command::new(&rustc)
        .args([
            "--edition=2024",
            "--crate-name",
            "cq_storage",
            "--crate-type",
            "rlib",
        ])
        .arg(runtime)
        .arg("--out-dir")
        .arg(&directory)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let path = directory.join("main.rs");
    std::fs::write(&path, source.to_string()).unwrap();
    let binary = directory.join("answers");
    let compiled = Command::new(&rustc)
        .arg("--edition=2024")
        .arg(&path)
        .arg("--extern")
        .arg(format!(
            "cq_storage={}",
            directory.join("libcq_storage.rlib").display()
        ))
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let result = Command::new(binary).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    // Direct merge premises are deliberately left to rustc, with the trait diagnostic.
    let unordered: structures::Program = syn::parse_quote!(physical {
        relation L(a:i32); relation R(a:i32);
        l = pipe rename L {a -> a}; r = pipe rename R {a -> a};
        j = pipe canon merge[a] l,r; return j as answer(a);
    } structures {});
    let complete = cq_structure_selection::compile(&unordered.physical).unwrap();
    let expression = compile(&complete).unwrap();
    let invalid = quote!(#source
        #[allow(non_snake_case)] fn unordered_merge() {
            let L: &[(i32,)] = &[(1,)]; let R: &[(i32,)] = &[(1,)];
            let _: Vec<_> = (#expression).collect();
        }
    );
    std::fs::write(&path, invalid.to_string()).unwrap();
    let rejected = Command::new(&rustc)
        .arg("--edition=2024")
        .arg(&path)
        .arg("--extern")
        .arg(format!(
            "cq_storage={}",
            directory.join("libcq_storage.rlib").display()
        ))
        .arg("-o")
        .arg(directory.join("invalid"))
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    let diagnostic = String::from_utf8_lossy(&rejected.stderr);
    assert!(
        diagnostic.contains("this read requires ordered inputs"),
        "{diagnostic}"
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn dispatch_uses_layout_and_allocates_fallbacks_outside_probe_closures() {
    let p: structures::Program = syn::parse_quote!(physical {
        relation L(a:i32); relation R(a:i32,b:i32);
        l = pipe rename L {a -> x}; r = pipe rename R {a -> x,b -> y};
        j = pipe canon probe first [x] l,r; return j as answer(x,y);
    } structures {});
    let text = program(&p);
    assert_eq!(text.matches("Lazy :: new").count(), 1);
    assert!(
        text.contains("& & :: cq_storage :: dispatch :: Host"),
        "{text}"
    );
    assert!(text.contains(". seek ("), "{text}");
    assert!(text.contains("HashSet < _ > > :: new"), "{text}");
    let mut nonleading = p;
    nonleading.physical.program.declarations[1] = syn::parse_quote!(relation R(b:i32,a:i32););
    let text = program(&nonleading);
    assert!(!text.contains("& & :: cq_storage :: dispatch :: Host"));
    assert!(text.contains("& :: cq_storage :: dispatch :: Host"));
}

#[test]
fn point_prints_build_free_dispatch_and_canonical_lets_use_the_distinct_provider() {
    let p: structures::Program = syn::parse_quote!(physical {
        relation R(a:i32,b:i32); r = pipe rename R {a -> x,b -> y};
        w = let canon point select r if (x = 2);
        p = pipe project w keep {x,y}; return p as answer(y,x);
    } structures {});
    let text = program(&p);
    assert_eq!(text.matches("Lazy :: new").count(), 0);
    assert!(text.contains("dispatch :: PointHost"));
    assert!(text.contains(". point ("));
    assert!(text.contains("HashSet < _ > > :: new"));
    assert!(text.contains("(y , x ,)"));
}
