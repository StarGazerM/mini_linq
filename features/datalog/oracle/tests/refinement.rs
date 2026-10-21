//! Execute the same 13 public-surface programs at every semantic boundary.
//! Each reference returns complete sets; no production CQ printer is involved.
use oracle_shared::{Data, RANDOM_SEEDS, Rows, SCENARIOS, generate};
use std::time::Duration;

oracle_shared::oracle_programs!(source, mini, ascent::ascent, oracle_shared::oracle_input,
    pipeline { print = cq_datalog_test_passes::eval_source_from_round; });
oracle_shared::oracle_programs!(strata, mini, ascent::ascent, oracle_shared::oracle_input,
    pipeline { print = cq_datalog_test_passes::eval_strata_from_round; });
oracle_shared::oracle_programs!(incremental, mini, ascent::ascent, oracle_shared::oracle_input,
    pipeline { print = cq_datalog_test_passes::eval_incremental_from_round; });
oracle_shared::oracle_programs!(round, mini, ascent::ascent, oracle_shared::oracle_input,
    pipeline { print = cq_datalog_test_passes::eval_round; });
oracle_shared::oracle_programs!(
    production,
    mini,
    ascent::ascent,
    oracle_shared::oracle_input,
    pipeline {}
);
oracle_shared::oracle_programs!(
    oracle,
    ascent,
    ascent::ascent,
    oracle_shared::oracle_input,
    pipeline {}
);

type Evaluate = fn(&Data) -> (Rows, Duration);
struct Case {
    name: &'static str,
    stages: [Evaluate; 5],
    oracle: Evaluate,
}
macro_rules! cases {
    ($($name:ident),* $(,)?) => {
        const CASES: &[Case] = &[$(Case {
            name: stringify!($name),
            stages: [source::$name::mini, strata::$name::mini,
                incremental::$name::mini, round::$name::mini, production::$name::mini],
            oracle: oracle::$name::oracle,
        }),*];
    };
}
cases! { reach, tc_left, tc_right, tc_nonlinear, same_generation,
mutual, multiple_seeds, guard, unreachable, nonedges,
recursive_upper, andersen, cspa }

#[test]
fn all_oracle_programs_refine_each_executable_stage() {
    for case in CASES {
        for (scenario, seed) in SCENARIOS
            .iter()
            .map(|&name| (name, 0x5eed))
            .chain((0..RANDOM_SEEDS).map(|seed| ("random", seed)))
        {
            let data = generate(scenario, 6, seed);
            let expected = (case.oracle)(&data).0;
            for (stage, evaluate) in ["Source", "Strata", "Incremental", "Round", "production"]
                .into_iter()
                .zip(case.stages)
            {
                assert_eq!(
                    evaluate(&data).0,
                    expected,
                    "{} / {stage} / {scenario} / seed={seed}",
                    case.name
                );
            }
        }
    }
}
