//! Parallel admission order is unspecified; all engines are compared as sets.
use std::collections::BTreeSet;

fn assert_ascent<T: Ord + std::fmt::Debug + Clone>(actual: &[T], expected: Vec<T>) {
    assert_eq!(
        actual.iter().cloned().collect::<BTreeSet<_>>(),
        expected.into_iter().collect()
    );
}

fn assert_same<T: Ord + Clone + std::fmt::Debug>(parallel: &[T], sequential: &[T]) {
    assert_eq!(
        parallel.iter().cloned().collect::<BTreeSet<_>>(),
        sequential.iter().cloned().collect()
    );
}

#[test]
fn self_join_matches_sequential_sets_and_ascent_in_local_pools() {
    for threads in [2, 4] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                // Disjoint short paths keep closure small while crossing the chunk boundary.
                for count in [0, 1, 7, 1100] {
                    let edges: Vec<_> = (0..count)
                        .flat_map(|i| {
                            [
                                (3 * i, 3 * i + 1),
                                (3 * i + 1, 3 * i + 2),
                                (3 * i, 3 * i + 1),
                            ]
                        })
                        .collect();
                    macro_rules! run {
                    ($($mode:tt)*) => {
                        cq_datalog::datalog! {
                            pipeline { $($mode)* }
                            struct Closure;
                            relation Edge(x:i32,y:i32) = (&edges);
                            relation Reach(x:i32,y:i32);
                            output Reach;
                            Reach(*x,*y) :- Edge(x,y);
                            Reach(*x,*z) :- Reach(x,y), Reach(y,z);
                        }.collect::<Vec<_>>()
                    };
                }
                    let sequential = run!();
                    let parallel = run!(storage_rewrites = [cq_datalog_parallel::shard_storage]; round = [cq_datalog_parallel::partition]; print = cq_datalog_parallel::print_rounds;);
                    assert_same(&parallel, &sequential);
                    ascent::ascent! {
                        struct Oracle;
                        relation edge(i32,i32);
                        relation reach(i32,i32);
                        reach(*x,*y) <-- edge(x,y);
                        reach(*x,*z) <-- reach(x,y), reach(y,z);
                    }
                    let mut oracle = Oracle {
                        edge: edges,
                        ..Default::default()
                    };
                    oracle.run();
                    assert_ascent(&parallel, oracle.reach);
                }
            });
    }
}

#[test]
fn later_delta_negation_generators_and_multiple_outputs_match_ascent() {
    rayon::ThreadPoolBuilder::new().num_threads(4).build().unwrap().install(|| {
        let edges: Vec<_> = (0..1100).flat_map(|i| [(3 * i, 3 * i + 1), (3 * i + 1, 3 * i + 2)]).collect();
        let seeds: Vec<_> = (0..1100).map(|i| (3 * i,)).collect();
        let blocked: Vec<_> = (0..1100).step_by(3).map(|i| (3 * i + 2,)).collect();
        let offset = 10;
        macro_rules! run {
            ($($mode:tt)*) => {{
                let (labels, reach, input) = cq_datalog::datalog! {
                    pipeline { $($mode)* }
                    struct Mixed;
                    relation Edge(x:i32,y:i32) = (&edges);
                    relation Seed(x:i32) = (&seeds);
                    relation Blocked(x:i32) = (&blocked);
                    relation Reach(x:i32);
                    relation Label(x:i32,y:i32);
                    output Label, Reach, Seed;
                    Reach(*x) :- Seed(x);
                    Reach(*y) :- Edge(x,y), Reach(x), !Blocked(y);
                    Label(*x,next + offset) :- Reach(x), for next in [*x, *x + 1, *x], if let Some(keep) = Some(*next), if *keep % 2 == 0;
                };
                (labels.collect::<Vec<_>>(), reach.collect::<Vec<_>>(), input.collect::<Vec<_>>())
            }};
        }
        let sequential = run!();
        let parallel = run!(storage_rewrites = [cq_datalog_parallel::shard_storage]; round = [cq_datalog_parallel::partition]; print = cq_datalog_parallel::print_rounds;);
        assert_same(&parallel.0, &sequential.0);
        assert_same(&parallel.1, &sequential.1);
        assert_same(&parallel.2, &sequential.2);
        ascent::ascent! {
            struct Oracle;
            relation edge(i32,i32);
            relation seed(i32);
            relation blocked(i32);
            relation reach(i32);
            relation label(i32,i32);
            reach(*x) <-- seed(x);
            reach(*y) <-- edge(x,y), reach(x), !blocked(y);
            label(*x,next + 10) <-- reach(x), for next in [*x, *x + 1, *x], if let Some(keep) = Some(next), if keep % 2 == 0;
        }
        let mut oracle = Oracle { edge: edges, seed: seeds, blocked, ..Default::default() };
        oracle.run();
        assert_ascent(&parallel.0, oracle.label);
        assert_ascent(&parallel.1, oracle.reach);
        assert_ascent(&parallel.2, oracle.seed);
    });
}

#[test]
fn resumed_stages_accept_the_extension_pipeline() {
    rayon::ThreadPoolBuilder::new()
        .num_threads(2)
        .build()
        .unwrap()
        .install(|| {
            let edges: Vec<_> = (0..1100).map(|x| (x, x + 1)).collect();
            let sequential = cq_datalog::datalog! {
                struct Copy;
                relation Edge(x:i32,y:i32) = (&edges);
                relation Out(x:i32,y:i32);
                output Out;
                Out(*x,*y) :- Edge(x,y);
            }
            .collect::<Vec<_>>();
            let stratified = cq_datalog::passes::incremental! {
                pipeline { cq_datalog::passes::storage, cq_datalog_parallel::shard_storage, cq_datalog::passes::fork, cq_datalog_parallel::partition, cq_datalog_parallel::print_rounds };
                struct Copy;
                relation Edge(x:i32,y:i32) = (&edges);
                relation Out(x:i32,y:i32);
                output Out;
                Out(*x,*y) :- Edge(x,y);
                strata { component [Out]; }
            }
            .collect::<Vec<_>>();
            let seminaive = cq_datalog::passes::storage! {
                pipeline { cq_datalog_parallel::shard_storage, cq_datalog::passes::fork, cq_datalog_parallel::partition, cq_datalog_parallel::print_rounds };
                struct Copy;
                relation Edge(x:i32,y:i32) = (&edges);
                relation Out(x:i32,y:i32);
                output Out;
                Out(*x,*y) :- Edge(x,y);
                strata { component [Out]; }
                incremental {
                    scheme seminaive_covering;
                    delta Out = Recent;
                    variant 0 seed { Out(*x,*y) :- Edge(x,y); }
                }
            }
            .collect::<Vec<_>>();
            assert_same(&stratified, &sequential);
            assert_same(&seminaive, &sequential);
        });
}

#[test]
fn edb_only_outputs_preserve_sets() {
    rayon::ThreadPoolBuilder::new()
        .num_threads(2)
        .build()
        .unwrap()
        .install(|| {
            for count in [0, 1, 1100] {
                let input: Vec<_> = (0..count).rev().flat_map(|x| [(x,), (x,)]).collect();
                macro_rules! run {
                ($($mode:tt)*) => {
                    cq_datalog::datalog! {
                        pipeline { $($mode)* }
                        struct Inputs;
                        relation Input(x:i32) = (&input);
                        output Input;
                    }.collect::<Vec<_>>()
                };
            }
                let parallel = run!(storage_rewrites = [cq_datalog_parallel::shard_storage]; round = [cq_datalog_parallel::partition]; print = cq_datalog_parallel::print_rounds;);
                assert_same(&parallel, &run!());
                ascent::ascent! {
                    struct Oracle;
                    relation input(i32);
                }
                let mut oracle = Oracle {
                    input,
                    ..Default::default()
                };
                oracle.run();
                assert_ascent(&parallel, oracle.input);
            }
        });
}

#[test]
fn observer_stays_on_the_evaluating_thread() {
    rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap()
        .install(|| {
            let caller = std::thread::current().id();
            // Rc<Cell<_>> deliberately is neither Send nor Sync: observers are not workers.
            let calls = std::rc::Rc::new(std::cell::Cell::new(0));
            let input: Vec<_> = (0..2200).map(|x| (x,)).collect();
            let rows = cq_datalog::datalog! {
                pipeline { storage_rewrites = [cq_datalog_parallel::shard_storage]; round = [cq_datalog_parallel::partition]; print = cq_datalog_parallel::print_rounds; }
                observe = (|_, _| {
                    assert_eq!(std::thread::current().id(), caller);
                    calls.set(calls.get() + 1);
                });
                struct Observed;
                relation Input(x:i32) = (&input);
                relation Out(x:i32);
                output Out;
                Out(*x) :- Input(x);
            }
            .collect::<Vec<_>>();
            assert_ascent(&rows, input);
            assert!(calls.get() > 0);
        });
}

#[test]
fn worker_panics_propagate_without_returning_partial_answers() {
    fn checked(x: i32) -> i32 {
        assert_ne!(x, 1500, "parallel candidate panic");
        x
    }
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(2)
        .build()
        .unwrap();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        pool.install(|| {
            let input: Vec<_> = (0..2200).map(|x| (x,)).collect();
            cq_datalog::datalog! {
                pipeline { storage_rewrites = [cq_datalog_parallel::shard_storage]; round = [cq_datalog_parallel::partition]; print = cq_datalog_parallel::print_rounds; }
                struct Panics;
                relation Input(x:i32) = (&input);
                relation Out(x:i32);
                output Out;
                Out(checked(*x)) :- Input(x);
            }
            .collect::<Vec<_>>()
        })
    }))
    .expect_err("candidate panic must escape the parallel evaluator");
    let message = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .unwrap_or("");
    assert!(message.contains("parallel candidate panic"));
}

#[test]
fn generated_bindings_do_not_shadow_rust_captures() {
    let input: Vec<_> = (0..2200).map(|x| (x,)).collect();
    let (source, evaluate, scope, grain, chunk) = (1, 2, 3, 4, 5);
    let (__cq_source, __cq_evaluate, __cq_scope, __cq_grain, __cq_admitted_chunk) =
        (6, 7, 8, 9, 10);
    macro_rules! run {
        ($($mode:tt)*) => {
            cq_datalog::datalog! {
                pipeline { $($mode)* }
                struct Hygiene;
                relation Input(x:i32) = (&input);
                relation Output(x:i32);
                output Output;
                Output(*x + source + evaluate + scope + grain + chunk
                    + __cq_source + __cq_evaluate + __cq_scope + __cq_grain + __cq_admitted_chunk)
                    :- Input(x);
            }.collect::<Vec<_>>()
        };
    }
    let sequential = run!();
    let parallel = run!(storage_rewrites = [cq_datalog_parallel::shard_storage]; round = [cq_datalog_parallel::partition]; print = cq_datalog_parallel::print_rounds;);
    assert_same(&parallel, &sequential);
    assert_eq!(
        parallel.into_iter().collect::<BTreeSet<_>>(),
        (55..2255).map(|x| (x,)).collect()
    );
}

#[test]
fn small_work_threshold_evaluates_inline_and_preserves_sets() {
    rayon::ThreadPoolBuilder::new()
        .num_threads(15)
        .build()
        .unwrap()
        .install(|| {
            let caller = std::thread::current().id();
            let inputs: Vec<_> = (0..31_i32).map(|x| (x,)).collect();
            let checked = |x: i32| {
                assert_eq!(std::thread::current().id(), caller);
                x
            };
            let actual = cq_datalog::datalog! {
                pipeline {
                    storage_rewrites = [cq_datalog_parallel::shard_storage];
                    round = [cq_datalog_parallel::partition(threshold = 1024, grain = 1)];
                    print = cq_datalog_parallel::print_rounds;
                }
                struct Inline;
                relation Input(x:i32) = (&inputs);
                relation Reach(x:i32);
                output Reach;
                Reach(checked(*x)) :- Input(x);
                Reach(checked(*x + 1)) :- Reach(x), if *x < 31;
            }
            .collect::<BTreeSet<_>>();
            assert_eq!(actual, (0..=31).map(|x| (x,)).collect());
        });
}
