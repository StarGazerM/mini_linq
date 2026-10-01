//! Compile and run every planner/split/reducer for opaque partial Rust steps.
use quote::{ToTokens, quote};

#[test]
fn every_planner_keeps_partial_steps_in_their_written_domain() {
    let queries: [(&str, proc_macro2::TokenStream); 3] = [
        (
            "struct Q; relation R(x:i32); relation S(w:i32); answer(x,y) :- R(x), S(w), if *x != 0 && *w > 1, let y:i32 = 10 / *x;",
            quote!(let R: &[(i32,)] = &[(0,), (2,)]; let S: &[(i32,)] = &[];),
        ),
        (
            "struct Q; relation R(x:i32); relation S(k:i32,y:i32); answer(x,y) :- R(x), if *x != 0, S(10 / *x,y);",
            quote!(let R: &[(i32,)] = &[(0,)]; let S: &[(i32,i32)] = &[(10,4)];),
        ),
        (
            "struct Q; relation R(x:i32); relation S(w:i32); answer(y,w) :- R(x), if *x != 0, let y:i32 = 10 / *x, S(w);",
            quote!(let R: &[(i32,)] = &[(0,)]; let S: &[(i32,)] = &[(1,), (2,)];),
        ),
    ];
    let mut tests = Vec::new();
    for (query, bindings) in queries {
        let source = cq_lang_cq::normal::normalize(&syn::parse_str(query).unwrap()).unwrap();
        for heuristic in [
            cq_order::Heuristic::Written,
            cq_order::Heuristic::Prefix(cq_order::Ranking::Connected),
            cq_order::Heuristic::Prefix(cq_order::Ranking::GreedyWidth),
            cq_order::Heuristic::Prefix(cq_order::Ranking::Auto),
            cq_order::Heuristic::Elimination,
        ] {
            for split in [cq_order::Split::Coarse, cq_order::Split::Fine] {
                let options = cq_order::Options { heuristic, split };
                let ordered = cq_order::compile(&source, &options).unwrap();
                cq_order::check(&source, &ordered, &options).unwrap();
                for guards in [cq_reduce::Pushdown::None, cq_reduce::Pushdown::Atoms] {
                    for semijoins in [cq_reduce::Reductions::None, cq_reduce::Reductions::Tree] {
                        let reduction = cq_reduce::Options { guards, semijoins };
                        let vo = cq_reduce::compile(&ordered, &reduction).unwrap();
                        cq_reduce::check(&ordered, &vo, &reduction).unwrap();
                        let ra = cq_elaborate::compile(&vo).unwrap();
                        cq_elaborate::check(&vo, &ra).unwrap();
                        let ix = cq_index_policy::compile(&ra).unwrap();
                        cq_index_policy::check(&ra, &ix).unwrap();
                        let policy = syn::parse_quote!(policy {});
                        let physical = cq_physical_lowering::compile(&ix, &policy).unwrap();
                        cq_physical_lowering::check(&ix, &physical, &policy).unwrap();
                        let structures =
                            cq_structure_selection::compile(&physical, &policy).unwrap();
                        cq_structure_selection::check(&physical, &structures, &policy).unwrap();
                        let expression = cq_physical_realization::compile(
                            &structures,
                            &cq_physical_realization::Endpoints {
                                distinct_state: syn::parse_quote!(distinct),
                                index_provider: syn::parse_quote!(indexes),
                                intersect: false,
                            },
                        )
                        .unwrap();
                        tests.push(quote!({ #bindings let rows: Vec<_> = (#expression).collect(); assert!(rows.is_empty()); }));
                    }
                }
            }
        }
    }
    // The right keyed input contains the chain itself. Its lazy index is
    // initialized by nonempty T, but the earlier empty S prevents the bind.
    let ra = syn::parse_quote!(relational {
        relation R(x:i32); relation S(w:i32); relation T(y:i32);
        r = rename R {x -> x}; s = rename S {w -> w};
        j = natural_join r with s;
        g = guard j if (*x != 0 && *w > 1);
        b = bind g with y:i32 = (10 / *x);
        t = rename T {y -> y}; indexed = natural_join t with b;
        p = project indexed keep {x,y}; output p as answer(x,y);
    });
    let ix = cq_index_policy::compile(&ra).unwrap();
    let policy = syn::parse_quote!(policy {});
    let physical = cq_physical_lowering::compile(&ix, &policy).unwrap();
    cq_physical_lowering::check(&ix, &physical, &policy).unwrap();
    let structures = cq_structure_selection::compile(&physical, &policy).unwrap();
    assert!(
        structures
            .structures
            .entries
            .iter()
            .any(|entry| entry.name == "b")
    );
    let expression = cq_physical_realization::compile(
        &structures,
        &cq_physical_realization::Endpoints {
            distinct_state: syn::parse_quote!(distinct),
            index_provider: syn::parse_quote!(indexes),
            intersect: false,
        },
    )
    .unwrap();
    tests.push(quote!({
        let R: &[(i32,)] = &[(0,)]; let S: &[(i32,)] = &[]; let T: &[(i32,)] = &[(1,)];
        let rows: Vec<_> = (#expression).collect(); assert!(rows.is_empty());
    }));
    let program = quote! {
        #![allow(non_snake_case, unused_variables)]
        macro_rules! distinct {
            (operator $id:ident; new) => { ::std::collections::HashSet::new() };
            (operator $id:ident; insert($set:expr, $row:expr)) => { $set.insert(Clone::clone($row)) };
        }
        macro_rules! indexes {
            (probe) => { ::cq_storage::HashIndex<'_, _, _> };
            (semi) => { ::std::collections::HashSet<_> };
            (ordered) => { ::std::collections::BTreeMap<_, Vec<_>> };
            (trie $inner:ty) => { ::cq_storage::Trie<'_, _, $inner> };
        }
        fn main() { #(#tests)* }
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = std::env::temp_dir().join(format!("cq-denotation-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.rs"), program.to_token_stream().to_string()).unwrap();
    let runtime = std::process::Command::new("rustc")
        .args([
            "--edition=2024",
            "--crate-name",
            "cq_storage",
            "--crate-type",
            "rlib",
        ])
        .arg(root.join("runtime/cq-storage/src/lib.rs"))
        .arg("--out-dir")
        .arg(&dir)
        .output()
        .unwrap();
    assert!(
        runtime.status.success(),
        "{}",
        String::from_utf8_lossy(&runtime.stderr)
    );
    let build = std::process::Command::new("rustc")
        .arg("--edition=2024")
        .arg(dir.join("main.rs"))
        .arg("--extern")
        .arg(format!(
            "cq_storage={}",
            dir.join("libcq_storage.rlib").display()
        ))
        .arg("-o")
        .arg(dir.join("run"))
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new(dir.join("run"))
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    std::fs::remove_dir_all(dir).unwrap();
}
