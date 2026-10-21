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
        quote::quote!(input = store rows r; result = store fold count input;),
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
        quote::quote!(input = store rows r; result = store tile 2 flatten tile 2 input;),
        quote::quote!(input at host; result at host;),
    );
    // The outer host tile enters pool; pool flatten remains a collective.
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
