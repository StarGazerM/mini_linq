use cq_lang_sequence::{Program, check_grading};
use cq_sequence_tasks::{machine, translate};
use quote::ToTokens;

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
        quote::quote!(input = store rows r; result = store tile 3 input;),
        quote::quote!(input at host; result at pool;),
    );
    let analysis = check_grading(&program, &machine()).unwrap();
    let expression = translate(&program, &analysis)
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(expression.contains("par_chunks (3usize)"), "{expression}");
    assert!(expression.contains("into_par_iter"), "{expression}");
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
    static DEPENDENCIES: std::sync::LazyLock<Vec<(String, std::path::PathBuf)>> =
        std::sync::LazyLock::new(|| {
            vec![
                cq_testkit::rustc::linked_dependency::<cq_storage::SortedRun<i32, i32>>(
                    "cq_storage",
                    "cq_storage::SortedRun<i32, i32>",
                ),
                cq_testkit::rustc::linked_dependency::<rayon::ThreadPool>(
                    "rayon",
                    "rayon::ThreadPool",
                ),
            ]
        });
    cq_testkit::rustc::Runtime::with_dependencies(DEPENDENCIES.clone()).run(&source.to_string());
}
