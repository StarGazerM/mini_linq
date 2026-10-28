//! All shared programs through parallel push evaluation and storage providers.
oracle_shared::oracle_programs!(
    pushed,
    mini,
    ascent::ascent,
    oracle_shared::oracle_input,
    pipeline {
        storage_rewrites = [cq_datalog_parallel::shard_storage];
        round = [cq_datalog_parallel::partition(threshold = 0, grain = 2)];
        print = cq_datalog_parallel::print_rounds;
        cq = pipeline { print_input = structures; print = ::cq_datalog::passes::print_push; };
    }
);

oracle_shared::oracle_programs!(
    by_head,
    mini,
    ascent::ascent,
    oracle_shared::oracle_input,
    pipeline {
        storage_rewrites = [cq_datalog_parallel::shard_storage];
        round = [cq_datalog_schedule::schedule(by_head), cq_datalog_parallel::partition(threshold = 0, grain = 2)];
        print = cq_datalog_parallel::print_rounds;
    }
);

oracle_shared::oracle_programs!(
    relational,
    mini,
    ascent::ascent,
    oracle_shared::oracle_input,
    pipeline {
        fork = cq_datalog::passes::fork(timing = none;);
        storage_rewrites = [cq_datalog_parallel::shard_storage];
        round = [cq_datalog_schedule::schedule(relational), cq_datalog_parallel::partition(threshold = 0, grain = 2)];
        print = cq_datalog_parallel::print_rounds;
    }
);

oracle_shared::oracle_programs!(
    sharded_layouts,
    mini,
    ascent::ascent,
    oracle_shared::oracle_input,
    pipeline {
        storage_rewrites = [cq_datalog_parallel::shard_storage(layouts(sharded))];
        round = [cq_datalog_parallel::partition(threshold = 0, grain = 2)];
        print = cq_datalog_parallel::print_rounds;
    }
);

oracle_shared::oracle_programs!(
    runs,
    mini,
    ascent::ascent,
    oracle_shared::oracle_input,
    pipeline {
        storage_rewrites = [cq_datalog_parallel::shard_storage(layouts(runs), seen(runs))];
        round = [cq_datalog_parallel::partition(threshold = 0, grain = 2)];
        print = cq_datalog_parallel::print_rounds;
    }
);

macro_rules! cases {
    ($($name:ident),* $(,)?) => {
        const CASES: &[(&str, oracle_shared::harness::Evaluate,
            oracle_shared::harness::Evaluate, oracle_shared::harness::Evaluate,
            oracle_shared::harness::Evaluate, oracle_shared::harness::Evaluate,
            oracle_shared::harness::Evaluate, oracle_shared::harness::Evaluate)] = &[
            $((stringify!($name), pushed::$name::mini,
                oracle_mini_par::$name::mini, oracle_ascent::$name::oracle,
                by_head::$name::mini, relational::$name::mini,
                sharded_layouts::$name::mini, runs::$name::mini)),*
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
                for &(name, push, pull, ascent, scheduled, relational, sharded_layouts, runs) in
                    CASES
                {
                    let compare = |data: &oracle_shared::Data| {
                        let expected = ascent(data).0;
                        assert_eq!(pull(data).0, expected, "{name} pull/{threads}");
                        assert_eq!(push(data).0, expected, "{name} push/{threads}");
                        assert_eq!(scheduled(data).0, expected, "{name} by_head/{threads}");
                        assert_eq!(relational(data).0, expected, "{name} relational/{threads}");
                        assert_eq!(
                            sharded_layouts(data).0,
                            expected,
                            "{name} sharded_layouts/{threads}"
                        );
                        assert_eq!(runs(data).0, expected, "{name} runs/{threads}");
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
