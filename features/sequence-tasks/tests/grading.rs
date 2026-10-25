use cq_lang_sequence::{Program, check_grading};
use cq_sequence_tasks::{machine, translate};
use quote::ToTokens;
use syn::visit_mut::{self, VisitMut};

fn program(sequence: proc_macro2::TokenStream, grades: proc_macro2::TokenStream) -> Program {
    syn::parse2(quote::quote! {
        physical {
            relation R(a:i32);
            r = pipe canon rename R@0 {a -> a};
            return r as answer(a);
        } structures { r host scan = ::core::iter::Map<_, _> []; r canon = ::std::collections::HashSet<_>; }
        sequence { #sequence }
        graded tasks { #grades }
    })
    .unwrap()
}

#[test]
fn checked_publication_owns_rows_once_and_aliases_without_copying() {
    use cq_lang_sequence::Publication;
    let program = program(
        quote::quote! {
            borrowed = store rows r;
            retained = store canon borrowed;
            retained_parts = partition_k[a](64) retained;
            alias = store retained;
            parts = store partition_k[a](64) alias;
            joined = store parts;
            result = flatten joined;
        },
        quote::quote! {
            borrowed at host; retained at host; retained_parts at pool.task;
            alias at host; parts at pool; joined at host; result at host;
        },
    );
    let facts = check_grading(&program, &machine()).unwrap();
    assert!(matches!(
        facts.publications["borrowed"],
        Publication::BorrowHost { .. }
    ));
    assert_eq!(
        facts.publications["retained"],
        Publication::OwnRows { staged: true }
    );
    assert_eq!(
        facts.publications["alias"],
        Publication::Alias {
            source: "retained".into()
        }
    );
    let tasks = translate(&program, &facts).unwrap();
    let tokens = tasks.to_token_stream().to_string();
    assert!(
        tokens.contains("let __cq_key_source_retained ="),
        "{tokens}"
    );
    assert!(!tokens.contains("let __cq_key_source_alias ="), "{tokens}");
    assert!(
        !tokens.contains("let __cq_key_source_borrowed ="),
        "{tokens}"
    );
    run_partition_fixture(quote::quote! {
        fn main() {
            for R in [&[][..], &[(1i32,)][..], &[(2,), (1,), (2,), (3,)][..]] {
                let mut actual: Vec<(i32,)> = (#tasks).map(|(a,)| (*a,)).collect();
                actual.sort();
                let mut expected = R.to_vec();
                expected.sort();
                expected.dedup();
                assert_eq!(actual, expected);
            }
        }
    });
}

#[test]
fn machine_has_separated_dynamic_pool_and_sequential_leaf() {
    let machine = machine();
    assert_eq!(machine.name, "tasks");
    assert_eq!(machine.root.name, "host");
    let pool = &machine.root.children[0];
    assert_eq!(pool.name, "pool");
    assert!(!pool.collaborative);
    assert!(pool.dynamic);
    let task = &pool.children[0];
    assert_eq!(pool.width, Some(64));
    assert_eq!(task.name, "task");
    assert!(!task.collaborative);
    assert!(task.children.is_empty());
}

#[test]
fn collective_at_separated_pool_is_rejected() {
    let program = program(
        quote::quote!(input = store rows r; result = fold count input;),
        quote::quote!(input at host; result at pool;),
    );
    let error = check_grading(&program, &machine()).unwrap_err();
    assert!(error.to_string().contains("BE-Collective"), "{error}");
}

#[test]
fn leaf_task_fold_is_a_sequential_loop() {
    let program = program(
        quote::quote!(input = store rows r; result = store fold count input;),
        quote::quote!(input at host; result at pool.task;),
    );
    let analysis = check_grading(&program, &machine()).unwrap();
    let expression = translate(&program, &analysis)
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(expression.contains("Iterator :: fold"), "{expression}");
    assert!(!expression.contains("rayon"));
}

#[test]
fn flatten_at_pool_is_rejected_even_with_tile() {
    let program = program(
        quote::quote!(input = store rows r; result = flatten tile 2 input;),
        quote::quote!(input at host; result at pool;),
    );
    // Strip-mining does not make the separated pool collaborative.
    let error = check_grading(&program, &machine()).unwrap_err();
    assert!(error.to_string().contains("BE-Collective"), "{error}");
}

#[test]
fn host_placement_does_not_emit_parallel_operations() {
    let program = program(
        quote::quote!(input = store rows r; result = store input;),
        quote::quote!(input at host; result at host;),
    );
    let analysis = check_grading(&program, &machine()).unwrap();
    let expression = translate(&program, &analysis)
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(!expression.contains("rayon"));
}

#[test]
fn pool_tile_and_store_emit_indexed_parallel_buffers() {
    let program = program(
        quote::quote!(input = store rows r; result = store tile 64 input;),
        quote::quote!(input at host; result at pool;),
    );
    let analysis = check_grading(&program, &machine()).unwrap();
    let expression = translate(&program, &analysis).unwrap();
    let rendered = expression.to_token_stream().to_string();
    assert!(rendered.contains("tile_rows (64usize)"), "{rendered}");
    assert!(!worker_bodies(&expression).is_empty());
    assert!(
        rendered.contains("collect :: < :: std :: vec :: Vec < _ > >"),
        "{rendered}"
    );
}

// Inspect the closure passed to the indexed parallel iterator, not merely the
// presence of Rayon: parallel copying of an already-evaluated segment is wrong.
fn worker_bodies(expression: &syn::Expr) -> Vec<syn::Expr> {
    #[derive(Default)]
    struct Workers(Vec<syn::Expr>);
    fn unparen(mut expression: &syn::Expr) -> &syn::Expr {
        while let syn::Expr::Paren(paren) = expression {
            expression = &paren.expr;
        }
        expression
    }

    impl VisitMut for Workers {
        fn visit_expr_method_call_mut(&mut self, call: &mut syn::ExprMethodCall) {
            if (call.method == "map" || call.method == "fold")
                && let syn::Expr::MethodCall(source) = unparen(call.receiver.as_ref())
                && (source.method == "par_chunks" || source.method == "tile_rows")
                && let Some(syn::Expr::Closure(worker)) = call.args.last()
            {
                self.0.push(*worker.body.clone());
            }
            visit_mut::visit_expr_method_call_mut(self, call);
        }
    }

    let mut workers = Workers::default();
    workers.visit_expr_mut(&mut expression.clone());
    workers.0
}

fn segment_program() -> Program {
    let physical = syn::parse_quote! {
        physical {
            relation L(a:i32,x:i32); relation R(a:i32,b:i32);
            consumer canonical;
            l = pipe rename L@0 {a -> a,x -> x};
            r = let rename R@1 {a -> a,b -> b};
            joined = pipe probe first [a] l,r;
            selected = pipe select joined if (b > 0);
            mapped = pipe bind selected with n:i32 = (*b * 100 + 1);
            projected = pipe project mapped keep {a,n};
            return projected as answer(a,n);
        }
    };
    let mut structures = cq_structure_selection::compile(&physical).unwrap();
    let provider: Program = syn::parse_quote! {
        #physical
        structures { r = ::cq_storage::SortedRun<_, _> [a]; }
        sequence { input = store rows l; result = store input; }
    };
    // Keep the same provider override convention as the conformance fixtures.
    for provider in provider.structures.structures.entries {
        structures.structures.entries.retain(|entry| {
            entry.name != provider.name
                || std::mem::discriminant(&entry.structure)
                    != std::mem::discriminant(&provider.structure)
        });
        structures.structures.entries.push(provider);
    }
    syn::parse_quote! {
        #structures
        sequence {
            r = store rows r;
            input = store rows l;
            parts = store capacity 4096 flatten
                map projected map mapped compact selected flatten lookup r [a] tile 64 input;
            result = store parts;
        }
        graded tasks { r at host; input at host; parts at pool; result at host; }
    }
}

#[test]
fn distributing_tile_keeps_the_whole_segment_inside_the_worker() {
    let program = segment_program();
    let analysis = check_grading(&program, &machine()).unwrap();
    assert_eq!(
        analysis.tilings["parts"],
        vec![cq_lang_sequence::Tiling::Distribute]
    );
    let expression = translate(&program, &analysis).unwrap();
    let workers = worker_bodies(&expression);
    assert_eq!(workers.len(), 1, "{}", expression.to_token_stream());
    let worker = workers[0].to_token_stream().to_string();
    for adapter in [
        "Iterator :: map",
        "Iterator :: filter",
        "Iterator :: flat_map",
        "__cq_buffer . extend",
    ] {
        assert!(
            worker.contains(adapter),
            "missing {adapter} inside worker: {worker}"
        );
    }
    assert!(
        worker.contains("IndexRead :: lookup") || worker.contains("RangeCursor :: range"),
        "lookup must execute inside the worker: {worker}"
    );
    // This total scalar expression belongs only to Map, so a parallel lookup
    // followed by a serial map cannot satisfy the worker-local assertion.
    assert!(
        worker.contains("100"),
        "mapped scalar escaped the worker: {worker}"
    );
}

#[test]
fn pool_store_joins_task_local_rows_in_driver_order_without_deduplicating() {
    let program = segment_program();
    let analysis = check_grading(&program, &machine()).unwrap();
    let tasks = translate(&program, &analysis).unwrap();
    let mut sequential = program.clone();
    sequential.graded = Some(syn::parse_quote! {
        graded tasks { r at host; input at host; parts at host; result at host; }
    });
    let facts = check_grading(&sequential, &machine()).unwrap();
    let pull = cq_sequence_pull::translate(&sequential, &facts).unwrap();
    let source = quote::quote! {
        fn main() {
            let R: &[(i32,i32)] = &[(1,10),(1,11),(2,20),(3,-30)];
            let populated: Vec<(i32,i32)> =
                (0..131).map(|index| (index % 4 + 1, index % 7)).collect();
            for threads in [1,4] {
                let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
                for L in [&[][..], &populated[..1], &populated[..64], &populated[..65], &populated[..]] {
                    let expected: Vec<(i32,i32)> = L.iter().flat_map(|&(a,_)| {
                        R.iter().filter_map(move |&(key,b)| {
                            (key == a && b > 0).then_some((a,b * 100 + 1))
                        })
                    }).collect();
                    let actual: Vec<(i32,i32)> = pool.install(|| {
                        (#tasks).into_iter().map(|(a,n)| (*a,n)).collect()
                    });
                    let serial: Vec<(i32,i32)> =
                        (#pull).into_iter().map(|(a,n)| (*a,n)).collect();
                    // No sorting or set comparison: publication order and every
                    // repeated valuation are observable in this raw stream.
                    assert_eq!(actual, expected, "threads={threads}, rows={}", L.len());
                    assert_eq!(actual, serial);
                }
            }
        }
    };
    run_partition_fixture(source);
}

fn grouped_program(global: bool) -> Program {
    let keys = if global {
        quote::quote!()
    } else {
        quote::quote!(k)
    };
    let head_keys = if global {
        quote::quote!()
    } else {
        quote::quote!(k,)
    };
    let physical = syn::parse2(quote::quote! {
        physical {
            relation rows(k:i32, witness:i32, value:i64);
            r = pipe canon rename rows@0 {k -> k, witness -> witness, value -> value};
            grouped = pipe group r by [#keys] {
                n: usize = count();
                total: i64 = sum(value);
                low: Option<i64> = min(value);
                high: Option<i64> = max(value);
            };
            return grouped as answer(#head_keys n,total,low,high);
        }
    })
    .unwrap();
    let structures = cq_structure_selection::compile(&physical).unwrap();
    syn::parse2(quote::quote! {
        #structures
        sequence {
            r = store canon rows r;
            groups = group grouped r;
            output_rows = store groups;
        }
        graded tasks { r at host; groups at host; output_rows at host; }
    })
    .unwrap()
}

#[test]
fn grouped_fold_requires_host_even_at_a_sequential_task_leaf() {
    for level in [quote::quote!(pool), quote::quote!(pool.task)] {
        let mut program = grouped_program(false);
        program.graded = Some(
            syn::parse2(quote::quote! {
                graded tasks { r at host; groups at #level; output_rows at host; }
            })
            .unwrap(),
        );
        let error = check_grading(&program, &machine()).unwrap_err();
        assert!(error.to_string().contains("BE-Group"), "{error}");
    }
}

#[test]
fn host_grouped_tasks_and_pull_execute_the_same_valuation_fold() {
    let keyed = grouped_program(false);
    let global = grouped_program(true);
    let keyed_facts = check_grading(&keyed, &machine()).unwrap();
    let global_facts = check_grading(&global, &machine()).unwrap();
    let keyed_tasks = translate(&keyed, &keyed_facts).unwrap();
    let global_tasks = translate(&global, &global_facts).unwrap();
    let keyed_pull = cq_sequence_pull::translate(&keyed, &keyed_facts).unwrap();
    let global_pull = cq_sequence_pull::translate(&global, &global_facts).unwrap();
    assert!(!keyed_tasks.to_token_stream().to_string().contains("rayon"));
    let source = quote::quote! {
        fn main() {
            let populated: &[(i32,i32,i64)] = &[(1,10,7),(1,10,7),(1,11,7),(2,20,-2)];
            for rows in [populated, &[][..]] {
                let mut tasks: Vec<(i32,usize,i64,Option<i64>,Option<i64>)> =
                    (#keyed_tasks).into_iter().map(|(k,n,s,l,h)| (*k,n,s,l,h)).collect();
                let mut pull: Vec<_> =
                    (#keyed_pull).into_iter().map(|(k,n,s,l,h)| (*k,n,s,l,h)).collect();
                tasks.sort();
                pull.sort();
                assert_eq!(tasks, pull);
                let tasks_global: Vec<(usize,i64,Option<i64>,Option<i64>)> =
                    (#global_tasks).into_iter().collect();
                let pull_global: Vec<_> = (#global_pull).into_iter().collect();
                assert_eq!(tasks_global, pull_global);
                if rows.is_empty() {
                    assert!(tasks.is_empty());
                    assert_eq!(tasks_global, vec![(0,0,None,None)]);
                } else {
                    assert_eq!(tasks, vec![(1,2,14,Some(7),Some(7)), (2,1,-2,Some(-2),Some(-2))]);
                    assert_eq!(tasks_global, vec![(3,12,Some(-2),Some(7))]);
                }
            }
        }
    };
    let runtime = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../runtime/cq-storage/src/lib.rs");
    cq_testkit::rustc::Runtime::new(&runtime, false).run(&source.to_string());
}

#[test]
fn joined_tiles_canonicalize_complete_valuations_before_grouping() {
    let keyed = grouped_program(false);
    let global = grouped_program(true);
    let translate_tiled = |mut program: Program| {
        program.sequence = syn::parse_quote! {
            sequence {
                input = store rows r;
                parts = store flatten tile 64 input;
                joined = store parts;
                r = store canon joined;
                groups = group grouped r;
                output_rows = store groups;
            }
        };
        program.graded = Some(syn::parse_quote! {
            graded tasks {
                input at host; parts at pool; joined at host;
                r at host; groups at host; output_rows at host;
            }
        });
        let facts = check_grading(&program, &machine()).unwrap();
        assert_eq!(
            facts.tilings["parts"],
            vec![cq_lang_sequence::Tiling::Distribute]
        );
        let expression = translate(&program, &facts).unwrap();
        assert_eq!(worker_bodies(&expression).len(), 1);
        expression
    };
    let keyed_tasks = translate_tiled(keyed.clone());
    let global_tasks = translate_tiled(global.clone());
    let keyed_facts = check_grading(&keyed, &machine()).unwrap();
    let global_facts = check_grading(&global, &machine()).unwrap();
    let keyed_pull = cq_sequence_pull::translate(&keyed, &keyed_facts).unwrap();
    let global_pull = cq_sequence_pull::translate(&global, &global_facts).unwrap();
    let source = quote::quote! {
        fn main() {
            // Repeated complete valuations span several 64-row tiles. Different
            // witnesses with the same key and value must both enter the fold.
            let populated: Vec<(i32,i32,i64)> = (0..131).map(|index| match index % 4 {
                0 | 1 => (1,10,7),
                2 => (1,11,7),
                _ => (2,20,-2),
            }).collect();
            for threads in [1,4] {
                let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
                for rows in [&populated[..], &[][..]] {
                    let mut tasks: Vec<(i32,usize,i64,Option<i64>,Option<i64>)> =
                        pool.install(|| (#keyed_tasks).into_iter()
                            .map(|(k,n,s,l,h)| (*k,n,s,l,h)).collect());
                    let mut pull: Vec<_> = (#keyed_pull).into_iter()
                        .map(|(k,n,s,l,h)| (*k,n,s,l,h)).collect();
                    tasks.sort();
                    pull.sort();
                    assert_eq!(tasks, pull);
                    let tasks_global: Vec<(usize,i64,Option<i64>,Option<i64>)> =
                        pool.install(|| (#global_tasks).into_iter().collect());
                    let pull_global: Vec<_> = (#global_pull).into_iter().collect();
                    assert_eq!(tasks_global, pull_global);
                    if rows.is_empty() {
                        assert!(tasks.is_empty());
                        assert_eq!(tasks_global, vec![(0,0,None,None)]);
                    } else {
                        assert_eq!(
                            tasks,
                            vec![(1,2,14,Some(7),Some(7)), (2,1,-2,Some(-2),Some(-2))]
                        );
                        assert_eq!(tasks_global, vec![(3,12,Some(-2),Some(7))]);
                    }
                }
            }
        }
    };
    run_partition_fixture(source);
}

fn run_partition_fixture(source: proc_macro2::TokenStream) {
    let runtime = cq_testkit::rustc::Runtime::with_dependencies(vec![
        cq_testkit::rustc::linked_dependency::<cq_sequence_tasks::Config>(
            "cq_sequence_tasks",
            "cq_sequence_tasks::Config",
        ),
        cq_testkit::rustc::linked_dependency::<cq_storage::sharded::FullKey>(
            "cq_storage",
            "cq_storage::sharded::FullKey",
        ),
        cq_testkit::rustc::linked_dependency::<cq_load::ColumnAt<0>>(
            "cq_load",
            "cq_load::ColumnAt<0>",
        ),
    ]);
    let source = quote::quote! {
        #![allow(non_snake_case, unused_variables)]
        #source
    };
    runtime.run(&source.to_string());
}

#[test]
fn key_partitioned_groups_preserve_exact_valuations_and_empty_global_identity() {
    let keyed = grouped_program(false);
    let global = grouped_program(true);
    let translate_partitioned = |mut program: Program, global: bool| {
        let body = if global {
            quote::quote!(group grouped canon partition_k[k,witness,value](64) input)
        } else {
            quote::quote!(flatten group grouped canon partition_k[k](64) input)
        };
        program.sequence = syn::parse_quote! {
            sequence {
                input = store rows r;
                parts = store #body;
                output_rows = store parts;
            }
        };
        program.graded = Some(syn::parse_quote! {
            graded tasks { input at host; parts at pool; output_rows at host; }
        });
        let facts = check_grading(&program, &machine()).unwrap();
        if global {
            assert_eq!(facts.shapes["parts"], vec![cq_lang_sequence::Len::Lit(1)]);
        }
        let expression = translate(&program, &facts).unwrap();
        let rendered = expression.to_token_stream().to_string();
        assert!(rendered.contains("combine :: route_by"), "{rendered}");
        assert!(rendered.contains("combine :: map_parts"), "{rendered}");
        if global {
            assert!(rendered.contains("let __cq_partials"), "{rendered}");
        }
        expression
    };
    let keyed_tasks = translate_partitioned(keyed.clone(), false);
    let global_tasks = translate_partitioned(global.clone(), true);
    let keyed_facts = check_grading(&keyed, &machine()).unwrap();
    let global_facts = check_grading(&global, &machine()).unwrap();
    let keyed_pull = cq_sequence_pull::translate(&keyed, &keyed_facts).unwrap();
    let global_pull = cq_sequence_pull::translate(&global, &global_facts).unwrap();
    run_partition_fixture(quote::quote! {
        fn main() {
            // Duplicates span several routing chunks; only two of the 64
            // logical key partitions can be occupied by the keyed fold.
            let populated: Vec<(i32,i32,i64)> = (0..4099).map(|index| match index % 4 {
                0 | 1 => (1,10,7),
                2 => (1,11,7),
                _ => (2,20,-2),
            }).collect();
            for threads in [1,24] {
                let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
                for rows in [&populated[..], &[][..]] {
                    let mut actual: Vec<(i32,usize,i64,Option<i64>,Option<i64>)> =
                        pool.install(|| (#keyed_tasks).into_iter()
                            .map(|(k,n,s,l,h)| (*k,n,s,l,h)).collect());
                    let mut expected: Vec<_> = (#keyed_pull).into_iter()
                        .map(|(k,n,s,l,h)| (*k,n,s,l,h)).collect();
                    actual.sort();
                    expected.sort();
                    assert_eq!(actual, expected);
                    let actual_global: Vec<(usize,i64,Option<i64>,Option<i64>)> =
                        pool.install(|| (#global_tasks).into_iter().collect());
                    let expected_global: Vec<_> = (#global_pull).into_iter().collect();
                    assert_eq!(actual_global, expected_global);
                    if rows.is_empty() {
                        assert!(actual.is_empty());
                        assert_eq!(actual_global, vec![(0,0,None,None)]);
                    } else {
                        assert_eq!(
                            actual,
                            vec![(1,2,14,Some(7),Some(7)), (2,1,-2,Some(-2),Some(-2))]
                        );
                        assert_eq!(actual_global, vec![(3,12,Some(-2),Some(7))]);
                    }
                }
            }
        }
    });
}

#[test]
fn globally_unique_tiles_fold_partial_aggregates_with_one_empty_identity() {
    let mut program = grouped_program(true);
    program.evidence = syn::parse_quote!(evidence { rows(witness) key; });
    program.sequence = syn::parse_quote! {
        sequence {
            input = store rows r;
            partials = store group grouped tile 64 input;
            output_rows = store partials;
        }
    };
    program.graded = Some(syn::parse_quote! {
        graded tasks { input at host; partials at pool; output_rows at host; }
    });
    let facts = check_grading(&program, &machine()).unwrap();
    assert_eq!(
        facts.shapes["partials"],
        vec![cq_lang_sequence::Len::Lit(1)]
    );
    assert_eq!(
        facts.tilings["partials"],
        vec![cq_lang_sequence::Tiling::Distribute]
    );
    let expression = translate(&program, &facts).unwrap();
    let rendered = expression.to_token_stream().to_string();
    assert!(rendered.contains("let __cq_partials"), "{rendered}");
    assert!(!rendered.contains("combine :: route_by"), "{rendered}");
    run_partition_fixture(quote::quote! {
        fn main() {
            let populated: Vec<(i32,i32,i64)> =
                (0..131).map(|index| (0,index,i64::from(index % 5 - 2))).collect();
            for threads in [1,24] {
                let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
                for rows in [&populated[..], &[][..]] {
                    let actual: Vec<(usize,i64,Option<i64>,Option<i64>)> =
                        pool.install(|| (#expression).into_iter().collect());
                    if rows.is_empty() {
                        assert_eq!(actual, vec![(0,0,None,None)]);
                    } else {
                        assert_eq!(actual, vec![(131,-2,Some(-2),Some(2))]);
                    }
                }
            }
        }
    });
}

fn routed_provider_program(routing: proc_macro2::TokenStream, named: bool) -> Program {
    let physical = syn::parse_quote! {
        physical {
            relation L(a:i32,b:i32); relation R(a:i32,b:i32,c:i32);
            consumer canonical;
            l = pipe rename L@0 {a -> a,b -> b};
            r = let rename R@1 {a -> a,b -> b,c -> c};
            joined = pipe probe first [a,b] l,r;
            return joined as answer(a,b,c);
        }
    };
    let mut structures = cq_structure_selection::compile(&physical).unwrap();
    let authored: Program = syn::parse_quote! {
        #physical
        structures {
            r = ::cq_storage::sharded::Sharded<::cq_storage::sharded::Index<_, _>, _> [a,b];
        }
        sequence { result = rows l; }
    };
    for provider in authored.structures.structures.entries {
        structures.structures.entries.retain(|entry| {
            entry.name != provider.name
                || std::mem::discriminant(&entry.structure)
                    != std::mem::discriminant(&provider.structure)
        });
        structures.structures.entries.push(provider);
    }
    let (provider, grade) = if named {
        (
            quote::quote! {
                routed = partition_k[#routing](64) provider_input;
                provider = store routed;
            },
            quote::quote!(routed at pool.task;),
        )
    } else {
        (
            quote::quote!(provider = store partition_k[#routing](64) provider_input;),
            quote::quote!(),
        )
    };
    syn::parse_quote! {
        #structures
        sequence {
            provider_input = store rows r;
            #provider
            input = store rows l;
            parts = store capacity 8192 flatten flatten lookup r [a,b] tile 64 input;
            result = store parts;
        }
        graded tasks {
            provider_input at host; #grade provider at pool;
            input at host; parts at pool; result at host;
        }
    }
}

#[test]
fn routed_providers_keep_subset_and_reordered_keys_through_named_publication() {
    let mut checks = Vec::new();
    for routing in [quote::quote!(b), quote::quote!(b, a)] {
        for named in [false, true] {
            let program = routed_provider_program(routing.clone(), named);
            let facts = check_grading(&program, &machine()).unwrap();
            assert_eq!(
                facts.capabilities[&("r".into(), vec!["a".into(), "b".into()])],
                cq_lang_sequence::Capability::Exact,
            );
            let expression = translate(&program, &facts).unwrap();
            let description = format!("routing [{routing}], named={named}");
            checks.push(quote::quote! {
                let actual: Vec<(i32,i32,i32)> = pool.install(|| {
                    (#expression).into_iter().map(|(a,b,c)| (*a,*b,*c)).collect()
                });
                assert_eq!(actual, expected, #description);
            });
        }
    }
    run_partition_fixture(quote::quote! {
        fn main() {
            let probes: Vec<(i32,i32)> = (0..131).map(|i| (i % 4,i % 7)).collect();
            let values: Vec<(i32,i32,i32)> = (0..131).map(|i| (i % 3,i % 5,i % 7)).collect();
            for threads in [1,24] {
                let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
                for L in [&probes[..], &[][..]] {
                    for R in [&values[..], &[][..]] {
                        let expected: Vec<(i32,i32,i32)> = L.iter().flat_map(|&(a,b)| {
                            R.iter().filter_map(move |&(ra,rb,c)| {
                                (ra == a && rb == b).then_some((a,b,c))
                            })
                        }).collect();
                        // Compare raw ordered rows: neither wrong routing nor
                        // lost duplicate payloads can hide behind set equality.
                        #(#checks)*
                    }
                }
            }
        }
    });
}

#[test]
fn catalog_retention_materializes_new_providers_and_preserves_native_entry_identity() {
    let source: Program = syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32); relation S(b:i32,c:i32);
            l = pipe rename L@0 {a -> a};
            r = pipe rename R@1 {a -> a,b -> b};
            s = let rename S@2 {b -> b,c -> c};
            joined = pipe probe first [a] l,r;
            result = pipe canon probe first [b] joined,s;
            return result as answer(a,b,c);
        }
        structures {
            l host scan = Reader [];
            r host scan = Reader [];
            r host joined = Reader [a];
            s host scan = Reader [];
            s = ::cq_storage::SortedRun<_, _, _> [b];
            s rows = ::std::vec::Vec<_>;
            result canon = ::std::collections::HashSet<_>;
        }
        sequence {
            l = rows l;
            r = rows r;
            s = store rows s;
            joined = flatten lookup r[a] l;
            result = canon flatten lookup s[b] joined;
            output_rows = store result;
        }
        graded tasks {
            l at host; r at host; s at host; joined at host;
            result at host; output_rows at host;
        }
    };
    let target =
        cq_sequence_tasks::select_providers(&source, &cq_sequence_tasks::catalog()).unwrap();
    assert!(
        target
            .sequence
            .definitions
            .iter()
            .any(|definition| definition.name == "r"
                && matches!(definition.term, cq_lang_sequence::Term::Store { .. }))
    );
    let native: syn::Type = syn::parse_quote!(::cq_storage::SortedRun<_, _, _>);
    assert!(
        target
            .structures
            .structures
            .entries
            .iter()
            .any(|entry| entry.name == "s"
                && matches!(&entry.structure,
            cq_lang_sequence::structures::Structure::Built { keyed, .. } if keyed.ty == native))
    );
    assert!(
        target
            .structures
            .structures
            .entries
            .iter()
            .any(|entry| entry.name == "s"
                && matches!(&entry.structure,
            cq_lang_sequence::structures::Structure::Rows { ty, .. }
                if *ty == syn::parse_quote!(::std::vec::Vec<_>)))
    );
    cq_lang_sequence::check_rewrite(&source, &target).unwrap();
    let facts = check_grading(&target, &machine()).unwrap();
    translate(&target, &facts).unwrap();
}

#[test]
fn base_relation_tile_uses_borrowed_rows_and_keeps_the_joined_answer() {
    let mut source = segment_program();
    source
        .sequence
        .definitions
        .retain(|definition| definition.name != "input" && definition.name != "result");
    source
        .graded
        .as_mut()
        .unwrap()
        .grades
        .retain(|grade| grade.name != "input" && grade.name != "result");
    fn inline(term: &mut cq_lang_sequence::Term) {
        use cq_lang_sequence::Term;
        match term {
            Term::Ref(name) if name == "input" => *term = syn::parse_quote!(rows l),
            Term::Map(_, inner)
            | Term::Compact(_, inner)
            | Term::Flatten(inner)
            | Term::Lookup { inner, .. }
            | Term::Tile { inner, .. }
            | Term::Store { inner, .. } => inline(inner),
            _ => {}
        }
    }
    inline(&mut source.sequence.definitions.last_mut().unwrap().term);
    let analysis = check_grading(&source, &machine()).unwrap();
    let rendered = translate(&source, &analysis)
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(rendered.contains("TileSource"), "{rendered}");
    assert!(rendered.contains("tile_rows"), "{rendered}");
    assert!(!rendered.contains("let __cq_driver = ("));
    assert!(!rendered.contains("Iterator :: collect :: < :: std :: vec :: Vec < _ > >"));
}

#[test]
fn paired_flatten_fuses_the_following_stream_until_publication() {
    let mut source = segment_program();
    source
        .sequence
        .definitions
        .iter_mut()
        .find(|definition| definition.name == "parts")
        .unwrap()
        .term = syn::parse_quote!(store map projected map mapped compact selected
            flatten lookup r [a] flatten tile 64 input);
    let analysis = check_grading(&source, &machine()).unwrap();
    let expression = translate(&source, &analysis).unwrap();
    let workers = worker_bodies(&expression);
    assert_eq!(workers.len(), 1);
    let body = workers[0].to_token_stream().to_string();
    assert!(
        body.contains("IndexRead :: lookup") || body.contains("RangeCursor :: range"),
        "{body}"
    );
    assert!(body.contains("100"), "{body}");
    assert!(
        !expression
            .to_token_stream()
            .to_string()
            .contains("__cq_sequence_store_input")
    );
}

#[test]
fn task_catalog_retains_membership_fallbacks_before_shared_reads() {
    let source: Program = syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32);
            consumer canonical;
            l = pipe rename L@0 {a -> a};
            r = pipe rename R@1 {a -> a,b -> b};
            result = pipe anti eager [a] l,r;
            return result as answer(a);
        }
        structures {
            l host scan = Reader [];
            r host scan = Reader [];
            r host result = ::cq_storage::dispatch::KeySet<_> [a];
        }
        sequence {
            l = rows l; r = rows r;
            result = compact result lookup r[a] l;
            output_rows = store result;
        }
    };
    let target =
        cq_sequence_tasks::select_providers(&source, &cq_sequence_tasks::catalog()).unwrap();
    assert!(
        target
            .sequence
            .definitions
            .iter()
            .any(|definition| definition.name == "r"
                && matches!(definition.term, cq_lang_sequence::Term::Store { .. }))
    );
    assert!(
        target
            .structures
            .structures
            .entries
            .iter()
            .any(|entry| entry.name == "r"
                && matches!(
                    &entry.structure,
                    cq_lang_sequence::structures::Structure::Built { .. }
                ))
    );
}

#[test]
fn membership_projected_probes_keep_native_keysets() {
    let source: Program = syn::parse_quote! {
        physical {
            relation L(a:i64); relation R(a:i64);
            consumer canonical;
            l = pipe rename L@0 {a -> a};
            r = let rename R@1 {a -> a};
            result = pipe probe first [a] l,r;
            return result as answer(a);
        }
        structures {
            l host scan = Reader []; r host scan = Reader [];
            r = ::cq_storage::AdaptiveKeySet<_, ::cq_storage::BorrowedI64TupleCodec, 16777216>
                [a] membership payload [a] constants {};
        }
        sequence {
            l = rows l; r = store rows r;
            result = compact result lookup r[a] l;
            output_rows = store result;
        }
    };
    let target =
        cq_sequence_tasks::select_providers(&source, &cq_sequence_tasks::catalog()).unwrap();
    let native: syn::Type = syn::parse_quote!(::cq_storage::AdaptiveKeySet<_, ::cq_storage::BorrowedI64TupleCodec, 16777216>);
    assert!(
        target
            .structures
            .structures
            .entries
            .iter()
            .any(|entry| entry.name == "r"
                && matches!(&entry.structure,
            cq_lang_sequence::structures::Structure::Built { keyed, .. } if keyed.ty == native))
    );
}

#[test]
fn adaptive_provider_guard_selects_identity_and_routed_builds_at_runtime() {
    use cq_lang_sequence::{Guard, Len, Term};
    let mut program = routed_provider_program(quote::quote!(a, b), false);
    let part = |parts| Term::PartitionK {
        keys: vec![syn::parse_quote!(a), syn::parse_quote!(b)],
        parts,
        inner: Box::new(Term::Ref(syn::parse_quote!(provider_input))),
    };
    program
        .sequence
        .definitions
        .iter_mut()
        .find(|definition| definition.name == "provider")
        .unwrap()
        .term = Term::Store {
        capacity: None,
        inner: Box::new(Term::Choose(vec![
            (Guard::Lt(Len::Input("provider_input".into()), 4), part(1)),
            (Guard::Else, part(64)),
        ])),
    };
    let facts = check_grading(&program, &machine()).unwrap();
    let mut expression = translate(&program, &facts).unwrap();
    struct ObserveBuild;
    impl VisitMut for ObserveBuild {
        fn visit_expr_call_mut(&mut self, call: &mut syn::ExprCall) {
            visit_mut::visit_expr_call_mut(self, call);
            if matches!(call.func.as_ref(), syn::Expr::Path(path)
                if path.path.segments.last().is_some_and(|segment| segment.ident == "build_from_parts_with_router"))
            {
                let parts = call.args.first().unwrap().clone();
                *call.args.first_mut().unwrap() = syn::parse_quote!(observe_parts(#parts));
            }
        }
    }
    ObserveBuild.visit_expr_mut(&mut expression);
    run_partition_fixture(quote::quote! {
        static PARTS: ::std::sync::atomic::AtomicUsize = ::std::sync::atomic::AtomicUsize::new(0);
        fn observe_parts<R>(parts: &[Vec<R>]) -> &[Vec<R>] {
            PARTS.store(parts.len(), ::std::sync::atomic::Ordering::Relaxed);
            parts
        }
        fn main() {
            let probes = [(1,2),(3,4)];
            let values = [(1,2,8),(3,4,9),(1,2,10),(3,4,11),(1,2,12)];
            for threads in [1,4] {
                let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
                for length in [0,3,4,5] {
                    let L = probes.as_slice();
                    let R = &values[..length];
                    let actual: Vec<_> = pool.install(|| (#expression).map(|(a,b,c)| (*a,*b,*c)).collect());
                    let expected: Vec<_> = L.iter().flat_map(|&(a,b)|
                        R.iter().filter_map(move |&(ra,rb,c)| (a==ra && b==rb).then_some((a,b,c)))).collect();
                    assert_eq!(actual, expected);
                    assert_eq!(PARTS.load(::std::sync::atomic::Ordering::Relaxed), if length < 4 { 1 } else { 64 });
                }
            }
        }
    });
}

#[test]
fn keyed_partial_maps_merge_at_host_and_preserve_empty_input() {
    let mut executions = Vec::new();
    for stored_parts in [false, true] {
        let mut program = grouped_program(false);
        program.evidence = syn::parse_quote!(evidence { rows(witness) key; });
        program.sequence = if stored_parts {
            syn::parse_quote!(sequence {
                input = store tile 64 rows r;
                partials = store group grouped input;
                output_rows = store partials;
            })
        } else {
            syn::parse_quote!(sequence {
                input = store rows r;
                partials = store group grouped tile 64 input;
                output_rows = store partials;
            })
        };
        program.graded = Some(syn::parse_quote! {
            graded tasks { input at host; partials at pool; output_rows at host; }
        });
        let facts = check_grading(&program, &machine()).unwrap();
        assert_eq!(facts.partial_groups["partials"], vec![true]);
        assert_eq!(facts.shapes["partials"].len(), 1);
        let tasks = translate(&program, &facts).unwrap();
        cq_sequence_tasks::translate_profile(&program, &facts).unwrap();
        let mut sequential = program.clone();
        sequential.graded = Some(syn::parse_quote! {
            graded tasks { input at host; partials at host; output_rows at host; }
        });
        let sequential_facts = check_grading(&sequential, &machine()).unwrap();
        let pull = cq_sequence_pull::translate(&sequential, &sequential_facts).unwrap();
        executions.push(quote::quote! {
            let mut actual: Vec<_> = (#tasks).map(|(key,n,sum,low,high)| (*key,n,sum,low,high)).collect();
            actual.sort();
            assert_eq!(actual, expected);
            let mut sequential: Vec<_> = (#pull).map(|(key,n,sum,low,high)| (*key,n,sum,low,high)).collect();
            sequential.sort();
            assert_eq!(sequential, expected);
        });
        program.evidence = Default::default();
        assert!(
            check_grading(&program, &machine())
                .unwrap_err()
                .to_string()
                .contains("BE-GroupPartial")
        );
    }
    run_partition_fixture(quote::quote! {
        fn main() {
            let populated: Vec<(i32,i32,i64)> = (0..4097).map(|i| (i%5,i,i64::from(i%7-3))).collect();
            for threads in [1,24] {
                rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap().install(|| {
                    for length in [0,1,63,64,65,4097] {
                        let rows = &populated[..length];
                        let mut expected = ::std::collections::BTreeMap::<i32,(usize,i64,Option<i64>,Option<i64>)>::new();
                        for &(key,_,value) in rows {
                            let (n,sum,low,high) = expected.entry(key).or_default();
                            *n += 1;
                            *sum += value;
                            *low = Some(low.map_or(value, |previous| previous.min(value)));
                            *high = Some(high.map_or(value, |previous| previous.max(value)));
                        }
                        let expected: Vec<_> = expected.into_iter().map(|(key,(n,sum,low,high))|
                            (key,n,sum,low,high)).collect();
                        #(#executions)*
                    }
                });
            }
        }
    });
}

fn late_covering_program(input: proc_macro2::TokenStream) -> Program {
    let physical = syn::parse_quote! {
        physical {
            relation L(a:i64,b:i64); relation R(c:i64,d:i64);
            l = pipe rename L@0 {a -> x,b -> y};
            r = pipe rename R@1 {c -> x,d -> z};
            s = pipe range select r if (z > 1);
            b = let project s keep {x,z};
            j = pipe probe first [x] l,b;
            out = pipe canon project j keep {x,y,z};
            return out as answer(x,y,z);
        }
    };
    let structures =
        cq_structure_selection::compile_with(&physical, quote::quote!(positional(L, R))).unwrap();
    let source = syn::parse2(quote::quote! {
        #structures
        sequence {
            input = store #input;
            b = store map b map s rows r;
            result = store canon map out flatten lookup b [x] input;
        }
        graded tasks { input at host; b at host; result at host; }
    })
    .unwrap();
    cq_sequence_tasks::select_providers(&source, &cq_sequence_tasks::catalog()).unwrap()
}

#[test]
fn retained_scans_never_use_join_filters_before_the_covering_build() {
    use std::collections::BTreeSet;

    struct Bindings {
        filters: BTreeSet<String>,
        declared: BTreeSet<String>,
    }
    impl VisitMut for Bindings {
        fn visit_local_mut(&mut self, local: &mut syn::Local) {
            // Rust evaluates an initializer before introducing its new binding.
            if let Some(initializer) = &mut local.init {
                self.visit_expr_mut(&mut initializer.expr);
            }
            if let syn::Pat::Ident(binding) = &local.pat {
                self.declared.insert(binding.ident.to_string());
            }
        }

        fn visit_expr_path_mut(&mut self, path: &mut syn::ExprPath) {
            if let Some(ident) = path.path.get_ident() {
                let name = ident.to_string();
                assert!(
                    !self.filters.contains(&name) || self.declared.contains(&name),
                    "join filter used before publication: {name}",
                );
            }
            visit_mut::visit_expr_path_mut(self, path);
        }

        fn visit_block_mut(&mut self, block: &mut syn::Block) {
            let outer = self.declared.clone();
            visit_mut::visit_block_mut(self, block);
            self.declared = outer;
        }
    }

    // The second form is an eager pipeline store, not the borrowed base-store
    // fast path. Both precede the covering provider in Sequence order.
    for input in [quote::quote!(rows l), quote::quote!(map l rows l)] {
        let program = late_covering_program(input);
        let facts = check_grading(&program, &machine()).unwrap();
        assert!(!facts.structures.join_filters["l"].is_empty());
        let context = cq_print::Context::new(&program.structures, &facts.structures);
        let filters = facts
            .structures
            .join_filters
            .iter()
            .flat_map(|(scan, filters)| {
                let context = &context;
                filters.iter().map(move |filter| {
                    context
                        .join_filter_ident(
                            scan,
                            &filter.covering.to_string(),
                            &filter.key.columns.first().unwrap().to_string(),
                        )
                        .to_string()
                })
            })
            .collect();
        let mut expression = translate(&program, &facts).unwrap();
        Bindings {
            filters,
            declared: BTreeSet::new(),
        }
        .visit_expr_mut(&mut expression);
    }
}

#[test]
fn covering_filters_reject_families_without_routed_key_enumeration() {
    let mut source = late_covering_program(quote::quote!(rows l));
    source.graded = None;
    cq_lang_sequence::contract::check(&source).unwrap();
    let families: [syn::Type; 3] = [
        syn::parse_quote!(::std::collections::BTreeMap<_, ::std::vec::Vec<_>>),
        syn::parse_quote!(::cq_storage::HashSet<_>),
        syn::parse_quote!(::cq_storage::AdaptiveKeySet<_, ::cq_storage::BorrowedI64TupleCodec, 64>),
    ];
    for ty in families {
        for routed in [false, true] {
            let mut invalid = source.clone();
            for entry in &mut invalid.structures.structures.entries {
                if entry.name == "b"
                    && let cq_lang_sequence::structures::Structure::Built { keyed, .. } =
                        &mut entry.structure
                {
                    keyed.ty = if routed {
                        syn::parse_quote!(::cq_storage::sharded::Sharded<#ty>)
                    } else {
                        ty.clone()
                    };
                }
            }
            let error = cq_lang_sequence::contract::check(&invalid).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("join filter requires an exact covering"),
                "{error}"
            );
            assert!(
                cq_sequence_plan::decompose(&invalid, &machine(), &cq_sequence_tasks::catalog())
                    .is_err()
            );
        }
    }
}

#[test]
fn tiled_folds_count_rows_within_each_tile_not_joined_tile_iterators() {
    let program = program(
        quote::quote! {
            input = store rows r;
            result = store fold count tile 2 input;
        },
        quote::quote!(input at host; result at pool;),
    );
    let facts = check_grading(&program, &machine()).unwrap();
    assert_eq!(
        facts.tilings["result"],
        vec![cq_lang_sequence::Tiling::Distribute]
    );
    let tasks = translate(&program, &facts).unwrap();
    run_partition_fixture(quote::quote! {
        fn main() {
            for R in [&[][..], &[(1i32,)][..], &[(1,), (2,), (3,), (4,), (5,)][..]] {
                let actual: Vec<usize> = (#tasks).collect();
                let expected: Vec<usize> = R.chunks(2).map(<[_]>::len).collect();
                assert_eq!(actual, expected);
            }
        }
    });
}

fn constant_key_provider_program() -> Program {
    let physical = syn::parse_quote! {
        physical {
            relation L(a:i32,b:i32); relation R(a:i32,b:i32,c:i32);
            consumer canonical;
            l = pipe rename L@0 {a -> a,b -> b};
            raw = pipe rename R@1 {a -> a,b -> b,c -> c};
            r = let point select raw if (a = 7);
            joined = pipe probe first [a,b] l,r;
            return joined as answer(a,b,c);
        }
    };
    let structures = cq_structure_selection::compile(&physical).unwrap();
    let source = syn::parse_quote! {
        #structures
        sequence {
            provider = store map r rows raw;
            input = rows l;
            result = store flatten lookup r [a,b] input;
        }
    };
    cq_sequence_tasks::select_providers(&source, &cq_sequence_tasks::catalog()).unwrap()
}

#[test]
fn planner_does_not_route_on_constant_bound_provider_keys() {
    let source = constant_key_provider_program();
    let facts = cq_lang_sequence::contract::check(&source).unwrap();
    assert!(
        facts.structures.filtered["r"]
            .constants
            .iter()
            .any(|binding| binding.column == "a")
    );
    let planned =
        cq_sequence_plan::decompose(&source, &machine(), &cq_sequence_tasks::catalog()).unwrap();
    assert!(!planned.trace.iter().any(|advice| {
        matches!(advice, cq_sequence_plan::Advice::KeyPartition(partition)
            if partition.name == "provider")
    }));
    assert!(
        cq_sequence_plan::decompose_with_key_partitions(
            &source,
            &machine(),
            &cq_sequence_tasks::catalog(),
            &[cq_sequence_plan::KeyPartitionPolicy {
                name: syn::parse_quote!(provider),
                select: true,
            }],
        )
        .is_err()
    );
}

#[test]
fn routed_provider_projects_the_stored_key_after_constant_elision() {
    let mut program = constant_key_provider_program();
    for entry in &mut program.structures.structures.entries {
        if entry.name == "r"
            && let cq_lang_sequence::structures::Structure::Built { keyed, .. } =
                &mut entry.structure
        {
            keyed.ty = syn::parse_quote!(
                ::cq_storage::sharded::Sharded<::cq_storage::sharded::Index<_, _>, _>
            );
        }
    }
    program.sequence = syn::parse_quote! {
        sequence {
            provider_input = store map r rows raw;
            provider = store partition_k[b](64) provider_input;
            input = rows l;
            result = store flatten lookup r [a,b] input;
        }
    };
    program.graded = Some(syn::parse_quote! {
        graded tasks {
            provider_input at host; provider at pool; input at host; result at host;
        }
    });
    let facts = check_grading(&program, &machine()).unwrap();
    let expression = translate(&program, &facts).unwrap();
    run_partition_fixture(quote::quote! {
        fn main() {
            let probes = [(7, 2), (8, 2), (7, 3), (7, 9)];
            let values = [(7, 2, 10), (8, 2, 20), (7, 2, 11), (7, 3, 30)];
            for L in [&probes[..], &[][..]] {
                for R in [&values[..], &[][..]] {
                    let actual: Vec<_> = (#expression).map(|(a,b,c)| (*a,*b,*c)).collect();
                    let expected: Vec<_> = L.iter().flat_map(|&(a,b)| {
                        R.iter().filter_map(move |&(ra,rb,c)| {
                            (ra == 7 && ra == a && rb == b).then_some((a,b,c))
                        })
                    }).collect();
                    assert_eq!(actual, expected);
                }
            }
        }
    });
}

#[test]
fn nested_tiles_keep_canonicalization_at_the_innermost_rows() {
    let program = program(
        quote::quote! {
            input = store rows r;
            result = store canon tile 2 tile 4 input;
        },
        quote::quote!(input at host; result at pool;),
    );
    let facts = check_grading(&program, &machine()).unwrap();
    assert!(facts.tilings["result"].contains(&cq_lang_sequence::Tiling::Distribute));
    let tasks = translate(&program, &facts).unwrap();
    let mut sequential = program.clone();
    sequential.graded = Some(syn::parse_quote! {
        graded tasks { input at host; result at host; }
    });
    let sequential_facts = check_grading(&sequential, &machine()).unwrap();
    let pull = cq_sequence_pull::translate(&sequential, &sequential_facts).unwrap();
    run_partition_fixture(quote::quote! {
        fn main() {
            for R in [&[][..], &[(1i32,)][..], &[(1,), (1,), (2,), (2,), (1,)][..]] {
                let actual: Vec<_> = (#tasks).map(|(a,)| *a).collect();
                let expected: Vec<_> = (#pull).map(|(a,)| *a).collect();
                assert_eq!(actual, expected);
            }
        }
    });
}

#[test]
fn literal_filtered_host_tiles_retain_the_source_predicate() {
    let physical = syn::parse_quote! {
        physical {
            relation R(tag:i32,a:i32);
            consumer canonical;
            r = let rename R@0 {a -> a} constants {tag = 7};
            return r as answer(a);
        }
    };
    let structures = cq_structure_selection::compile(&physical).unwrap();
    let program = syn::parse_quote! {
        #structures
        sequence {
            input = store rows r;
            result = store tile 2 input;
        }
        graded tasks { input at host; result at pool; }
    };
    let facts = check_grading(&program, &machine()).unwrap();
    let expression = translate(&program, &facts).unwrap();
    assert!(!worker_bodies(&expression).is_empty());
    run_partition_fixture(quote::quote! {
        fn main() {
            let R = &[(7i32, 1i32), (8, 2), (7, 3), (9, 4), (7, 5)][..];
            let actual: Vec<_> = (#expression).map(|(a,)| *a).collect();
            assert_eq!(actual, [1, 3, 5]);
        }
    });
}

#[test]
fn flattened_key_covers_restore_positional_reads() {
    let physical = syn::parse_quote! {
        physical {
            relation R(a:i64);
            r = pipe rename R@0 {a -> a};
            selected = pipe range select r if (a > 1);
            out = pipe canon project selected keep {a};
            return out as answer(a);
        }
    };
    let structures =
        cq_structure_selection::compile_with(&physical, quote::quote!(positional(R))).unwrap();
    let program = syn::parse_quote! {
        #structures
        sequence {
            input = store rows r;
            parts = store partition_k[a](64) input;
            joined = store parts;
            result = store canon map out map selected flatten joined;
        }
        graded tasks { input at host; parts at pool; joined at host; result at host; }
    };
    let facts = check_grading(&program, &machine()).unwrap();
    assert!(facts.structures.reads.contains_key("selected"));
    let mut expression = translate(&program, &facts).unwrap();
    struct Reads(usize);
    impl VisitMut for Reads {
        fn visit_expr_path_mut(&mut self, path: &mut syn::ExprPath) {
            if path
                .path
                .segments
                .iter()
                .any(|segment| segment.ident == "BlockRead")
            {
                self.0 += 1;
            }
            visit_mut::visit_expr_path_mut(self, path);
        }
    }
    let mut reads = Reads(0);
    reads.visit_expr_mut(&mut expression);
    assert!(
        reads.0 > 0,
        "completed key cover still suppresses positional reads"
    );
    run_partition_fixture(quote::quote! {
        fn main() {
            let R = &[(0i64,), (2,), (2,), (3,), (1,)][..];
            let mut actual: Vec<_> = (#expression).map(|(a,)| *a).collect();
            actual.sort();
            assert_eq!(actual, [2, 3]);
        }
    });
}

#[test]
fn filtered_host_tiles_apply_predicates_without_collecting_the_host() {
    let mut program = late_covering_program(quote::quote!(rows l));
    program.sequence = syn::parse_quote! {
        sequence {
            b = store map b map s rows r;
            parts = store map out flatten lookup b [x] flatten tile 64 rows l;
            joined = store parts;
            result = store canon joined;
        }
    };
    program.graded = Some(syn::parse_quote! {
        graded tasks { b at host; parts at pool; joined at host; result at host; }
    });
    let facts = check_grading(&program, &machine()).unwrap();
    assert!(!facts.structures.join_filters["l"].is_empty());
    let expression = translate(&program, &facts).unwrap();
    let rendered = expression.to_token_stream().to_string();
    assert!(rendered.contains("position_range"), "{rendered}");
    assert!(rendered.contains("ReadPredicate :: test"), "{rendered}");
    assert!(!rendered.contains("let __cq_driver = ("), "{rendered}");
    run_partition_fixture(quote::quote! {
        fn main() {
            let left: Vec<_> = (0..257i64).map(|i| (i % 17, i)).collect();
            let right = [(2i64, 4i64), (5, 3), (9, 0), (11, 8)];
            for threads in [1, 24] {
                rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap().install(|| {
                    for length in [0, 1, 63, 64, 65, 257] {
                        let L = &left[..length];
                        let R = &right[..];
                        let expected: ::std::collections::BTreeSet<_> = L.iter()
                            .flat_map(|&(x, y)| R.iter()
                                .filter(move |&&(key, z)| key == x && z > 1)
                                .map(move |&(_, z)| (x, y, z)))
                            .collect();
                        let actual: ::std::collections::BTreeSet<_> =
                            (#expression).map(|(x, y, z)| (*x, *y, *z)).collect();
                        assert_eq!(actual, expected);
                    }
                });
            }
        }
    });
}

#[test]
fn borrowed_host_key_cover_uses_host_tiles_without_input_collection() {
    let program = program(
        quote::quote! {
            input = store rows r;
            parts = store canon partition_k[a](64) input;
            joined = store parts;
            result = flatten joined;
        },
        quote::quote! {
            input at host; parts at pool; joined at host; result at host;
        },
    );
    let facts = check_grading(&program, &machine()).unwrap();
    assert!(matches!(
        facts.publications["input"],
        cq_lang_sequence::Publication::BorrowHost { .. }
    ));
    let expression = translate(&program, &facts).unwrap();
    let rendered = expression.to_token_stream().to_string();
    assert!(rendered.contains("__cq_route_source"), "{rendered}");
    assert!(
        !rendered.contains("let __cq_key_source_input"),
        "{rendered}"
    );
    run_partition_fixture(quote::quote! {
        fn main() {
            let rows: Vec<_> = (0..4097i32).map(|i| (i % 131,)).collect();
            for threads in [1, 24] {
                rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap().install(|| {
                    for length in [0, 1, 63, 64, 65, 4097] {
                        let R = &rows[..length];
                        let actual: ::std::collections::BTreeSet<_> =
                            (#expression).map(|(a,)| *a).collect();
                        let expected: ::std::collections::BTreeSet<_> =
                            R.iter().map(|&(a,)| a).collect();
                        assert_eq!(actual, expected);
                    }
                });
            }
        }
    });
}
