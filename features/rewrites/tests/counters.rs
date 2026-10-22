//! One finite fan experiment; this is not a WCOJ growth proof.
use cq_storage::counters::{self, Snapshot};
use cq_testkit::{Database, Rows, SCENARIOS, cq_semantics};

type Run = fn(&[(i32, i32)], &[(i32, i32)], &[(i32, i32)]) -> Rows;

macro_rules! runner {
    ($name:ident; $($rewrite:path),* $(,)?) => {
        #[allow(non_snake_case)]
        fn $name(R: &[(i32, i32)], S: &[(i32, i32)], T: &[(i32, i32)]) -> Rows {
            cq_rewrite_test_driver::adjacent_triangle! {
                pipeline {
                    $($rewrite,)*
                    mini_linq::passes::select,
                    cq_rewrites::retype(dedup => cq_storage::counters::DistinctSet<_>),
                    mini_linq::passes::sequence, mini_linq::passes::sequential, mini_linq::passes::translate_pull
                }
            }.map(|(a, b, c)| vec![*a, *b, *c]).collect()
        }
    };
}

runner!(first;);
runner!(canonical; cq_rewrites::canon_prefixes);
runner!(smallest; cq_rewrites::smallest_intersections);
runner!(combined; cq_rewrites::canon_prefixes, cq_rewrites::smallest_intersections);

#[test]
fn canonical_prefixes_and_smallest_intersections_reduce_fan_work() {
    let n = 48;
    let r: Vec<_> = (0..n).map(|b| (0, b)).collect();
    let s: Vec<_> = (0..n).flat_map(|b| (0..n).map(move |c| (b, c))).collect();
    let t = [(0, 0)]; // corpus T has declaration order (c, a)
    let database: Database = [
        ("R", r.as_slice()),
        ("S", s.as_slice()),
        ("T", t.as_slice()),
    ]
    .into_iter()
    .map(|(name, rows)| {
        (
            name.to_owned(),
            rows.iter().map(|(a, b)| vec![*a, *b]).collect(),
        )
    })
    .collect();
    let scenario = SCENARIOS
        .iter()
        .find(|s| s.name == "fine_triangle")
        .unwrap();
    let expected = cq_semantics::evaluate_cq(&scenario.parse_cq().unwrap(), &database);
    assert_eq!(expected, (0..n).map(|b| vec![0, b, 0]).collect());

    let measured = |run: Run| -> Snapshot {
        counters::reset();
        let actual = run(&r, &s, &t);
        let work = counters::snapshot();
        assert_eq!(actual, expected);
        assert!(work.distinct_insertions > 0);
        work
    };
    let first = measured(first);
    let canonical = measured(canonical);
    let smallest = measured(smallest);
    let combined = measured(combined);
    eprintln!(
        "fan n={n}: first={first:?}\ncanon={canonical:?}\nsmallest={smallest:?}\ncombined={combined:?}"
    );
    assert!(canonical.rows_enumerated < first.rows_enumerated);
    assert!(canonical.lookups < first.lookups);
    assert!(smallest.rows_enumerated < first.rows_enumerated);
    // The extra membership build enumerates the same rows saved by candidate
    // selection on this fan. Lookup and cursor work still fall independently.
    assert!(combined.rows_enumerated <= canonical.rows_enumerated);
    assert!(combined.lookups < canonical.lookups);
    assert!(combined.cursor_steps < canonical.cursor_steps);
    assert!(combined.rows_enumerated < smallest.rows_enumerated);
    assert!(combined.rows_enumerated < first.rows_enumerated);
}

#[test]
#[allow(non_snake_case)]
fn active_smallest_matches_authored_corpus_answers_and_empty_inputs() {
    let case = SCENARIOS
        .iter()
        .find(|s| s.name == "fine_triangle")
        .unwrap()
        .case()
        .unwrap();
    let rows = |name| {
        case.inputs
            .iter()
            .find(|(n, _)| *n == name)
            .unwrap()
            .1
            .iter()
            .map(|r| (r[0], r[1]))
            .collect::<Vec<_>>()
    };
    let R = rows("R");
    let S = rows("S");
    let T = rows("T");
    for run in [first, canonical, smallest, combined] {
        assert_eq!(run(&R, &S, &T), case.expected_rows());
        assert!(run(&[], &S, &T).is_empty());
        assert!(run(&R, &[], &T).is_empty());
        assert!(run(&R, &S, &[]).is_empty());
    }
}
