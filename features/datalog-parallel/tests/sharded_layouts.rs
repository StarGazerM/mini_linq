//! Keyed IDB full/delta layouts must preserve exact Round answers across schedules.
use std::collections::BTreeSet;

fn sparse_graph() -> Vec<(i32, i32)> {
    // Four layers cover 2,000 nodes without a dense transitive closure. Each
    // source has three distinct successors, so the 4,500-row seed also crosses
    // the runtime's bulk-insertion threshold. Reuse the relational fixture's LCG.
    let mut state = 0x5eed_u64;
    let mut edges = Vec::with_capacity(4500);
    for source in 0..1500 {
        let next_layer = (source / 500 + 1) * 500;
        let column = source % 500;
        edges.push((source, next_layer + column));
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let first = 1 + ((state >> 32) % 249) as i32;
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let second = 250 + ((state >> 32) % 250) as i32;
        edges.push((source, next_layer + (column + first) % 500));
        edges.push((source, next_layer + (column + second) % 500));
    }
    edges
}

fn assert_exact<T: Ord + std::fmt::Debug>(
    actual: impl IntoIterator<Item = T>,
    expected: &BTreeSet<T>,
    threads: usize,
    schedule: &str,
) {
    let rows: Vec<_> = actual.into_iter().collect();
    assert_eq!(rows.len(), expected.len(), "duplicate or missing rows: {schedule}/{threads}");
    assert_eq!(&rows.into_iter().collect::<BTreeSet<_>>(), expected, "{schedule}/{threads}");
}

macro_rules! closure {
    ($edges:expr; $($pipeline:tt)*) => {
        cq_datalog::datalog! {
            pipeline { $($pipeline)* }
            struct Closure;
            relation Edge(x:i32, y:i32) = ($edges);
            relation Reach(x:i32, y:i32);
            output Reach;
            Reach(*x, *y) :- Edge(x, y);
            Reach(*x, *z) :- Reach(x, y), Reach(y, z);
        }
    };
}

#[test]
fn recursive_keyed_joins_match_round_on_2000_node_sparse_graph() {
    let edges = sparse_graph();
    let expected: BTreeSet<_> = closure!(&edges;
        print = cq_datalog_test_passes::eval_round;
    ).into_iter().collect();
    for threads in [1, 4, 8] {
        rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap().install(|| {
            macro_rules! check {
                ($schedule:ident) => {
                    assert_exact(closure!(&edges;
                        fork = cq_datalog::passes::fork(timing = none;);
                        storage_rewrites = [cq_datalog_parallel::shard_storage(layouts(sharded))];
                        round = [cq_datalog_schedule::schedule($schedule), cq_datalog_parallel::partition(threshold = 0, grain = 128)];
                        print = cq_datalog_parallel::print_rounds;
                    ), &expected, threads, stringify!($schedule));
                };
            }
            check!(whole);
            check!(by_head);
            check!(relational);
        });
    }
}

#[test]
fn keyed_three_member_scc_matches_round() {
    let edges = sparse_graph();
    macro_rules! run {
        ($($pipeline:tt)*) => {
            cq_datalog::datalog! {
                pipeline { $($pipeline)* }
                struct Cycle;
                relation Edge(x:i32, y:i32) = (&edges);
                relation A(x:i32, y:i32);
                relation B(x:i32, y:i32);
                relation C(x:i32, y:i32);
                output A, B, C;
                A(*x, *y) :- Edge(x, y);
                A(*x, *z) :- C(x, y), B(y, z);
                B(*x, *y) :- A(x, y);
                C(*x, *y) :- B(x, y);
            }
        };
    }
    let (a, b, c) = run!(print = cq_datalog_test_passes::eval_round;);
    let expected = (
        a.into_iter().collect::<BTreeSet<_>>(),
        b.into_iter().collect::<BTreeSet<_>>(),
        c.into_iter().collect::<BTreeSet<_>>(),
    );
    for threads in [1, 4, 8] {
        rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap().install(|| {
            macro_rules! check {
                ($schedule:ident) => {{
                    let (a, b, c) = run!(
                        fork = cq_datalog::passes::fork(timing = none;);
                        storage_rewrites = [cq_datalog_parallel::shard_storage(layouts(sharded))];
                        round = [cq_datalog_schedule::schedule($schedule), cq_datalog_parallel::partition(threshold = 0, grain = 128)];
                        print = cq_datalog_parallel::print_rounds;
                    );
                    assert_exact(a, &expected.0, threads, stringify!($schedule));
                    assert_exact(b, &expected.1, threads, stringify!($schedule));
                    assert_exact(c, &expected.2, threads, stringify!($schedule));
                }};
            }
            check!(whole);
            check!(by_head);
            check!(relational);
        });
    }
}

#[test]
fn keyed_hasher_composes_with_sharded_layouts_in_both_orders() {
    let edges = [(0, 1), (1, 2), (2, 0), (2, 3), (0, 1)];
    let expected: BTreeSet<_> = closure!(&edges;
        print = cq_datalog_test_passes::eval_round;
    ).into_iter().collect();
    rayon::ThreadPoolBuilder::new().num_threads(4).build().unwrap().install(|| {
        macro_rules! check {
            ($($storage:tt)*) => {
                assert_exact(closure!(&edges;
                    storage_rewrites = [$($storage)*];
                    round = [cq_datalog_hasher::hasher_round(keyed), cq_datalog_schedule::schedule(by_head), cq_datalog_parallel::partition(threshold = 0, grain = 2)];
                    print = cq_datalog_parallel::print_rounds;
                ), &expected, 4, "keyed/by_head");
            };
        }
        check!(cq_datalog_hasher::hasher(keyed), cq_datalog_parallel::shard_storage(layouts(sharded)));
        check!(cq_datalog_parallel::shard_storage(layouts(sharded)), cq_datalog_hasher::hasher(keyed));
    });
}
