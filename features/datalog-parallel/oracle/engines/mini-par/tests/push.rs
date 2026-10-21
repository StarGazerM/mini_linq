//! All shared programs through sharded push evaluation and push key collection.
oracle_shared::oracle_programs!(
    pushed,
    mini,
    ascent::ascent,
    oracle_shared::oracle_input,
    pipeline {
        storage_rewrites = [cq_datalog_parallel::shard_storage];
        round = [cq_datalog_parallel::partition(threshold = 0, grain = 2)];
        print = cq_datalog_parallel::print_rounds;
        cq = pipeline { print = ::cq_datalog::passes::print_push; };
    }
);

macro_rules! cases {
    ($($name:ident),* $(,)?) => {
        const CASES: &[(&str, oracle_shared::harness::Evaluate,
            oracle_shared::harness::Evaluate, oracle_shared::harness::Evaluate)] = &[
            $((stringify!($name), pushed::$name::mini,
                oracle_mini_par::$name::mini, oracle_ascent::$name::oracle)),*
        ];
    };
}
cases!(
    reach,
    tc_left,
    tc_right,
    tc_nonlinear,
    same_generation,
    mutual,
    multiple_seeds,
    guard,
    unreachable,
    nonedges,
    recursive_upper,
    andersen,
    cspa
);

#[test]
fn all_oracle_programs_match_pull_and_ascent_in_local_pools() {
    for threads in [1, 2, 4] {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                for &(name, push, pull, ascent) in CASES {
                    let compare = |data: &oracle_shared::Data| {
                        let expected = ascent(data).0;
                        assert_eq!(pull(data).0, expected, "{name} pull/{threads}");
                        assert_eq!(push(data).0, expected, "{name} push/{threads}");
                    };
                    for scenario in oracle_shared::SCENARIOS {
                        compare(&oracle_shared::generate(scenario, 6, 0x5eed));
                    }
                    for seed in 0..oracle_shared::RANDOM_SEEDS {
                        compare(&oracle_shared::generate("random", 6, seed));
                    }
                }
            });
    }
}
