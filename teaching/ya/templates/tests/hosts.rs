//! Change host construction by a macro alias while retaining the same stages.
mod borrowed {
    pub use ::mini_linq::{
        collect_access, collect_pipeline, graph_query as query,
        materialized_query as index_requirements, result_indexes as relational_plan,
    };
}

#[test]
fn one_compiled_query_accepts_separate_borrowed_input_bindings() {
    use borrowed as mini_linq;
    let query = mini_linq::query! {
        struct Pair;
        relation R(a:i32,b:i32); relation S(b:i32,c:i32);
        answer(a,c) :- R(a,b), S(b,c);
    };
    let full = [(1, 2), (3, 4)];
    let delta = [(5, 2)];
    let right = [(2, 6), (4, 7)];
    assert_eq!(
        query(full.iter().cloned(), right.iter().cloned()),
        [(1, 6), (3, 7)].into_iter().collect()
    );
    assert_eq!(
        query(delta.iter().cloned(), right.iter().cloned()),
        [(5, 6)].into_iter().collect()
    );
    assert_eq!(right, [(2, 6), (4, 7)]);
}
