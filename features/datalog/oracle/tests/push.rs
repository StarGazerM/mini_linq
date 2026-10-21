//! The unchanged oracle corpus through streaming push holes, including key holes.
#![cfg(all(feature = "mini", feature = "ascent"))]

oracle_shared::oracle_programs!(
    pushed,
    mini,
    ascent::ascent,
    oracle_shared::oracle_input,
    pipeline { cq = pipeline { print = ::cq_datalog::passes::print_push; }; }
);

macro_rules! cases {
    ($($name:ident),* $(,)?) => {
        const CASES: &[(&str, oracle_shared::harness::Evaluate,
            oracle_shared::harness::Evaluate, oracle_shared::harness::Evaluate)] = &[
            $((stringify!($name), pushed::$name::mini,
                oracle_mini::$name::mini, oracle_ascent::$name::oracle)),*
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
fn all_oracle_programs_match_pull_and_ascent() {
    for &(name, push, pull, ascent) in CASES {
        let compare = |data: &oracle_shared::Data| {
            let expected = ascent(data).0;
            assert_eq!(pull(data).0, expected, "{name} pull");
            assert_eq!(push(data).0, expected, "{name} push");
            if let Some(reference) = oracle_shared::naive(name, data) {
                assert_eq!(expected, reference, "{name} independent reference");
            }
        };
        for scenario in oracle_shared::SCENARIOS {
            compare(&oracle_shared::generate(scenario, 6, 0x5eed));
        }
        for seed in 0..oracle_shared::RANDOM_SEEDS {
            compare(&oracle_shared::generate("random", 6, seed));
        }
    }
}
