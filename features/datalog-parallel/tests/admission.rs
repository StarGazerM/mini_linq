//! Same-round duplicate candidates are admitted once by the canonical layout,
//! whether it is sharded or a sorted run, with parallel (threshold 0) or inline
//! (large threshold) admission.
use std::collections::BTreeSet;

macro_rules! one_hole {
    ($threshold:literal; $($storage:tt)*) => {{
        let mut rounds = Vec::new();
        // Seed and step candidates each repeat within one hole's round.
        let edges = [(1, 5), (2, 5), (5, 7), (5, 7), (6, 7), (5, 8), (7, 8)];
        let seed = [(1,), (2,), (6,)];
        let rows: Vec<_> = cq_datalog::datalog! {
            pipeline {
                storage_rewrites = [$($storage)*];
                round = [cq_datalog_parallel::partition(threshold = $threshold)];
                print = cq_datalog_parallel::print_rounds;
            }
            observe = (|round, count| rounds.push((round, count)));
            struct OneHole;
            relation Edge(x:i32,y:i32) = (&edges);
            relation Seed(x:i32) = (&seed);
            relation Out(y:i32);
            output Out;
            Out(*y) :- Seed(x), Edge(x,y);
            Out(*z) :- Out(y), Edge(y,z);
        }
        .collect();
        (rows, rounds)
    }};
}

macro_rules! two_holes {
    ($threshold:literal; $($storage:tt)*) => {{
        let mut rounds = Vec::new();
        let left = [(1,), (2,), (3,)];
        let right = [(2,), (3,), (4,)];
        let rows: Vec<_> = cq_datalog::datalog! {
            pipeline {
                storage_rewrites = [$($storage)*];
                round = [cq_datalog_parallel::partition(threshold = $threshold)];
                print = cq_datalog_parallel::print_rounds;
            }
            observe = (|round, count| rounds.push((round, count)));
            struct TwoHoles;
            relation Left(x:i32) = (&left);
            relation Right(x:i32) = (&right);
            relation Both(x:i32);
            output Both;
            Both(*x) :- Left(x);
            Both(*x) :- Right(x);
        }
        .collect();
        (rows, rounds)
    }};
}

macro_rules! configurations {
    ($check:ident) => {
        $check!(0; cq_datalog_parallel::shard_storage);
        $check!(1000000; cq_datalog_parallel::shard_storage);
        $check!(0; cq_datalog_parallel::shard_storage(layouts(sharded)));
        $check!(1000000; cq_datalog_parallel::shard_storage(layouts(sharded)));
        $check!(0; cq_datalog_parallel::shard_storage(layouts(runs)));
        $check!(1000000; cq_datalog_parallel::shard_storage(layouts(runs)));
    };
}

fn pools() -> impl Iterator<Item = rayon::ThreadPool> {
    [1, 4].into_iter().map(|threads| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
    })
}

#[test]
fn same_round_duplicates_from_one_hole_are_admitted_once() {
    for pool in pools() {
        pool.install(|| {
            macro_rules! check {
                ($threshold:literal; $($storage:tt)*) => {{
                    let (rows, rounds): (Vec<(i32,)>, Vec<(usize, usize)>) =
                        one_hole!($threshold; $($storage)*);
                    let configuration = stringify!($threshold; $($storage)*);
                    assert_eq!(rows.len(), 3, "{configuration}");
                    assert_eq!(
                        rows.into_iter().collect::<BTreeSet<_>>(),
                        BTreeSet::from([(5,), (7,), (8,)]),
                        "{configuration}"
                    );
                    // Seed candidates 5, 5, 7 admit {5, 7}; step candidates 7, 8, 8 admit {8}.
                    assert_eq!(rounds, [(0, 2), (1, 1)], "{configuration}");
                }};
            }
            configurations!(check);
        });
    }
}

#[test]
fn same_round_duplicates_from_two_holes_are_admitted_once() {
    for pool in pools() {
        pool.install(|| {
            macro_rules! check {
                ($threshold:literal; $($storage:tt)*) => {{
                    let (rows, rounds): (Vec<(i32,)>, Vec<(usize, usize)>) =
                        two_holes!($threshold; $($storage)*);
                    let configuration = stringify!($threshold; $($storage)*);
                    assert_eq!(rows.len(), 4, "{configuration}");
                    assert_eq!(
                        rows.into_iter().collect::<BTreeSet<_>>(),
                        BTreeSet::from([(1,), (2,), (3,), (4,)]),
                        "{configuration}"
                    );
                    // Six candidates from two seed holes publish one delta of four rows.
                    assert_eq!(rounds, [(0, 4)], "{configuration}");
                }};
            }
            configurations!(check);
        });
    }
}
