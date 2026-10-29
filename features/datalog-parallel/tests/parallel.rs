//! Parallel admission order is unspecified; all engines are compared as sets.
//! Small fixtures also evaluate Round independently before and after each rewrite.
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
fn recorded_waves_match_round_oracle_for_mutual_heads() {
    for threads in [1, 4] {
        rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap().install(|| {
            let rows: Vec<_> = (0..32).map(|x| (x,)).collect();
            macro_rules! run {
                ($($mode:tt)*) => {{
                    let (a, b) = cq_datalog::datalog! {
                        pipeline { $($mode)* }
                        struct MutualWaves;
                        relation E(x:i32) = (&rows);
                        relation A(x:i32);
                        relation B(x:i32);
                        output A, B;
                        A(*x) :- E(x);
                        A(*x) :- B(x);
                        A(*x + 1) :- B(x), where x < 40;
                        B(*x) :- E(x);
                        B(*x) :- A(x);
                        B(*x + 1) :- A(x), where x < 40;
                    };
                    (a.into_iter().collect::<Vec<_>>(), b.into_iter().collect::<Vec<_>>())
                }};
            }
            let expected = run!(print = cq_datalog_test_passes::eval_round;);
            macro_rules! check {
                ($schedule:ident) => {{
                    let actual = run!(
                        storage_rewrites = [cq_datalog_parallel::shard_storage];
                        round = [cq_datalog_schedule::schedule($schedule), cq_datalog_parallel::partition(threshold = 0, grain = 2)];
                        print = cq_datalog_parallel::print_rounds;
                    );
                    assert_same(&actual.0, &expected.0);
                    assert_same(&actual.1, &expected.1);
                }};
            }
            check!(whole);
            check!(per_hole);
            check!(by_head);
        });
    }
}

#[test]
fn early_publication_of_three_head_cycle_matches_round() {
    for threads in [1, 4] {
        rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap().install(|| {
            let rows: Vec<_> = (0..8).map(|x| (x,)).collect();
            macro_rules! run {
                ($($mode:tt)*) => {{
                    let (a, b, c) = cq_datalog::datalog! {
                        pipeline { $($mode)* }
                        struct EarlyCycle;
                        relation E(x:i32) = (&rows);
                        relation A(x:i32);
                        relation B(x:i32);
                        relation C(x:i32);
                        output A, B, C;
                        A(*x) :- E(x);
                        A(*x) :- C(x);
                        B(*x) :- A(x);
                        C(*x + 1) :- B(x), where x < 12;
                    };
                    (a.into_iter().collect::<Vec<_>>(), b.into_iter().collect::<Vec<_>>(), c.into_iter().collect::<Vec<_>>())
                }};
            }
            let expected = run!(print = cq_datalog_test_passes::eval_round;);
            let actual = run!(
                storage_rewrites = [cq_datalog_parallel::shard_storage];
                round = [cq_datalog_schedule::schedule(per_hole), cq_datalog_parallel::partition(threshold = 0, grain = 2)];
                print = cq_datalog_parallel::print_rounds;
            );
            assert_same(&actual.0, &expected.0);
            assert_same(&actual.1, &expected.1);
            assert_same(&actual.2, &expected.2);
        });
    }
}

#[test]
fn per_hole_chunk_policy_preserves_two_rule_sets() {
    for threads in [1, 4] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                let rows: Vec<_> = (0..32).map(|x| (x,)).collect();
                macro_rules! run {
                ($($policy:tt)*) => {
                    cq_datalog::datalog! {
                        pipeline {
                            storage_rewrites = [cq_datalog_parallel::shard_storage];
                            round = [$($policy)*];
                            print = cq_datalog_parallel::print_rounds;
                        }
                        struct Policy;
                        relation E(x:i32) = (&rows);
                        relation A(x:i32);
                        output A;
                        A(*x) :- E(x);
                        A(*x + 1) :- E(x);
                    }.into_iter().collect::<Vec<_>>()
                };
            }
                assert_same(
                    &run!(cq_datalog_parallel_test_passes::hole_policy),
                    &run!(cq_datalog_parallel::partition),
                );
            });
    }
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
                        }.into_iter().collect::<Vec<_>>()
                    };
                }
                    let sequential = run!();
                    let parallel = run!(storage_rewrites = [cq_datalog_parallel::shard_storage]; round = [cq_datalog_parallel::partition]; print = cq_datalog_parallel::print_rounds;);
                    assert_same(&parallel, &sequential);
                    if count <= 7 {
                        let reference = run!(print = cq_datalog_test_passes::eval_round;);
                        let incremental = run!(print = cq_datalog_test_passes::eval_incremental_from_round;);
                        assert_same(&reference, &incremental);
                        let sharded = run!(storage_rewrites = [cq_datalog_parallel::shard_storage]; print = cq_datalog_test_passes::eval_round;);
                        let rewritten = run!(storage_rewrites = [cq_datalog_parallel::shard_storage]; round = [cq_datalog_parallel::partition]; print = cq_datalog_parallel_test_passes::eval_round;);
                        assert_same(&reference, &sequential);
                        assert_same(&sharded, &reference);
                        assert_same(&rewritten, &reference);
                    }
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
                (labels.into_iter().collect::<Vec<_>>(), reach.into_iter().collect::<Vec<_>>(), input.into_iter().collect::<Vec<_>>())
            }};
        }
        let sequential = run!();
        let parallel = run!(storage_rewrites = [cq_datalog_parallel::shard_storage]; round = [cq_datalog_parallel::partition]; print = cq_datalog_parallel::print_rounds;);
        assert_same(&parallel.0, &sequential.0);
        assert_same(&parallel.1, &sequential.1);
        assert_same(&parallel.2, &sequential.2);
        let reference = run!(print = cq_datalog_test_passes::eval_round;);
        let incremental = run!(print = cq_datalog_test_passes::eval_incremental_from_round;);
        assert_same(&reference.0, &incremental.0);
        assert_same(&reference.1, &incremental.1);
        assert_same(&reference.2, &incremental.2);
        let rewritten = run!(storage_rewrites = [cq_datalog_parallel::shard_storage]; round = [cq_datalog_parallel::partition]; print = cq_datalog_parallel_test_passes::eval_round;);
        assert_same(&reference.0, &sequential.0);
        assert_same(&reference.1, &sequential.1);
        assert_same(&reference.2, &sequential.2);
        assert_same(&rewritten.0, &reference.0);
        assert_same(&rewritten.1, &reference.1);
        assert_same(&rewritten.2, &reference.2);
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
            macro_rules! resumed {
                ($($evaluate:ident)::+; $($pipeline:tt)*) => {
                    $($evaluate)::+! {
                pipeline { $($pipeline)* };
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
                    }.into_iter().collect::<Vec<_>>()
                };
            }
            let seminaive = resumed!(cq_datalog::passes::storage;
                cq_datalog_parallel::shard_storage, cq_datalog::passes::fork,
                cq_datalog_parallel::partition, cq_datalog_parallel::print_rounds);
            let incremental = resumed!(cq_datalog_test_passes::eval_incremental;);
            let round = resumed!(cq_datalog::passes::storage;
                cq_datalog::passes::fork, cq_datalog_test_passes::eval_round);
            let rewritten = resumed!(cq_datalog::passes::storage;
                cq_datalog_parallel::shard_storage, cq_datalog::passes::fork,
                cq_datalog_parallel::partition, cq_datalog_parallel_test_passes::eval_round);
            assert_same(&stratified, &sequential);
            assert_same(&seminaive, &sequential);
            assert_same(&incremental, &sequential);
            assert_same(&round, &incremental);
            assert_same(&rewritten, &round);
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
                    }.into_iter().collect::<Vec<_>>()
                };
            }
                let parallel = run!(storage_rewrites = [cq_datalog_parallel::shard_storage]; round = [cq_datalog_parallel::partition]; print = cq_datalog_parallel::print_rounds;);
                assert_same(&parallel, &run!());
                let reference = run!(print = cq_datalog_test_passes::eval_round;);
                let rewritten = run!(storage_rewrites = [cq_datalog_parallel::shard_storage]; round = [cq_datalog_parallel::partition]; print = cq_datalog_parallel_test_passes::eval_round;);
                assert_same(&reference, &parallel);
                assert_same(&rewritten, &reference);
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
            let threads = [
                std::sync::Mutex::new(Vec::new()),
                std::sync::Mutex::new(Vec::new()),
            ];
            let inputs: Vec<_> = (0..31_i32).map(|x| (x,)).collect();
            let checked = |x: i32, hole: usize| {
                threads[hole]
                    .lock()
                    .unwrap()
                    .push(std::thread::current().id());
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
                Reach(checked(*x, 0)) :- Input(x);
                Reach(checked(*x + 1, 1)) :- Reach(x), where x < 31;
            }
            .collect::<BTreeSet<_>>();
            assert_eq!(actual, (0..=31).map(|x| (x,)).collect());
            // Small drivers inline their chunks, not concurrency between holes.
            // Each hole here emits 31 callbacks in its sole nonempty evaluation.
            for recorded in threads {
                let recorded = recorded.into_inner().unwrap();
                assert_eq!(recorded.len(), 31);
                assert!(recorded.iter().all(|thread| thread == &recorded[0]));
            }
        });
}

#[test]
fn duplicate_heavy_partitioned_and_single_task_rules_share_canonical_admission() {
    for threads in [1, 4] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                let rows: Vec<_> = (0..128_i32).map(|x| (x,)).collect();
                let blocked = [(127_i32,)];
                let enabled = true;
                macro_rules! run {
                    ($($mode:tt)*) => {
                        cq_datalog::datalog! {
                            pipeline { $($mode)* }
                            struct RepeatedCandidates;
                            relation E(x:i32) = (&rows);
                            relation B(x:i32) = (blocked);
                            relation R(x:i32);
                            output R;
                            R(*x % 4) :- E(x), !B(x);
                            R((*x + *offset) % 4) :- if enabled, E(x),
                                for offset in [0, 0, 1, 1], !B(x);
                            R((*x + 1) % 4) :- R(x);
                        }.collect::<Vec<_>>()
                    };
                }
                let sequential = run!();
                let parallel = run!(
                    storage_rewrites = [cq_datalog_parallel::shard_storage];
                    round = [cq_datalog_parallel::partition(threshold = 0, grain = 2)];
                    print = cq_datalog_parallel::print_rounds;
                );
                assert_eq!(parallel.len(), 4);
                assert_same(&parallel, &sequential);
                assert_eq!(
                    parallel.into_iter().collect::<BTreeSet<_>>(),
                    BTreeSet::from([(0,), (1,), (2,), (3,)])
                );
            });
    }
}
