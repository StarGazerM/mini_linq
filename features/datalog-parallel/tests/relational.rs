//! Repeated set-oracle comparisons exercise member-batch interleavings.
use std::collections::BTreeSet;

fn sets<T: Ord>(rows: impl IntoIterator<Item = T>) -> BTreeSet<T> {
    rows.into_iter().collect()
}

#[test]
fn closure_chain_and_random_graph() {
    let chain: Vec<(i32, i32)> = (0..199).map(|i| (i, i + 1)).collect();
    let mut state = 0x5eed_u64;
    let random: Vec<_> = (0..180)
        .map(|_| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let a = ((state >> 32) % 40) as i32;
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            (a, ((state >> 32) % 40) as i32)
        })
        .collect();
    for edges in [&chain, &random] {
        macro_rules! run {
            ($($pipeline:tt)*) => { cq_datalog::datalog! {
                pipeline { $($pipeline)* }
                struct Closure;
                relation E(x:i32, y:i32) = (edges);
                relation R(x:i32, y:i32);
                output R;
                R(*x, *y) :- E(x, y);
                R(*x, *z) :- R(x, y), E(y, z);
            } };
        }
        let expected = sets(run!(print = cq_datalog_test_passes::eval_round;));
        for threads in [1, 2, 4, 8] {
            rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap().install(|| {
                for _ in 0..20 {
                    let actual = run!(
                        fork = cq_datalog::passes::fork(timing = none;);
                        storage_rewrites = [cq_datalog_parallel::shard_storage];
                        round = [cq_datalog_schedule::schedule(relational), cq_datalog_parallel::partition(threshold = 0, grain = 2)];
                        print = cq_datalog_parallel::print_rounds;
                    );
                    assert_eq!(sets(actual), expected);
                }
            });
        }
    }
}

#[test]
fn three_member_two_rules_per_head() {
    let rows = vec![(0_i32,), (3,)];
    macro_rules! run {
        ($($pipeline:tt)*) => { cq_datalog::datalog! {
            pipeline { $($pipeline)* }
            struct Cycle;
            relation E(x:i32) = (&rows);
            relation A(x:i32);
            relation B(x:i32);
            relation C(x:i32);
            output A, B, C;
            A(*x) :- E(x);
            A(*x) :- C(x);
            B(*x) :- A(x);
            B(*x + 1) :- A(x), if *x < 30;
            C(*x) :- B(x);
            C(*x + 1) :- B(x), if *x < 30;
        } };
    }
    let (a, b, c) = run!(print = cq_datalog_test_passes::eval_round;);
    let expected = (sets(a), sets(b), sets(c));
    for threads in [1, 2, 4, 8] {
        rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap().install(|| {
            for _ in 0..20 {
                let (a,b,c) = run!(
                    fork = cq_datalog::passes::fork(timing = none;);
                    storage_rewrites = [cq_datalog_parallel::shard_storage];
                    round = [cq_datalog_schedule::schedule(relational), cq_datalog_parallel::partition(threshold = 0, grain = 2)];
                    print = cq_datalog_parallel::print_rounds;
                );
                assert_eq!((sets(a),sets(b),sets(c)), expected);
            }
        });
    }
}

#[test]
fn lower_stratum_negation_feeds_recursion() {
    let rows: Vec<_> = (0..12_i32).map(|x| (x,)).collect();
    let blocked = vec![(0_i32,), (2,), (4,)];
    macro_rules! run {
        ($($pipeline:tt)*) => { cq_datalog::datalog! {
            pipeline { $($pipeline)* }
            struct NegativeSeed;
            relation E(x:i32) = (&rows);
            relation Block(x:i32) = (&blocked);
            relation Allowed(x:i32);
            relation R(x:i32);
            output Allowed, R;
            Allowed(*x) :- E(x), !Block(x);
            R(*x) :- Allowed(x);
            R(*x + 2) :- R(x), if *x < 40;
        } };
    }
    let (a, r) = run!(print = cq_datalog_test_passes::eval_round;);
    let expected = (sets(a), sets(r));
    for threads in [1, 2, 4, 8] {
        rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap().install(|| {
            for _ in 0..20 {
                let (a,r) = run!(
                    fork = cq_datalog::passes::fork(timing = none;);
                    storage_rewrites = [cq_datalog_parallel::shard_storage];
                    round = [cq_datalog_schedule::schedule(relational), cq_datalog_parallel::partition(threshold = 0, grain = 2)];
                    print = cq_datalog_parallel::print_rounds;
                );
                assert_eq!((sets(a),sets(r)), expected);
            }
        });
    }
}
