mod generated_support;

use cq_storage::counters::{self, Snapshot};
use cq_testkit::Rows;
use generated_support::{Inputs, Run, binary, fan, generic, oracle};

fn measured(run: Run, inputs: &Inputs, expected: &Rows) -> Snapshot {
    counters::reset();
    let actual = run(&inputs[0], &inputs[1], &inputs[2]);
    let counts = counters::snapshot();
    assert_eq!(&actual, expected);
    assert!(counts.rows_enumerated > 0);
    assert!(counts.index_builds > 0);
    assert!(counts.distinct_insertions > 0);
    counts
}

#[test]
fn generated_generic_fine_smallest_has_linear_fan_growth_against_binary() {
    let mut observations = Vec::new();
    for scale in [24, 48, 96] {
        let inputs = fan(scale);
        // Fixture/oracle work precedes reset; each snapshot follows a fresh,
        // fully exhausted query through the public planner/rewrite macros.
        let expected = oracle(&inputs);
        let binary = measured(binary, &inputs, &expected);
        let generic = measured(generic, &inputs, &expected);
        eprintln!("fan n={scale}: binary={binary:?}; generic={generic:?}");
        observations.push((scale, binary, generic));
    }
    for pair in observations.windows(2) {
        let (small_scale, small_binary, small_generic) = pair[0];
        let (large_scale, large_binary, large_generic) = pair[1];
        assert_eq!(large_scale, small_scale * 2);
        assert!(
            large_binary.rows_enumerated >= 3 * small_binary.rows_enumerated,
            "binary fan growth below 3x: {:?} -> {:?}",
            pair[0],
            pair[1]
        );
        assert!(
            2 * large_generic.rows_enumerated <= 5 * small_generic.rows_enumerated,
            "generic-fine + canon_prefixes + smallest fan growth exceeds 2.5x: {:?} -> {:?}",
            pair[0],
            pair[1]
        );
    }
}
