use cq_lang_sequence::{Program, contract};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};

fn pipeline(query: TokenStream) -> cq_lang_structures::Program {
    let cq: cq_lang_cq::Program = syn::parse2(query).unwrap();
    let normal = cq_lang_cq::normal::normalize(&cq).unwrap();
    let vo = cq_order::compile(&normal, &Default::default()).unwrap();
    let ra = cq_elaborate::compile(&vo).unwrap();
    let ix = cq_index_policy::compile(&ra).unwrap();
    let physical = cq_physical_lowering::compile(&ix).unwrap();
    cq_structure_selection::compile(&physical).unwrap()
}

fn assert_answers(
    structures: cq_lang_structures::Program,
    data: TokenStream,
    expected: TokenStream,
) -> TokenStream {
    let program = cq_sequence_lowering::compile(&structures).unwrap();
    cq_sequence_lowering::check(&structures, &program).unwrap();
    let analysis = contract::check(&program).unwrap();
    let sequence = cq_sequence_pull::translate(&program, &analysis)
        .unwrap_or_else(|error| panic!("{error}\n{}", program.to_token_stream()));
    quote!({
        #data
        let actual: ::std::collections::BTreeSet<_> = (#sequence).into_iter().collect();
        let expected: ::std::collections::BTreeSet<_> = (#expected).into_iter().collect();
        assert_eq!(actual, expected);
    })
}

#[test]
fn reserve_previous_rejects_non_arena_pull_terminals() {
    let structures = pipeline(quote! {
        struct Query;
        relation R(a:i32);
        answer(a) :- R(a);
    });
    let mut program = cq_sequence_lowering::compile(&structures).unwrap();
    program.sequence.output = Some(cq_lang_sequence::OutputProtocol::ReservePrevious);
    let analysis = contract::check(&program).unwrap();
    for result in [
        cq_sequence_pull::translate(&program, &analysis),
        cq_sequence_pull::translate_profile(&program, &analysis),
    ] {
        let error = result.unwrap_err().to_string();
        assert!(
            error.contains("output reserve_previous requires an arena terminal"),
            "{error}"
        );
    }
}

#[test]
fn cq_pipeline_answers_equal_independent_expected_sets() {
    let data = quote! {
        let L = vec![(1i32, 2i32), (1, 2), (2, 3), (3, 4)];
        let R = vec![(2i32, 7i32), (2, 8), (3, 9), (5, 10)];
        let S = vec![(7i32, 42i32), (8, 41), (9, 42)];
    };
    let declarations = quote! {
        struct Query;
        relation L(a:i32,b:i32);
        relation R(b:i32,c:i32);
        relation S(c:i32,d:i32);
    };
    let queries = [
        (
            quote!(answer(a,c) :- L(a,b), R(b,c);),
            quote!([(&1i32, &7i32), (&1, &8), (&2, &9)]),
        ),
        (
            quote!(answer(a,b,c) :- L(a,b), R(b,c), S(c,42);),
            quote!([(&1i32, &2i32, &7i32), (&2, &3, &9)]),
        ),
        (quote!(answer(a) :- L(a,b), R(b,7);), quote!([(&1i32,)])),
        (
            quote!(answer(a,b) :- L(a,b), R(b,_);),
            quote!([(&1i32, &2i32), (&2, &3)]),
        ),
        (
            quote!(answer(a) :- L(a,_);),
            quote!([(&1i32,), (&2,), (&3,)]),
        ),
        (
            quote!(answer(a,c) :- L(a,b), { R(b,c) } else { S(c,b) };),
            quote!([(&1i32, &7i32), (&1, &8), (&2, &9)]),
        ),
        (
            quote!(answer(a,c) :- L(a,b), { R(b,c) } | { S(c,b) };),
            quote!([(&1i32, &7i32), (&1, &8), (&2, &9)]),
        ),
    ];
    let checks = queries
        .into_iter()
        .map(|(query, expected)| {
            assert_answers(
                pipeline(quote!(#declarations #query)),
                data.clone(),
                expected,
            )
        })
        .collect::<Vec<_>>();
    run(quote!(fn main() { #(#checks)* }));
}

#[test]
fn closed_relational_operands_run_once_and_rust_stays_verbatim() {
    let structures = pipeline(quote! {
        struct Query;
        relation R(a:i32);
        answer(a) :- R(a), where a >= minimum(), if (*a + 1) > 2;
    });
    let program = cq_sequence_lowering::compile(&structures).unwrap();
    let expression =
        cq_sequence_pull::translate(&program, &contract::check(&program).unwrap()).unwrap();
    let printed = expression.to_token_stream().to_string();
    assert!(printed.contains("(* a + 1) > 2"), "{printed}");
    assert!(!printed.contains("wrapping_"), "{printed}");
    assert_eq!(printed.matches("minimum ()").count(), 1, "{printed}");
    run(quote! {
        fn main() {
            let calls = ::std::cell::Cell::new(0);
            let minimum = || { calls.set(calls.get() + 1); 2i32 };
            let R = [(1i32,), (2,), (3,)];
            let actual: ::std::collections::BTreeSet<_> = (#expression).into_iter().collect();
            assert_eq!(actual, [(&2i32,), (&3,)].into_iter().collect());
            assert_eq!(calls.get(), 1);
        }
    });
}

#[test]
fn closed_path_parameters_preserve_streaming() {
    let structures = pipeline(quote! {
        struct Query;
        relation R(a:i32);
        answer(a) :- R(a), where a >= minimum;
    });
    let program = cq_sequence_lowering::compile(&structures).unwrap();
    let expression =
        cq_sequence_pull::translate(&program, &contract::check(&program).unwrap()).unwrap();
    let printed = expression.to_token_stream().to_string();
    assert!(printed.contains("= & minimum ;"), "{printed}");
    assert!(
        !printed.contains("collect :: < :: std :: vec :: Vec"),
        "{printed}"
    );
    run(quote! {
        fn main() {
            let minimum = 2i32;
            let R = [(1i32,), (2,), (3,)];
            let actual: ::std::collections::BTreeSet<_> = (#expression).into_iter().collect();
            assert_eq!(actual, [(&2i32,), (&3,)].into_iter().collect());
        }
    });
}

#[test]
fn generators_preserve_capture_types_and_row_borrowing() {
    let scalar = assert_answers(
        pipeline(quote! {
            struct Query;
            relation R(k:i32);
            relation answer(y:i32);
            answer(y) :- R(k), for y in 0..limit;
        }),
        quote! { let limit = 2i32; let R = [(1i32,), (2,)]; },
        quote!([(0i32,), (1,)]),
    );
    let row = assert_answers(
        pipeline(quote! {
            struct Query;
            relation R(values:Vec<i32>);
            relation answer(y:&i32);
            answer(y) :- R(x), for y in x.iter();
        }),
        quote! { let R = [(vec![1i32, 2],), (vec![3],)]; },
        quote!([(&1i32,), (&2,), (&3,)]),
    );
    let mixed = assert_answers(
        pipeline(quote! {
            struct Query;
            relation R(values:Vec<i32>);
            relation answer(y:&i32);
            answer(y) :- R(x), for y in x.iter().chain(extra.iter());
        }),
        quote! {
            let R = [(vec![1i32, 2],), (vec![3],)];
            let extra = vec![4i32];
        },
        quote!([(&1i32,), (&2,), (&3,), (&4,)]),
    );
    run(quote!(fn main() { #scalar #row #mixed }));
}

fn run(source: TokenStream) {
    let source_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../runtime/cq-storage/src/lib.rs");
    let runtime = cq_testkit::rustc::Runtime::new(&source_path, false);
    runtime.run(&source.to_string());
}

#[test]
fn deliberately_false_evidence_preserves_generated_code_and_answers() {
    let structures = pipeline(quote! {
        struct Query;
        relation R(a:i32,b:i32);
        relation S(a:i32);
        answer(a,b) :- R(a,b), S(a);
    });
    let source = cq_sequence_lowering::compile(&structures).unwrap();
    let mut advised = source.clone();
    advised.evidence = syn::parse_quote! {
        evidence {
            count R = 0;
            key R [a];
            foreign_key R [a] -> S [a];
            selectivity R = 0 / 1;
            bounds R.a = 100 ..= 200;
            sorted R [a];
        }
    };
    cq_lang_sequence::check_rewrite(&source, &advised).unwrap();
    let original =
        cq_sequence_pull::translate(&source, &contract::check(&source).unwrap()).unwrap();
    let estimated =
        cq_sequence_pull::translate(&advised, &contract::check(&advised).unwrap()).unwrap();
    assert_eq!(
        original, estimated,
        "evidence must not alter execution premises"
    );
    run(quote! {
        fn main() {
            let R = vec![(2i32, 20i32), (1, 10), (1, 11), (9, 90), (1, 10)];
            let S = vec![(1i32,), (2,)];
            let original: ::std::collections::BTreeSet<_> = (#original).into_iter().collect();
            let estimated: ::std::collections::BTreeSet<_> = (#estimated).into_iter().collect();
            let expected: ::std::collections::BTreeSet<_> =
                [(&1i32, &10i32), (&1, &11), (&2, &20)].into_iter().collect();
            assert_eq!(original, expected);
            assert_eq!(estimated, expected);
        }
    });
}

fn simple(body: TokenStream) -> Program {
    let physical = syn::parse_quote! {
        physical {
            relation R(a:i32);
            r = pipe canon rename R@0 {a -> a};
            return r as answer(a);
        }
    };
    Program {
        structures: cq_structure_selection::compile(&physical).unwrap(),
        sequence: syn::parse2(quote!(sequence { #body })).unwrap(),
        graded: None,
        evidence: Default::default(),
        representation: Default::default(),
    }
}

#[test]
fn sequence_owned_store_keeps_independent_row_accounting_live() {
    let program = simple(quote! {
        input = store rows r;
        out = store canon input;
    });
    let analysis = contract::check(&program).unwrap();
    for output in [
        cq_sequence_pull::translate(&program, &analysis).unwrap(),
        cq_sequence_pull::translate_profile(&program, &analysis).unwrap(),
    ] {
        let printed = output.to_token_stream().to_string();
        assert!(printed.contains(
            "let __cq_sequence_store_input_allocation = :: cq_storage :: retain_rows (& __cq_sequence_store_input)"
        ), "{printed}");
    }
}

#[test]
fn constructors_execute_and_choose_uses_the_else_arm() {
    let cases = [
        (
            quote!(out = store canon flatten tile 3 rows r;),
            quote!(vec![(&1i32,), (&2i32,)]),
        ),
        (quote!(out = store fold count rows r;), quote!(vec![3usize])),
        (quote!(out = store fold or rows r;), quote!(vec![true])),
        (quote!(out = store fold sum a rows r;), quote!(vec![4i32])),
        (
            quote!(out = store canon choose [len R < 2 => rows r, else => rows r];),
            quote!(vec![(&1i32,), (&2i32,)]),
        ),
    ];
    let checks = cases
        .into_iter()
        .map(|(body, expected)| {
            let program = simple(body);
            let analysis = contract::check(&program).unwrap();
            let output = cq_sequence_pull::translate(&program, &analysis).unwrap();
            quote!({
                let R = vec![(1i32,), (1,), (2,)];
                assert_eq!((#output).into_iter().collect::<Vec<_>>(), #expected);
            })
        })
        .collect::<Vec<_>>();
    run(quote!(fn main() { #(#checks)* }));
}

#[test]
fn translation_requires_host_grades_but_does_not_recheck_the_program() {
    let program = simple(quote!(out = store canon rows r;));
    let mut analysis = contract::check(&program).unwrap();
    analysis
        .levels
        .insert("out".into(), vec!["host".into(), "pool".into()]);
    assert!(cq_sequence_pull::translate(&program, &analysis).is_err());
    analysis.levels.insert("out".into(), vec!["host".into()]);
    assert!(
        cq_sequence_pull::translate(&program, &analysis)
            .unwrap()
            .to_token_stream()
            .to_string()
            .contains("collect")
    );
}

#[test]
fn stored_row_lookup_uses_exact_buckets_without_range_cursor() {
    let physical = syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32);
            l = pipe rename L@0 {a -> a};
            r = let rename R@1 {a -> a,b -> b};
            j = pipe canon probe first [a] l,r;
            return j as answer(a,b);
        }
    };
    let mut structures = cq_structure_selection::compile(&physical).unwrap();
    for entry in &mut structures.structures.entries {
        if let cq_lang_structures::Structure::Built { keyed, .. } = &mut entry.structure {
            keyed.ty = syn::parse_quote!(::cq_storage::SortedRun<_, _>);
        }
    }
    let program = Program {
        structures,
        sequence: syn::parse_quote!(sequence {
            right = store rows r;
            expanded = store flatten lookup r [a] rows l;
            out = store canon expanded;
        }),
        graded: None,
        evidence: Default::default(),
        representation: Default::default(),
    };
    let analysis = contract::check(&program).unwrap();
    assert_eq!(
        analysis.capabilities[&("r".into(), vec!["a".into()])],
        cq_lang_sequence::Capability::Exact
    );
    let output = cq_sequence_pull::translate(&program, &analysis).unwrap();
    assert!(!output.to_token_stream().to_string().contains("RangeCursor"));
    run(quote!(
        fn main() {
            let L = vec![(1i32,), (3,), (1,)];
            let R = vec![(1i32, 7i32), (1, 8), (2, 9)];
            assert_eq!((#output).into_iter().collect::<Vec<_>>(), vec![(&1, &7), (&1, &8)]);
        }
    ));
}

#[test]
fn canon_sorts_and_uniques_without_requiring_build() {
    let program = simple(quote!(out = store canon rows r;));
    let analysis = contract::check(&program).unwrap();
    let output = cq_sequence_pull::translate(&program, &analysis).unwrap();
    run(quote!(
        fn main() {
            let R = vec![(2i32,), (1,), (2,)];
            assert_eq!((#output).into_iter().collect::<Vec<_>>(), vec![(&1,), (&2,)]);
        }
    ));
}

#[test]
fn membership_and_payload_reads_share_the_same_provider() {
    let physical = syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32);
            l = pipe rename L@0 {a -> a};
            r = let rename R@1 {a -> a,b -> b};
            s = pipe semi eager [a] l,r;
            j = pipe canon probe first [a] s,r;
            return j as answer(a,b);
        }
    };
    let check = assert_answers(
        cq_structure_selection::compile(&physical).unwrap(),
        quote! {
            let L = vec![(1i32,), (3,), (1,)];
            let R = vec![(1i32, 7i32), (1, 8), (2, 9)];
        },
        quote!([(&1i32, &7i32), (&1, &8)]),
    );
    run(quote!(fn main() { #check }));
}

#[test]
fn non_host_exists_is_stored_before_a_membership_read() {
    let physical = syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(b:i32);
            l = pipe rename L@0 {a -> a};
            r = pipe rename R@1 {b -> b};
            s = exists canon project r keep {};
            j = pipe canon probe first [] l,s;
            return j as answer(a);
        }
    };
    let check = assert_answers(
        cq_structure_selection::compile(&physical).unwrap(),
        quote! {
            let L = vec![(1i32,), (3,), (1,)];
            let R = vec![(7i32,), (8,)];
        },
        quote!([(&1i32,), (&3,)]),
    );
    run(quote!(fn main() { #check }));
}

#[test]
fn lifted_maps_folds_and_canon_preserve_tile_boundaries() {
    let cases = [
        (
            quote!(out = store map r tile 2 rows r;),
            quote!(vec![(&1i32,), (&1,), (&2,)]),
        ),
        (
            quote!(out = store fold count tile 2 rows r;),
            quote!(vec![2usize, 1]),
        ),
        (
            quote!(out = store canon tile 2 rows r;),
            quote!(vec![(&1i32,), (&2,)]),
        ),
        (
            quote!(out = store fold sum a tile 2 rows r;),
            quote!(vec![2i32, 2]),
        ),
        (
            quote!(out = store flatten tile 1 tile 2 rows r;),
            quote!(vec![(&1i32,), (&1,), (&2,)]),
        ),
    ];
    let checks = cases
        .into_iter()
        .map(|(body, expected)| {
            let program = simple(body);
            let analysis = contract::check(&program).unwrap();
            let output = cq_sequence_pull::translate(&program, &analysis).unwrap();
            quote!({
                let R = vec![(1i32,), (1,), (2,)];
                assert_eq!((#output).into_iter().collect::<Vec<_>>(), #expected);
            })
        })
        .collect::<Vec<_>>();
    run(quote!(fn main() { #(#checks)* }));
}

#[test]
fn canonical_consumer_receives_duplicate_witnesses_without_local_answer_sets() {
    let mut checks = Vec::new();
    for head in [quote!(a), quote!(*a % 2)] {
        let physical = syn::parse2(quote! {
            physical {
                relation R(a:i32);
                consumer canonical;
                r = pipe rename R@0 {a -> a};
                return r as answer(#head);
            }
        })
        .unwrap();
        let structures = cq_structure_selection::compile(&physical).unwrap();
        let sequence = cq_sequence_lowering::compile(&structures).unwrap();
        let analysis = contract::check(&sequence).unwrap();
        let output = cq_sequence_pull::translate(&sequence, &analysis).unwrap();
        let printed = output.to_token_stream().to_string();
        assert!(!printed.contains("__cq_seen"), "{printed}");
        assert!(!printed.contains("HashSet"), "{printed}");
        checks.push(quote! {
            {
                let R = [(1i32,), (1,), (3,)];
                let rows: Vec<_> = (#output).into_iter().collect();
                assert_eq!(rows.len(), 3);
            }
        });
    }
    run(quote!(fn main() { #(#checks)* }));
}

#[test]
fn canonical_consumer_preserves_internal_canon() {
    let physical = syn::parse_quote! {
        physical {
            relation R(a:i32,b:i32);
            consumer canonical;
            r = pipe canon rename R@0 {a -> a,b -> b};
            p = pipe project r keep {a};
            return p as answer(a);
        }
    };
    let structures = cq_structure_selection::compile(&physical).unwrap();
    let sequence = cq_sequence_lowering::compile(&structures).unwrap();
    let analysis = contract::check(&sequence).unwrap();
    let output = cq_sequence_pull::translate(&sequence, &analysis).unwrap();
    let printed = output.to_token_stream().to_string();
    assert_eq!(printed.matches("dedup :: sort").count(), 1, "{printed}");
    run(quote! {
        fn main() {
            let R = [(1i32, 10i32), (1, 10), (1, 20)];
            let rows: Vec<_> = (#output).into_iter().collect();
            assert_eq!(rows, vec![(&1i32,), (&1i32,)]);
        }
    });
}

#[test]
fn ordered_canon_consumes_verified_source_without_hashing() {
    let structures: cq_lang_structures::Program = syn::parse_quote! {
        physical {
            relation R(a:i32,b:i32);
            r = pipe canon rename R@0 {a -> x,b -> y};
            return r as answer(x,y);
        }
        structures {
            r host scan = ::core::iter::Map<_,_> [];
            r canon = ordered;
        }
    };
    let program = cq_sequence_lowering::compile(&structures).unwrap();
    let analysis = contract::check(&program).unwrap();
    let output = cq_sequence_pull::translate(&program, &analysis).unwrap();
    let printed = output.to_token_stream().to_string();
    assert!(printed.contains("ordered_ref"), "{printed}");
    assert!(!printed.contains("HashSet"), "{printed}");
    run(quote! {
        fn main() {
            let R = cq_storage::dedup::verify_prefix_order(
                vec![(1i32, 9i32), (1, 3), (1, 9), (2, 4), (2, 4)],
                |left, right| left.0.cmp(&right.0),
            ).unwrap();
            let rows: Vec<_> = (#output).collect();
            assert_eq!(rows, vec![(&1, &3), (&1, &9), (&2, &4)]);
            assert_eq!(R.len(), 5);
        }
    });
    let mut detached = program.clone();
    detached.sequence.definitions[0].term = syn::parse_quote!(canon store rows r);
    assert!(contract::check(&detached).is_err());
}
