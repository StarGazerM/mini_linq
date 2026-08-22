#![allow(clippy::clone_on_copy)] // Preserve the shape required by non-Copy schemas.

use std::cell::Cell;

use mini_linq::{iterator_pipeline, pull};

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
        pull! {
            triangle(x, y, z) => (((*x).clone(), (*y).clone(), (*z).clone())) :-
            R(x, y) => for (x, y) in (self.rows0.iter()),
            S(y, z) => for (z,) in (
                self.index0
                    .get(&((*y).clone(),))
                    .into_iter()
                    .flatten()
            ),
            T(z, x) => if (self.index1.contains(&((*z).clone(), (*x).clone()))).
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
        pull! {
            project(x) => (((*x).clone(),)) :-
            R(x, _y) => for (x, _y) in (self.rows0.iter()).
        }
    }
}

struct LazyResultSet {
    rows0: Vec<(i32,)>,
    rows_requested: Cell<usize>,
}

impl LazyResultSet {
    fn query(&self) -> impl Iterator<Item = (i32,)> + '_ {
        pull! {
            answer(x) => (((*x).clone(),)) :-
            R(x) => for (x,) in (
                self.rows0.iter().inspect(|_| {
                    self.rows_requested.set(self.rows_requested.get() + 1);
                })
            ).
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

    let mut rows = pull! {
        answer(x) => ((x,)) :-
        R(x) => for (x,) in ({
            counter.set(counter.get() + 1);
            [(1,), (2,)].into_iter()
        }).
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

    let result = pull! {
        answer(x, y) => ((x, y,)) :-
        Seed(x) => for (x,) in (rows.into_iter()),
        Edge(x, y) => for (y,) in (
            iter0.get(&(x,)).cloned().unwrap_or_default().into_iter()
        ).
    }
    .collect::<Vec<_>>();

    assert_eq!(result, [(1, 10), (1, 11), (2, 20)]);
}

#[test]
fn iterator_pipeline_is_an_independently_invocable_lazy_stage() {
    let evaluations = Cell::new(0);
    let counter = &evaluations;

    let mut rows = iterator_pipeline! {
        iter0 = scan (x,) in ({
            counter.set(counter.get() + 1);
            [(1,), (2,), (3,)].into_iter()
        }) yield (x,);
        iter1 = filter iter0 as (x,) if (x != 2);
        iter2 = project iter1 as (x,) yield ((x,));
        iter3 = distinct iter2;
        return iter3.
    };

    assert_eq!(evaluations.get(), 0);
    assert_eq!(rows.next(), Some((1,)));
    assert_eq!(evaluations.get(), 1);
    assert_eq!(rows.collect::<Vec<_>>(), vec![(3,)]);
    assert_eq!(evaluations.get(), 1);
}
