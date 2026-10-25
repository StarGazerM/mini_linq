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
        } structures { r host scan = Reader []; r canon = ::std::collections::HashSet<_>; }
        sequence { #sequence }
        graded tasks { #grades }
    })
    .unwrap()
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
    assert!(rendered.contains("par_chunks (64usize)"), "{rendered}");
    assert!(!worker_bodies(&expression).is_empty());
    assert!(rendered.contains("collect :: < :: std :: vec :: Vec < _ > >"), "{rendered}");
}

// Inspect the closure passed to the indexed parallel iterator, not merely the
// presence of Rayon: parallel copying of an already-evaluated segment is wrong.
fn worker_bodies(expression: &syn::Expr) -> Vec<syn::Expr> {
    #[derive(Default)]
    struct Workers(Vec<syn::Expr>);

    impl VisitMut for Workers {
        fn visit_expr_method_call_mut(&mut self, call: &mut syn::ExprMethodCall) {
            if call.method == "map"
                && let syn::Expr::MethodCall(source) = call.receiver.as_ref()
                && source.method == "par_chunks"
                && let Some(syn::Expr::Closure(worker)) = call.args.first()
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
    assert_eq!(analysis.tilings["parts"], vec![cq_lang_sequence::Tiling::Distribute]);
    let expression = translate(&program, &analysis).unwrap();
    let workers = worker_bodies(&expression);
    assert_eq!(workers.len(), 1, "{}", expression.to_token_stream());
    let worker = workers[0].to_token_stream().to_string();
    for adapter in [
        "Iterator :: map",
        "Iterator :: filter",
        "Iterator :: flat_map",
        "collect :: < :: std :: vec :: Vec < _ > >",
    ] {
        assert!(worker.contains(adapter), "missing {adapter} inside worker: {worker}");
    }
    assert!(
        worker.contains("IndexRead :: lookup") || worker.contains("RangeCursor :: range"),
        "lookup must execute inside the worker: {worker}"
    );
    // This total scalar expression belongs only to Map, so a parallel lookup
    // followed by a serial map cannot satisfy the worker-local assertion.
    assert!(worker.contains("100"), "mapped scalar escaped the worker: {worker}");
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
        #![allow(non_snake_case, unused_variables)]
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
    let runtime = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../runtime/cq-storage/src/lib.rs");
    cq_testkit::rustc::Runtime::new(&runtime, false).run(&source.to_string());
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
        assert_eq!(facts.tilings["parts"], vec![cq_lang_sequence::Tiling::Distribute]);
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
    let runtime = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../runtime/cq-storage/src/lib.rs");
    cq_testkit::rustc::Runtime::new(&runtime, false).run(&source.to_string());
}

fn run_partition_fixture(source: proc_macro2::TokenStream) {
    let runtime = cq_testkit::rustc::Runtime::with_dependencies(vec![
        cq_testkit::rustc::linked_dependency::<cq_sequence_tasks::Config>(
            "cq_sequence_tasks", "cq_sequence_tasks::Config",
        ),
        cq_testkit::rustc::linked_dependency::<cq_storage::sharded::FullKey>(
            "cq_storage", "cq_storage::sharded::FullKey",
        ),
    ]);
    // Match the consumer's runtime namespace while linking the real task and
    // storage implementations, not a fixture-specific substitute.
    let source = quote::quote! {
        #![allow(non_snake_case, unused_variables)]
        extern crate self as mini_linq;
        pub mod __tasks { pub use cq_sequence_tasks::combine; }
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
            assert!(rendered.contains("combine :: par_group_global_with"), "{rendered}");
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
    assert_eq!(facts.shapes["partials"], vec![cq_lang_sequence::Len::Lit(1)]);
    assert_eq!(facts.tilings["partials"], vec![cq_lang_sequence::Tiling::Distribute]);
    let expression = translate(&program, &facts).unwrap();
    let rendered = expression.to_token_stream().to_string();
    assert!(rendered.contains("combine :: par_group_global_with"), "{rendered}");
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
    for routing in [quote::quote!(b), quote::quote!(b,a)] {
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
    let target = cq_sequence_tasks::select_providers(&source, &cq_sequence_tasks::catalog()).unwrap();
    assert!(target.sequence.definitions.iter().any(|definition|
        definition.name == "r" && matches!(definition.term, cq_lang_sequence::Term::Store { .. })));
    let native: syn::Type = syn::parse_quote!(::cq_storage::SortedRun<_, _, _>);
    assert!(target.structures.structures.entries.iter().any(|entry|
        entry.name == "s" && matches!(&entry.structure,
            cq_lang_sequence::structures::Structure::Built { keyed, .. } if keyed.ty == native)));
    assert!(target.structures.structures.entries.iter().any(|entry|
        entry.name == "s" && matches!(&entry.structure,
            cq_lang_sequence::structures::Structure::Rows { ty, .. }
                if *ty == syn::parse_quote!(::std::vec::Vec<_>))));
    cq_lang_sequence::check_rewrite(&source, &target).unwrap();
    let facts = check_grading(&target, &machine()).unwrap();
    translate(&target, &facts).unwrap();
}
