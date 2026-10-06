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

/// Default selection and printed Rust must agree with an independent CQ Set
/// oracle for every host/retained orientation, including shared storage reads.
fn mixed_smallest_checks() -> Vec<proc_macro2::TokenStream> {
    let query: cq_testkit::cq::Program = syn::parse_quote!(
        struct Mixed;
        relation L(a:i32,b:i32); relation R(b:i32,c:i32); relation S(a:i32,c:i32);
        answer(a,b,c) :- L(a,b), R(b,c), S(a,c);
    );
    let mut inputs = vec![[
        vec![
            vec![1, 1],
            vec![1, 1],
            vec![2, 2],
            vec![3, 3],
            vec![4, 4],
            vec![5, 5],
            vec![6, 6],
        ],
        vec![
            vec![1, 10],
            vec![1, 11],
            vec![1, 12],
            vec![1, 12],
            vec![2, 20],
            vec![3, 30],
            vec![3, 31],
            vec![4, 40],
            vec![6, 60],
        ],
        vec![
            vec![1, 10],
            vec![2, 20],
            vec![2, 21],
            vec![2, 21],
            vec![3, 32],
            vec![5, 50],
            vec![6, 60],
        ],
    ]];
    for seed in 0..24 {
        let database = cq_testkit::databases::binary_database(&query, seed);
        inputs.push(["L", "R", "S"].map(|name| {
            let mut rows: Vec<_> = database[name].iter().cloned().collect();
            if let Some(first) = rows.first().cloned() {
                rows.push(first);
            }
            rows
        }));
    }
    let fixtures: Vec<_> = inputs
        .into_iter()
        .map(|inputs| {
            let database = ["L", "R", "S"]
                .into_iter()
                .zip(&inputs)
                .map(|(name, rows)| (name.to_owned(), rows.iter().cloned().collect()))
                .collect();
            let expected = cq_testkit::cq_semantics::evaluate_cq(&query, &database);
            (inputs, expected)
        })
        .collect();
    assert!(!fixtures[0].1.is_empty());

    let mut programs = Vec::new();
    for (r_retained, s_retained) in [(false, true), (true, false), (true, true), (false, false)] {
        let r = if r_retained {
            "r0 = pipe rename R@0 {b -> b,c -> c}; r = let project r0 keep {b,c};"
        } else {
            "r = pipe rename R@0 {b -> b,c -> c};"
        };
        let s = if s_retained {
            "s0 = pipe rename S@0 {a -> a,c -> c}; s = let project s0 keep {a,c};"
        } else {
            "s = pipe rename S@0 {a -> a,c -> c};"
        };
        let physical: structures::physical::Program = syn::parse_str(&format!(
            "physical {{
                relation L(a:i32,b:i32); relation R(b:i32,c:i32); relation S(a:i32,c:i32);
                l = pipe rename L@0 {{a -> a,b -> b}}; {r} {s}
                candidate = pipe probe smallest [b] l,r;
                checked = pipe canon semi eager [a,c] candidate,s;
                return checked as answer(a,b,c);
            }}"
        ))
        .unwrap();
        programs.push((format!("retained R={r_retained}, S={s_retained}"), physical));
    }
    programs.push((
        "shared payload, membership and ordered reads".into(),
        syn::parse_quote!(physical {
            relation L(a:i32,b:i32); relation R(b:i32,c:i32); relation S(a:i32,c:i32);
            l0 = pipe rename L@0 {a -> a,b -> b};
            r0 = pipe rename R@1 {b -> b,c -> c}; r = let project r0 keep {b,c};
            s = pipe rename S@2 {a -> a,c -> c};
            payload = pipe probe first [b] l0,r;
            l1 = pipe project payload keep {a,b};
            members = let semi eager [c] s,r;
            l2 = pipe semi eager [a] l1,members;
            ordered = let merge[b,c] r,r;
            l = pipe semi eager [b] l2,ordered;
            candidate = pipe probe smallest [b] l,r;
            checked = pipe canon semi eager [a,c] candidate,s;
            return checked as answer(a,b,c);
        }),
    ));

    let mut checks = Vec::new();
    for (label, physical) in programs {
        let target = cq_structure_selection::compile(&physical).unwrap();
        let mut reversed = target.clone();
        reversed.structures.entries.reverse();
        for target in [target, reversed] {
            let expression = compile(&target).unwrap();
            assert!(text(&expression).contains("__cq_choose_a"));
            let assertions = fixtures.iter().enumerate().map(|(case, (inputs, expected))| {
                    let bindings = ["L", "R", "S"].into_iter().zip(inputs).map(|(name, rows)| {
                        let name = syn::Ident::new(name, proc_macro2::Span::call_site());
                        let rows = rows.iter().map(|row| quote!((#(#row,)*)));
                        quote!(let #name: &[(i32, i32)] = &[#(#rows),*];)
                    });
                    let expected = expected.iter().map(|row| quote!((#(#row,)*)));
                    quote!({
                        #(#bindings)*
                        let expected: ::std::collections::BTreeSet<_> = [#(#expected),*].into_iter().collect();
                        assert_eq!(run(L, R, S), expected, "{} case {}", #label, #case);
                    })
                });
            checks.push(quote!({
                    fn run(L: &[(i32, i32)], R: &[(i32, i32)], S: &[(i32, i32)]) -> ::std::collections::BTreeSet<(i32, i32, i32)> {
                        (#expression).map(|(a, b, c)| (*a, *b, *c)).collect()
                    }
                    #(#assertions)*
                }));
        }
    }
    checks
}

fn filtered_checks() -> Vec<proc_macro2::TokenStream> {
    let mut checks = Vec::new();
    for (selection, kept, expected) in [
        (
            "point select r if (a = 7)",
            "a,k,v",
            quote!([(7, 1, 10), (7, 2, 30)]),
        ),
        (
            "point select r if (a = @chosen)",
            "a,k,v",
            quote!([(7, 1, 10), (7, 2, 30)]),
        ),
        (
            "point select r if (a in {7})",
            "a,k,v",
            quote!([(7, 1, 10), (7, 2, 30)]),
        ),
        (
            "point select r if (a in {7,8})",
            "a,k,v",
            quote!([(7, 1, 10), (8, 1, 20), (7, 2, 30)]),
        ),
        ("point select r if (a = 7, a = 8)", "a,k,v", quote!([])),
        (
            "range select r if (v > 15)",
            "a,k,v",
            quote!([(8, 1, 20), (7, 2, 30)]),
        ),
    ] {
        for stream in [false, true] {
            for sorted in [false, true] {
                let rename = if stream {
                    "r0 = pipe rename R@1 {a -> a,k -> k,v -> v,dead -> dead}; r = pipe project r0 keep {a,k,v,dead};"
                } else {
                    "r = pipe rename R@1 {a -> a,k -> k,v -> v,dead -> dead};"
                };
                let source: structures::physical::Program = syn::parse_str(&format!(
                    "physical {{
                    relation L(k:i32); relation R(a:i32,k:i32,v:i32,dead:i32);
                    l = pipe rename L@0 {{k -> k}}; {rename}
                    f = let {selection}; j = pipe probe first [k] l,f;
                    keys = pipe project j keep {{k}};
                    members = pipe semi eager [k] keys,f;
                    shared = pipe probe first [k] members,f;
                    out = pipe canon project shared keep {{{kept}}}; return out as answer(a,k,v);
                }}"
                ))
                .unwrap();
                let mut target = cq_structure_selection::compile(&source).unwrap();
                assert!(
                    structures::contract::check(&target)
                        .unwrap()
                        .filtered
                        .contains_key("f")
                );
                if sorted {
                    for entry in &mut target.structures.entries {
                        if entry.name == "f" {
                            if let structures::Structure::Built { keyed, .. } = &mut entry.structure
                            {
                                keyed.ty = syn::parse_quote!(::std::collections::BTreeMap<_, ::std::vec::Vec<_>>);
                            }
                        }
                    }
                }
                let expression = compile(&target).unwrap();
                assert!(!text(&expression).contains("__cq_f_rows"));
                checks.push(quote!({
                    let L: &[(i32,)] = &[(1,), (1,), (2,), (9,)];
                    let R: &[(i32,i32,i32,i32)] = &[(7,1,10,999), (7,1,10,999), (8,1,20,888), (7,2,30,777)];
                    let chosen = 7;
                    let actual: ::std::collections::BTreeSet<(i32,i32,i32)> = (#expression).map(|(a,k,v)| (*a,*k,*v)).collect();
                    let expected: ::std::collections::BTreeSet<(i32,i32,i32)> = #expected.into_iter().collect();
                    assert_eq!(actual, expected, "{}", #selection);
                }));
            }
        }
    }
    for key in ["a", "a,k"] {
        let source: structures::physical::Program = syn::parse_str(&format!(
            "physical {{
            relation L(a:i32,k:i32); relation R(a:i32,k:i32);
            l = pipe rename L@0 {{a -> a,k -> k}}; r = pipe rename R@1 {{a -> a,k -> k}};
            f = let point select r if (a = 7);
            j = pipe probe first [{key}] l,f;
            out = pipe canon semi eager [{key}] j,f; return out as answer(a,k);
        }}"
        ))
        .unwrap_or_else(|e| panic!("{e}"));
        // For a unary key the two relations must share only that attribute.
        let source = if key == "a" {
            syn::parse_quote!(physical {
                relation L(a:i32); relation R(a:i32,k:i32);
                l = pipe rename L@0 {a -> a}; r = pipe rename R@1 {a -> a,k -> k};
                f = let point select r if (a = 7);
                j = pipe probe first [a] l,f;
                out = pipe canon project j keep {a,k}; return out as answer(a,k);
            })
        } else {
            source
        };
        let target = cq_structure_selection::compile(&source).unwrap();
        let expression = compile(&target).unwrap();
        let input = if key == "a" {
            quote!(let L: &[(i32,)] = &[(7,), (8,)];)
        } else {
            quote!(let L: &[(i32,i32)] = &[(7,1), (8,1), (7,2)];)
        };
        checks.push(quote!({
            #input
            let R: &[(i32,i32)] = &[(7,1), (7,2), (8,1)];
            let actual: ::std::collections::BTreeSet<_> = (#expression).map(|(a,k)| (*a,*k)).collect();
            assert_eq!(actual, [(7,1), (7,2)].into_iter().collect());
        }));
    }
    let source: structures::physical::Program = syn::parse_quote!(physical {
        relation L(k:i32); relation R(tag:String,k:i32);
        l = pipe rename L@0 {k -> k}; r = pipe rename R@1 {tag -> tag,k -> k};
        f = let point select r if (tag = "a");
        j = pipe canon probe first [k] l,f; return j as answer(k,tag);
    });
    let expression = compile(&cq_structure_selection::compile(&source).unwrap()).unwrap();
    checks.push(quote!({
        let L: &[(i32,)] = &[(1,), (2,)];
        let R = vec![(String::from("a"),1), (String::from("b"),2)];
        let actual: Vec<(&i32,&String)> = (#expression).collect();
        assert_eq!(actual, vec![(&1, &String::from("a"))]);
    }));
    for ty in ["&'a str", "&'a String"] {
        let source: structures::physical::Program = syn::parse_str(&format!(
            "physical {{
            relation L(k:i32); relation R(tag:{ty},k:i32);
            l = pipe rename L@0 {{k -> k}}; r = pipe rename R@1 {{tag -> tag,k -> k}};
            f = let point select r if (tag = \"a\");
            j = pipe canon probe first [k] l,f; return j as answer(k,tag);
        }}"
        ))
        .unwrap();
        let expression = compile(&cq_structure_selection::compile(&source).unwrap()).unwrap();
        let ty: syn::Type = syn::parse_str(ty).unwrap();
        let input = if text(&ty).contains("String") {
            quote!(let a = String::from("a"); let b = String::from("b"); let R = [(&a,1),(&b,2)];)
        } else {
            quote!(let R = [("a",1),("b",2)];)
        };
        checks.push(quote!({
            fn run<'a>(L: &'a [(i32,)], R: &'a [(#ty,i32)]) -> Vec<(&'a i32,&'a #ty)> {
                (#expression).collect()
            }
            let L = [(1,), (2,)];
            #input
            assert_eq!(run(&L,&R), vec![(&L[0].0,&R[0].0)]);
        }));
    }
    checks
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
    checks.extend(mixed_smallest_checks());
    checks.extend(filtered_checks());
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
        l = pipe rename L@0 {a -> a}; r = pipe rename R@1 {a -> a};
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
        l = pipe rename L@0 {a -> x}; r = pipe rename R@1 {a -> x,b -> y};
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
        relation R(a:i32,b:i32); r = pipe rename R@0 {a -> x,b -> y};
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
