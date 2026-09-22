#![allow(clippy::clone_on_copy)] // Preserve the shape required by non-Copy schemas.

#[allow(unused_imports)]
mod execution {
    pub use ::mini_linq::*;
    pub use cq_example_providers::{hash_distinct as distinct_state, hash_set_sink as set_sink};
}
use execution as mini_linq;

use std::cell::Cell;

use mini_linq::{access_to_distinct_iter, pipeline_to_distinct_iter};

pub struct AnnotatedTriangle;

pub struct AnnotatedTriangleStorage {
    rows0: Vec<(i32, i32)>,
    index0: std::collections::HashMap<(i32,), Vec<(i32,)>>,
    index1: std::collections::HashSet<(i32, i32)>,
}

impl AnnotatedTriangle {
    pub fn build(
        input0: impl IntoIterator<Item = (i32, i32)>,
        input1: impl IntoIterator<Item = (i32, i32)>,
        input2: impl IntoIterator<Item = (i32, i32)>,
    ) -> AnnotatedTriangleStorage {
        let relation0: std::collections::HashSet<(i32, i32)> = input0.into_iter().collect();
        let relation1: std::collections::HashSet<(i32, i32)> = input1.into_iter().collect();
        let relation2: std::collections::HashSet<(i32, i32)> = input2.into_iter().collect();

        let rows0 = relation0.into_iter().collect();
        let mut index0: std::collections::HashMap<(i32,), Vec<(i32,)>> =
            std::collections::HashMap::new();
        for (y, z) in relation1 {
            index0.entry((y,)).or_default().push((z,));
        }
        let index1: std::collections::HashSet<(i32, i32)> = relation2;

        AnnotatedTriangleStorage {
            rows0,
            index0,
            index1,
        }
    }

    pub fn run(
        input0: impl IntoIterator<Item = (i32, i32)>,
        input1: impl IntoIterator<Item = (i32, i32)>,
        input2: impl IntoIterator<Item = (i32, i32)>,
    ) -> Vec<(i32, i32, i32)> {
        Self::build(input0, input1, input2).materialize()
    }
}

impl AnnotatedTriangleStorage {
    pub fn query(&self) -> impl Iterator<Item = (i32, i32, i32)> + '_ {
        access_to_distinct_iter! {
            triangle(x, y, z) => (((*x).clone(), (*y).clone(), (*z).clone())) :-
            R(x, y) => bind (x, y) <- (self.rows0.iter()),
            S(y, z) => bind (z,) <- (
                self.index0
                    .get(&((*y).clone(),))
                    .into_iter()
                    .flatten()
            ),
            T(z, x) => if (self.index1.contains(&((*z).clone(), (*x).clone())));
        }
    }

    pub fn materialize(&self) -> Vec<(i32, i32, i32)> {
        let mut result = self.query().collect::<Vec<_>>();
        result.sort_unstable();
        result
    }
}

struct ProjectionSet {
    rows0: Vec<(i32, i32)>,
}

impl ProjectionSet {
    fn query(&self) -> impl Iterator<Item = (i32,)> + '_ {
        access_to_distinct_iter! {
            project(x) => (((*x).clone(),)) :-
            R(x, _y) => bind (x, _y) <- (self.rows0.iter());
        }
    }
}

struct LazyResultSet {
    rows0: Vec<(i32,)>,
    rows_requested: Cell<usize>,
}

impl LazyResultSet {
    fn query(&self) -> impl Iterator<Item = (i32,)> + '_ {
        access_to_distinct_iter! {
            answer(x) => (((*x).clone(),)) :-
            R(x) => bind (x,) <- (
                self.rows0.iter().inspect(|_| {
                    self.rows_requested.set(self.rows_requested.get() + 1);
                })
            );
        }
    }
}

#[test]
fn built_triangle_storage_exposes_lazy_and_materialized_results() {
    let storage =
        AnnotatedTriangle::build([(1, 2), (1, 2), (2, 3)], [(2, 3), (3, 4)], [(3, 1), (4, 2)]);
    let expected = vec![(1, 2, 3), (2, 3, 4)];

    assert_eq!(storage.query().collect::<Vec<_>>().len(), 2);
    assert_eq!(storage.materialize(), expected);
    assert_eq!(
        AnnotatedTriangle::run([(1, 2)], [(2, 3)], [(3, 1)]),
        vec![(1, 2, 3)]
    );
}

#[test]
fn public_query_applies_set_semantics_lazily() {
    let storage = ProjectionSet {
        rows0: vec![(1, 2), (1, 3), (2, 4)],
    };
    assert_eq!(storage.query().collect::<Vec<_>>(), vec![(1,), (2,)]);
}

#[test]
fn pull_is_one_resumable_iterator_for_the_entire_result_set() {
    let storage = LazyResultSet {
        rows0: vec![(1,), (2,), (3,)],
        rows_requested: Cell::new(0),
    };

    let mut cursor = storage.query();
    assert_eq!(storage.rows_requested.get(), 0);
    assert_eq!(cursor.next(), Some((1,)));
    assert_eq!(storage.rows_requested.get(), 1);
    drop(cursor);

    assert_eq!(storage.query().collect::<Vec<_>>(), vec![(1,), (2,), (3,)]);
}

#[test]
fn pull_defers_the_root_source_expression_until_first_next() {
    let evaluations = Cell::new(0);
    let counter = &evaluations;

    let mut rows = access_to_distinct_iter! {
        answer(x) => ((x,)) :-
        R(x) => bind (x,) <- ({
            counter.set(counter.get() + 1);
            [(1,), (2,)].into_iter()
        });
    };

    assert_eq!(evaluations.get(), 0);
    assert_eq!(rows.next(), Some((1,)));
    assert_eq!(evaluations.get(), 1);
    assert_eq!(rows.next(), Some((2,)));
    assert_eq!(evaluations.get(), 1);
    assert_eq!(rows.next(), None);
    assert_eq!(evaluations.get(), 1);
}

#[test]
fn generated_iter0_does_not_capture_a_user_leaf_named_iter0() {
    let rows = [(1,), (2,)];
    let index = std::collections::HashMap::from([((1,), vec![(10,), (11,)]), ((2,), vec![(20,)])]);
    let iter0 = &index;

    let result = access_to_distinct_iter! {
        answer(x, y) => ((x, y,)) :-
        Seed(x) => bind (x,) <- (rows.into_iter()),
        Edge(x, y) => bind (y,) <- (
            iter0.get(&(x,)).cloned().unwrap_or_default().into_iter()
        );
    }
    .collect::<Vec<_>>();

    assert_eq!(result, [(1, 10), (1, 11), (2, 20)]);
}

#[test]
fn iterator_pipeline_is_an_independently_invocable_lazy_stage() {
    let evaluations = Cell::new(0);
    let counter = &evaluations;

    let mut rows = pipeline_to_distinct_iter! {
        iter0 = scan (x,) in ({
            counter.set(counter.get() + 1);
            [(1,), (2,), (3,)].into_iter()
        }) yield (x,);
        iter1 = filter iter0 as (x,) if (x != 2);
        iter2 = project iter1 as (x,) yield ((x,));
        iter3 = distinct iter2;
        return iter3;
    };

    assert_eq!(evaluations.get(), 0);
    assert_eq!(rows.next(), Some((1,)));
    assert_eq!(evaluations.get(), 1);
    assert_eq!(rows.collect::<Vec<_>>(), vec![(3,)]);
    assert_eq!(evaluations.get(), 1);
}

#[test]
fn quoted_let_accesses_preserve_bindings_and_empty_binding_effects_on_demand() {
    let evaluations = Cell::new((0, 0));
    let counter = &evaluations;
    let compute = |x| {
        let (bindings, effects) = counter.get();
        counter.set((bindings + 1, effects));
        x + 10
    };
    let record = || {
        let (bindings, effects) = counter.get();
        counter.set((bindings, effects + 1));
    };
    let mut rows = access_to_distinct_iter! {
        answer(x, y) => ((x, y)) :-
            R(x) => bind (x,) <- ([(1,), (2,), (1,)]),
            let y = compute(x) => bind (y,) <- (::std::iter::once((compute(x),))),
            let () = record() => bind () <- ({ record(); ::std::iter::once(()) }),
            if y > 11 => if (y > 11);
    };

    assert_eq!(evaluations.get(), (0, 0));
    assert_eq!(rows.next(), Some((2, 12)));
    assert_eq!(evaluations.get(), (2, 2));
    assert_eq!(rows.next(), None);
    assert_eq!(evaluations.get(), (3, 3));
}

#[test]
fn dependent_bind_sources_are_demanded_in_order_and_before_item_binding() {
    let evaluations = Cell::new((0, 0));
    let observed = &evaluations;
    // This outer Rust name is deliberately also the first logical binder.
    // Its iterable expression is evaluated before that binder is introduced.
    let a = [1, 2, 3];
    let mut rows = access_to_distinct_iter! {
        answer(a, b) => ((a, b)) :-
        R(a) => bind a <- ({
            let (_, lookups) = observed.get();
            observed.set((1, lookups));
            a.into_iter()
        }),
        S(a, b) => bind b <- ({
            let (sources, lookups) = observed.get();
            observed.set((sources, lookups + 1));
            [a * 10, a * 10 + 1]
        });
    };
    assert_eq!(evaluations.get(), (0, 0));
    assert_eq!(rows.next(), Some((1, 10)));
    assert_eq!(evaluations.get(), (1, 1));
    assert_eq!(rows.next(), Some((1, 11)));
    assert_eq!(evaluations.get(), (1, 1));
    assert_eq!(rows.next(), Some((2, 20)));
    assert_eq!(evaluations.get(), (1, 2));
    drop(rows);
    assert_eq!(evaluations.get(), (1, 2));
}
