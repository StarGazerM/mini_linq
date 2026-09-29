//! Structures → Rust: authored modules print the statements and the pull
//! iterator this pass owns — one build and guard per structure, host maps
//! read in place, merges over native or sorted cursors, bounded scans,
//! extends, and the distinct wrapper — with answers in head order.
use cq_lang_structures as structures;
use cq_physical_realization::{Endpoints, check, compile};
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

fn program(module: &structures::Module) -> String {
    let expr = compile(module, &endpoints()).unwrap();
    assert!(check(module, &expr, &endpoints()));
    text(expr)
}

/// `answer(c, a) :- L(a), R(b, a), S(b, c)`: the head reverses the sorted
/// heading `[a, c]`, and `R` declares `b` before `a`.
fn chain(physical: TokenStream, entries: TokenStream) -> structures::Module {
    syn::parse_quote! {
        struct Chain;
        relation L(a: i32);
        relation R(b: i32, a: i32);
        relation S(b: i32, c: i32);
        answer(c, a) :- L(a), R(b, a), S(b, c);
        relational {
            l = rename L {a -> a};
            r = rename R {b -> b, a -> a};
            s = rename S {b -> b, c -> c};
            j0 = natural_join l with r;
            j1 = natural_join j0 with s;
            p = project j1 keep {a, c};
            output p as answer(c, a);
        }
        indexes { i0 = @r[0]; i1 = @s[0]; }

        physical { #physical }
        structures { #entries }
    }
}

#[test]
fn hash_chain_builds_each_structure_once_and_answers_in_head_order() {
    let module = chain(
        quote! {
            l = alias rename; r = let rename; s = let rename;
            j0 = pipe probe[a] l, r; j1 = pipe probe[b] j0, s;
            p = pipe canon project; return p;
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
            l = alias rename; r = alias rename; s = alias rename;
            j0 = pipe probe[a] l, r; j1 = pipe probe[b] j0, s;
            p = pipe canon project; return p;
        },
        quote! { L; R keyed [a]; S keyed [b]; },
    );
    let printed = program(&module);
    // Nothing is built, so the block is the lazy iterator itself.
    assert!(
        printed.starts_with("{ :: core :: iter :: Iterator :: map ("),
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
    let module: structures::Module = syn::parse_quote! {
        struct Reduced;
        relation L(a: i32);
        relation R(b: i32, a: i32);
        answer(a, b) :- R(b, a), L(a);
        relational {
            r = rename R {b -> b, a -> a};
            l = rename L {a -> a};
            x = semijoin r with l;
            output x as answer(a, b);
        }
        indexes { i0 = @l[0]; }

        physical { r = alias rename; l = let rename; x = pipe canon semi[a] r, l; return x; }
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
    let module: structures::Module = syn::parse_quote! {
        struct Merged;
        relation L(b: i32);
        relation R(b: i32, a: i32);
        relation S(b: i32, c: i32);
        answer(a, c) :- L(b), R(b, a), S(b, c);
        relational {
            l = rename L {b -> b};
            r = rename R {b -> b, a -> a};
            s = rename S {b -> b, c -> c};
            j0 = natural_join l with r;
            j1 = natural_join j0 with s;
            p = project j1 keep {a, c};
            output p as answer(a, c);
        }
        indexes { i0 = @r[1]; i1 = @s[0]; }

        physical {
            l = let rename; r = let rename; s = alias rename;
            j0 = pipe merge[b] l, r; j1 = pipe merge[b] j0, s;
            p = pipe canon project; return p;
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
    let module: structures::Module = syn::parse_quote! {
        struct Windowed;
        relation R(a: i32, b: i32);
        answer(a, b) :- R(a, b), if a >= 2 && a < 9 && b != 5;
        relational {
            r = rename R {a -> a, b -> b};
            w = select r if (a >= 2 && a < 9 && b != 5);
            output w as answer(a, b);
        }
        indexes { }

        physical { r = alias rename; w = pipe range[a]; return w; }
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
    let module: structures::Module = syn::parse_quote! {
        struct Windowed;
        relation R(a: i32, b: i32);
        answer(a, b) :- R(a, b), if a > 2;
        relational {
            r = rename R {a -> a, b -> b};
            w = select r if (a > 2);
            output w as answer(a, b);
        }
        indexes { }

        physical { r = let rename; w = pipe canon range[a]; return w; }
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
    let module: structures::Module = syn::parse_quote! {
        struct Computed;
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        answer(a, s, c) :- R(a, b), S(b, c), let s: String = format!("{}", a);
        relational {
            r = rename R {a -> a, b -> b};
            s = rename S {b -> b, c -> c};
            j = natural_join r with s;
            e = extend j with {s: String} value ({ let s: String = format!("{}", a); (s,) });
            p = project e keep {a, c, s};
            output p as answer(a, s, c);
        }
        indexes { i0 = @s[0]; }

        physical {
            r = alias rename; s = let rename; j = pipe probe[b] r, s;
            e = pipe extend; p = pipe canon project; return p;
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
    let module: structures::Module = syn::parse_quote! {
        struct Doubled;
        relation R(a: i32, b: i32);
        relation answer(a: i32, d: i32);
        answer(a, b + b) :- R(a, b), let n: i32 = *b;
        relational {
            r = rename R {a -> a, b -> b};
            e = extend r with {n: i32} value ((*b,));
            output e as answer(a, b + b);
        }
        indexes { }

        physical { r = alias rename; e = pipe canon extend; return e; }
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
    let module: structures::Module = syn::parse_quote! {
        struct Twice;
        relation R(a: i32);
        relation S(a: i32);
        answer(a) :- R(a), S(a);
        relational {
            r = rename R {a -> a};
            s = rename S {a -> a};
            x = semijoin r with s;
            output x as answer(a);
        }
        indexes { i0 = @s[0]; }

        physical { r = alias rename; s = alias rename; x = let canon semi[a] r, s; return x; }
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
    let module: structures::Module = syn::parse_quote! {
        struct Misordered;
        relation L(a: i32, b: i32);
        relation R(b: i32, a: i32);
        answer(a, b) :- L(a, b), R(b, a);
        relational {
            l = rename L {a -> a, b -> b};
            r = rename R {b -> b, a -> a};
            j = natural_join l with r;
            output j as answer(a, b);
        }
        indexes { i0 = @r[0, 1]; }

        physical { l = alias rename; r = alias rename; j = pipe canon merge[a, b] l, r; return j; }
        structures { L keyed [a, b] ordered; R keyed [b, a] ordered; }
    };
    let error = compile(&module, &endpoints()).unwrap_err().to_string();
    assert!(
        error.contains("host `R` is ordered by [\"b\", \"a\"]"),
        "{error}"
    );
    // A range whose condition has no literal bound on its column.
    let module: structures::Module = syn::parse_quote! {
        struct Unbounded;
        relation R(a: i32, b: i32);
        answer(a, b) :- R(a, b), if a < b;
        relational {
            r = rename R {a -> a, b -> b};
            w = select r if (a < b);
            output w as answer(a, b);
        }
        indexes { }

        physical { r = alias rename; w = pipe range[a]; return w; }
        structures { R keyed [a] ordered canon; }
    };
    let error = compile(&module, &endpoints()).unwrap_err().to_string();
    assert!(
        error.contains("does not compare with an integer literal"),
        "{error}"
    );
}
