//! Structures → Rust: authored modules print the statements and the pull
//! iterator this pass owns — one build and guard per structure, host maps
//! read in place, merges over native or sorted cursors, bounded scans,
//! extends, and the distinct wrapper — with answers in head order.
use cq_lang_structures as structures;
use cq_physical_realization::{Endpoints, compile};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};

fn endpoints() -> Endpoints {
    Endpoints {
        intersect: false,
        distinct_state: syn::parse_quote!(mini_linq::distinct_state),
    }
}

fn text(tokens: impl ToTokens) -> String {
    tokens.to_token_stream().to_string()
}

fn program(module: &structures::Program) -> String {
    let expr = compile(module, &endpoints()).unwrap();
    text(expr)
}

/// `answer(c, a) :- L(a), R(b, a), S(b, c)`: the head reverses the sorted
/// heading `[a, c]`, and `R` declares `b` before `a`.
fn chain(physical: TokenStream, entries: TokenStream) -> structures::Program {
    let mut module: structures::Program = syn::parse_quote! {
            physical {
    relation L(a: i32);
    relation R(b: i32, a: i32);
    relation S(b: i32, c: i32);
     #physical }
            structures { #entries }
        };
    for entry in &module.structures.entries {
        if let structures::Structure::Host(host) = &entry.structure {
            let d = module
                .physical
                .program
                .declarations
                .iter_mut()
                .find(|d| d.name == entry.name)
                .unwrap();
            let (key, canon) = (&host.key, &host.canon);
            d.attrs.push(syn::parse_quote!(#[host(#key #canon)]));
        }
    }
    module
}

#[test]
fn hash_chain_builds_each_structure_once_and_answers_in_head_order() {
    let module = chain(
        quote! {
            l = alias rename L {a -> a}; r = let rename R {b -> b,a -> a}; s = let rename S {b -> b,c -> c};
            j0 = pipe probe[a] l, r; j1 = pipe probe[b] j0, s;
            p = pipe canon project j1 keep {a,c}; return p as answer(c,a);
        },
        quote! { L; R; S; r = ::cq_storage::HashIndex [a]; s = ::cq_storage::HashIndex [b]; },
    );
    let printed = program(&module);
    // One build per structure, from ((key,), (payload,)) pairs of the host
    // scan rebound to sorted heading order, each followed by its guard.
    let expected_r: syn::Stmt = syn::parse_quote! {
        let __cq_r_on_a = &{
            use ::cq_storage::Build as _;
            ::cq_storage::HashIndex::build(::core::iter::Iterator::map(
                ::core::iter::Iterator::map(R.iter(), move |(b, a,)| (a, b,)),
                |(a, b,)| ((a,), (b,))
            ))
        };
    };
    assert!(printed.contains(&text(expected_r)), "{printed}");
    assert!(
        printed.contains(":: cq_storage :: require :: seek (__cq_r_on_a) ;"),
        "{printed}"
    );
    assert!(
        printed.contains(":: cq_storage :: require :: seek (__cq_s_on_b) ;"),
        "{printed}"
    );
    assert_eq!(printed.matches(":: cq_storage :: require").count(), 2);
    assert!(!printed.contains("__cq_r_rows"), "{printed}");
    // Probes look the built structure up by a tuple of column references
    // and read the payload by position; nothing is cloned.
    let expected_probe: syn::Expr = syn::parse_quote! {
        ::core::iter::Iterator::flat_map(
            ::core::iter::Iterator::map(L.iter(), move |(a,)| (a,)),
            move |(a,)| {
                ::core::iter::Iterator::map(
                    ::cq_storage::IndexRead::lookup(&__cq_r_on_a, &(a,)),
                    move |__cq_v| (a, __cq_v.0,)
                )
            }
        )
    };
    assert!(printed.contains(&text(expected_probe)), "{printed}");
    assert!(!printed.contains("Clone"), "{printed}");
    // The projection drops b; the canonical return is deduplicated through
    // the caller's provider; the answer follows the head, not the heading.
    assert!(
        printed.contains("move | (a , _ , c ,) | (a , c ,)"),
        "{printed}"
    );
    assert!(
        printed.contains("mini_linq :: distinct_state ! { operator p ; new }"),
        "{printed}"
    );
    assert!(
        printed.contains("insert (__cq_seen , __cq_row)"),
        "{printed}"
    );
    assert!(
        printed.contains("move | (a , c ,) | (c , a ,)"),
        "{printed}"
    );
    // Structures live for the block: the answers are collected inside it.
    assert!(printed.starts_with("{ let __cq_r_on_a"), "{printed}");
    assert!(
        printed.contains(":: core :: iter :: IntoIterator :: into_iter (:: core :: iter :: Iterator :: collect :: < :: std :: vec :: Vec < _ > > ("),
        "{printed}"
    );
}

#[test]
fn host_maps_are_read_in_place_with_constructed_keys_in_declared_order() {
    let module = chain(
        quote! {
            l = alias rename L {a -> a}; r = alias rename R {b -> b,a -> a}; s = alias rename S {b -> b,c -> c};
            j0 = pipe probe[a] l, r; j1 = pipe probe[b] j0, s;
            p = pipe canon project j1 keep {a,c}; return p as answer(c,a);
        },
        quote! { L; R keyed [a]; S keyed [b]; },
    );
    let printed = program(&module);
    // Nothing is built, so the block is the lazy iterator itself.
    assert!(
        printed.starts_with("{ :: cq_storage :: require :: keyed"),
        "{printed}"
    );
    assert!(!printed.contains("collect"), "{printed}");
    // The host map is entered with a key the map's owned type imposes; its
    // payload is R's remaining declared column, `b`.
    let expected: syn::Expr = syn::parse_quote! {
        ::core::iter::Iterator::map(
            ::cq_storage::IndexRead::lookup(&&R, &(::core::clone::Clone::clone(a),)),
            move |__cq_v| (a, &__cq_v.0,)
        )
    };
    assert!(printed.contains(&text(expected)), "{printed}");
    assert!(
        printed.contains("lookup (& & S , & (:: core :: clone :: Clone :: clone (b) ,))"),
        "{printed}"
    );
}

#[test]
fn keyed_hosts_scan_their_groups_and_semijoins_test_membership() {
    let module: structures::Program = syn::parse_quote! {
            physical {
    relation L(a: i32);
    #[host(keyed [b])] relation R(b: i32, a: i32);
     r = alias rename R {b -> b, a -> a}; l = let rename L {a -> a}; x = pipe canon semi[a] r, l; return x as answer(a, b); }
            structures { L; R keyed [b]; l = ::std::collections::HashSet<_> [a]; }
        };
    let printed = program(&module);
    // A keyed host scanned in full: every (key, group) pair, then each row.
    let scan: syn::Expr = syn::parse_quote! {
        ::core::iter::Iterator::flat_map(R.iter(), move |((b,), __cq_values)| {
            ::core::iter::Iterator::map(__cq_values.iter(), move |(a,)| (a, b,))
        })
    };
    assert!(printed.contains(&text(scan)), "{printed}");
    // Written type arguments select the qualified build; the semijoin binds
    // only its key and tests the built set.
    assert!(
        printed.contains("let __cq_l_on_a = & < :: std :: collections :: HashSet < _ > as :: cq_storage :: Build < _ > > :: build ("),
        "{printed}"
    );
    assert!(
        printed.contains(":: cq_storage :: require :: contains (__cq_l_on_a) ;"),
        "{printed}"
    );
    assert!(
        printed.contains("move | & (a , _ ,) | :: cq_storage :: ExactMembership :: contains (& __cq_l_on_a , & (a ,))"),
        "{printed}"
    );
    assert!(
        printed.contains("move | (a , b ,) | (a , b ,)") || printed.contains("(a , b ,)"),
        "{printed}"
    );
}

#[test]
fn merge_chain_co_iterates_native_cursors_and_sorts_a_pipe_side() {
    // j0 merges two built ordered maps natively; j1 merges the pipe j0,
    // sorted on b by its own merge, with the host S ordered on b.
    let module: structures::Program = syn::parse_quote! {
            physical {
    relation L(b: i32);
    relation R(b: i32, a: i32);
    #[host(keyed [b] ordered)] relation S(b: i32, c: i32);

                l = let rename L {b -> b}; r = let rename R {b -> b, a -> a}; s = alias rename S {b -> b, c -> c};
                j0 = pipe merge[b] l, r; j1 = pipe merge[b] j0, s;
                p = pipe canon project j1 keep {a, c}; return p as answer(a, c);
            }
            structures {
                L; R; S keyed [b] ordered;
                l = ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> [b];
                r = ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> [b];
            }
        };
    let printed = program(&module);
    assert!(
        printed.contains(":: cq_storage :: require :: ordered (__cq_l_on_b) ;"),
        "{printed}"
    );
    assert!(
        printed.contains(":: cq_storage :: require :: ordered (__cq_r_on_b) ;"),
        "{printed}"
    );
    let native: syn::Expr = syn::parse_quote! {
        ::core::iter::Iterator::map(
            ::cq_storage::merge(
                ::cq_storage::OrderedRead::ordered(&__cq_l_on_b),
                ::cq_storage::OrderedRead::ordered(&__cq_r_on_b)
            ),
            move |(__cq_k, __cq_l, __cq_r)| (__cq_r.0, __cq_k.0,)
        )
    };
    assert!(printed.contains(&text(native)), "{printed}");
    // The pipe side is presented as a sorted cursor keyed by (b,); the host
    // side, being of another kind, is re-keyed the same way.
    assert!(
        printed.contains(":: cq_storage :: sorted_by (:: core :: iter :: Iterator :: map (:: core :: iter :: Iterator :: map (:: cq_storage :: merge ("),
        "{printed}"
    );
    assert!(
        printed.contains("move | (a , b ,) | ((b ,) , (a ,))"),
        "{printed}"
    );
    let host_side: syn::Expr = syn::parse_quote! {
        ::cq_storage::sorted_by(::core::iter::Iterator::map(
            ::cq_storage::OrderedRead::ordered(&S),
            |(__cq_k, __cq_v)| ((&__cq_k.0,), __cq_v)
        ))
    };
    assert!(printed.contains(&text(host_side)), "{printed}");
    assert!(
        printed.contains(
            "move | (__cq_k , __cq_l , __cq_r) | (__cq_l . 0 , __cq_k . 0 , & __cq_r . 0 ,)"
        ),
        "{printed}"
    );
}

#[test]
fn range_enters_an_ordered_host_at_the_bounds_and_keeps_the_residual() {
    let module: structures::Program = syn::parse_quote! {
            physical {
    #[host(keyed [a] ordered canon)] relation R(a: i32, b: i32);
     r = alias rename R {a -> a, b -> b}; w = pipe range[a] r if (a >= 2 && a < 9 && b != 5); return w as answer(a, b); }
            structures { R keyed [a] ordered canon; }
        };
    let printed = program(&module);
    assert!(
        printed.starts_with("{ :: cq_storage :: require :: canonical (& R) ;"),
        "{printed}"
    );
    let expected: syn::Expr = syn::parse_quote! {
        ::core::iter::Iterator::filter(
            {
                let mut __cq_cursor = ::cq_storage::OrderedRead::ordered(&R);
                ::cq_storage::OrderedCursor::seek(&mut __cq_cursor, &&(2i32,));
                ::core::iter::Iterator::map(
                    ::core::iter::Iterator::take_while(__cq_cursor, |(__cq_k, _)| **__cq_k <= (8i32,)),
                    move |(__cq_k, __cq_v)| (&__cq_k.0, &__cq_v.0,)
                )
            },
            move |#[allow(unused_variables)] &(a, b,)| *b != 5i32
        )
    };
    assert!(printed.contains(&text(expected)), "{printed}");
    // Canonical by the host fact: no distinct wrapper, no collection.
    assert!(!printed.contains("distinct_state"), "{printed}");
    assert!(!printed.contains("collect"), "{printed}");
}

#[test]
fn range_over_a_built_ordered_map_uses_reference_keys() {
    let module: structures::Program = syn::parse_quote! {
            physical {
    relation R(a: i32, b: i32);
     r = let rename R {a -> a, b -> b}; w = pipe canon range[a] r if (a > 2); return w as answer(a, b); }
            structures { R; r = ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> [a]; }
        };
    let printed = program(&module);
    assert!(
        printed.contains("seek (& mut __cq_cursor , & & (& 3i32 ,))"),
        "{printed}"
    );
    assert!(!printed.contains("take_while"), "{printed}");
    assert!(
        printed.contains("move | (__cq_k , __cq_v) | (__cq_k . 0 , __cq_v . 0 ,)"),
        "{printed}"
    );
    assert!(
        printed.contains(":: cq_storage :: require :: ordered (__cq_r_on_a) ;"),
        "{printed}"
    );
}

#[test]
fn extend_binds_the_declared_columns_and_the_head_orders_them() {
    let module: structures::Program = syn::parse_quote! {
            physical {
    relation R(a: i32, b: i32);
    relation S(b: i32, c: i32);

                r = alias rename R {a -> a, b -> b}; s = let rename S {b -> b, c -> c}; j = pipe probe[b] r, s;
                e = pipe extend j with {s: String} value ({ let s: String = format!("{}", a); (s,) }); p = pipe canon project e keep {a, c, s}; return p as answer(a, s, c);
            }
            structures { R; S; s = ::cq_storage::HashIndex [b]; }
        };
    let printed = program(&module);
    let extend: syn::Expr = syn::parse_quote! {
        ::core::iter::Iterator::map(
            ::core::iter::Iterator::flat_map(
                ::core::iter::Iterator::map(R.iter(), move |(a, b,)| (a, b,)),
                move |(a, b,)| {
                    ::core::iter::Iterator::map(
                        ::cq_storage::IndexRead::lookup(&__cq_s_on_b, &(b,)),
                        move |__cq_v| (a, b, __cq_v.0,)
                    )
                }
            ),
            move |(a, b, c,)| {
                let (s,): (String,) = { { let s: String = format!("{}", a); (s,) } };
                (a, b, c, s,)
            }
        )
    };
    assert!(printed.contains(&text(extend)), "{printed}");
    // The projection drops b; its heading is [a, c, s]; the answer is (a, s, c).
    assert!(
        printed.contains("move | (a , _ , c , s ,) | (a , c , s ,)"),
        "{printed}"
    );
    assert!(
        printed.contains("move | (a , c , s ,) | (a , s , c ,)"),
        "{printed}"
    );
}

#[test]
fn computed_head_arguments_see_columns_as_references() {
    let module: structures::Program = syn::parse_quote! {
            physical {
    relation R(a: i32, b: i32);
    relation answer(a: i32, d: i32);
     r = alias rename R {a -> a, b -> b}; e = pipe canon extend r with {n: i32} value ((*b,)); return e as answer(a, b + b); }
            structures { R; }
        };
    let printed = program(&module);
    // Variables stay as bound; expressions are evaluated first with every
    // column, computed ones included, shadowed as `&T`.
    let expected: syn::Expr = syn::parse_quote! {
        ::core::iter::Iterator::map(
            {
                let mut __cq_seen = mini_linq::distinct_state! { operator e; new };
                ::core::iter::Iterator::filter(
                    ::core::iter::Iterator::map(
                        ::core::iter::Iterator::map(R.iter(), move |(a, b,)| (a, b,)),
                        move |(a, b,)| { let (n,): (i32,) = { (*b,) }; (a, b, n,) }
                    ),
                    move |__cq_row| { mini_linq::distinct_state! { operator e; insert(__cq_seen, __cq_row) } }
                )
            },
            move |#[allow(unused_variables)] (a, b, n,)| {
                let (__cq_h1,) = { #[allow(unused_variables)] let n = &n; (b + b,) };
                (a, __cq_h1,)
            }
        )
    };
    assert!(printed.contains(&text(expected)), "{printed}");
}

#[test]
fn a_canonical_let_is_a_set_of_its_rows_and_scans_read_it() {
    // `x` is read through a key by `y` and scanned by the return, so it is a
    // `let`; flagged `canon`, its rows are collected into a set.
    let module: structures::Program = syn::parse_quote! {
            physical {
    relation R(a: i32);
    #[host(keyed [a])] relation S(a: i32);
     r = alias rename R {a -> a}; s = alias rename S {a -> a}; x = let canon semi[a] r, s; return x as answer(a); }
            structures { R; S keyed [a]; }
        };
    let printed = program(&module);
    assert!(
        printed.contains("let __cq_x_on_a = & < :: std :: collections :: HashSet < _ > as :: cq_storage :: Build < _ > > :: build ("),
        "{printed}"
    );
    assert!(
        printed.contains(":: cq_storage :: require :: canonical (__cq_x_on_a) ;"),
        "{printed}"
    );
    assert!(
        printed.contains(":: core :: iter :: Iterator :: copied (__cq_x_on_a . iter ())"),
        "{printed}"
    );
    assert!(!printed.contains("__cq_x_rows"), "{printed}");
    assert!(!printed.contains("distinct_state"), "{printed}");
}

#[test]
fn unserved_reads_are_errors_not_fallbacks() {
    // A host ordered by a key that is not the merge's order is rejected.
    let module: structures::Program = syn::parse_quote! {
            physical {
    #[host(keyed [a, b] ordered)] relation L(a: i32, b: i32);
    #[host(keyed [b, a] ordered)] #[host(keyed [a] ordered canon)] relation R(b: i32, a: i32);
     l = alias rename L {a -> a, b -> b}; r = alias rename R {b -> b, a -> a}; j = pipe canon merge[a, b] l, r; return j as answer(a, b); }
            structures { L keyed [a, b] ordered; R keyed [b, a] ordered; }
        };
    let error = compile(&module, &endpoints()).unwrap_err().to_string();
    assert!(error.contains("R"), "{error}");
    // A range whose condition has no literal bound on its column.
    let module: structures::Program = syn::parse_quote! {
            physical {
    #[host(keyed [b, a] ordered)] #[host(keyed [a] ordered canon)] relation R(a: i32, b: i32);
     r = alias rename R {a -> a, b -> b}; w = pipe range[a] r if (a < b); return w as answer(a, b); }
            structures { R keyed [a] ordered canon; }
        };
    let error = compile(&module, &endpoints()).unwrap_err().to_string();
    assert!(error.contains("w"), "{error}");
}

#[allow(dead_code)]
#[path = "../../../test-support/relational/cases.rs"]
mod cases;
#[path = "../../../test-support/relational/scenarios.rs"]
mod scenarios;

#[test]
fn complete_scenarios_compile_and_execute_against_authored_answers() {
    use std::process::Command;
    let directory = std::env::temp_dir().join(format!("cq-realization-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let endpoints = Endpoints {
        distinct_state: syn::parse_quote!(state),
        intersect: false,
    };
    let mut checks = Vec::new();
    for case in scenarios::SCENARIOS {
        let source: structures::Program = syn::parse_str(&case.structured()).unwrap();
        let expression = compile(&source, &endpoints).unwrap();
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
        macro_rules! state {
            (operator $id:ident; new) => { ::std::collections::HashSet::new() };
            (operator $id:ident; insert($state:expr, $row:expr)) => { $state.insert(Clone::clone($row)) };
        }
        fn main() { #(#checks)* }
    };
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let runtime = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
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
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn every_host_fact_emits_a_guard_even_when_only_scanned() {
    for (facts, guards) in [
        ("", (false, false, false)),
        ("canon", (true, false, false)),
        ("keyed [b,a]", (false, true, false)),
        ("keyed [b,a] ordered, canon", (true, true, true)),
    ] {
        let host: structures::syntax::HostFacts = syn::parse_str(facts).unwrap();
        let (key, canon) = (&host.key, &host.canon);
        let module: structures::Program = syn::parse_quote! {
            physical {
                #[host(#host)] relation R(a: &str, b: u64);
                r = alias canon rename R {a -> a,b -> b}; return r as answer(a,b);
            }
            structures { R #key #canon; }
        };
        let printed = program(&module);
        for (guard, present) in [
            ("canonical", guards.0),
            ("keyed", guards.1),
            ("ordered", guards.2),
        ] {
            assert_eq!(
                printed.contains(&format!(":: cq_storage :: require :: {guard}")),
                present,
                "{printed}"
            );
        }
        if guards.1 {
            assert!(
                printed.contains("keyed :: < (u64 , & str ,) , _ > (& R)"),
                "{printed}"
            );
        }
    }
}

#[test]
fn named_hosts_preserve_the_existing_guard_order() {
    let module: structures::Program = syn::parse_quote! {
        physical {
            #[host(canon)] relation R(a:i32);
            #[host(canon)] relation S(a:i32);
            r = alias rename R {a -> a}; return r as answer(a);
        }
        structures { r#S canon; R canon; }
    };
    let printed = program(&module);
    assert!(printed.starts_with("{ :: cq_storage :: require :: canonical (& S) ; :: cq_storage :: require :: canonical (& R) ;"), "{printed}");
}
