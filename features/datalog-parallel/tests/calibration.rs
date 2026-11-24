#![cfg(feature = "planner-calibration")]
//! PS-After under a relational schedule: there are no synchronous rounds,
//! and a row still has an age, on the clock of the program's publications.
use cq_storage::statistics::collection::{with_observer, Age, Observer, RelationRows, ScalarValue};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Ages(Mutex<BTreeMap<usize, Vec<(Age, i64)>>>);
impl Observer for Ages {
    fn needs_rounds(&self, _relation: usize) -> bool { true }
    fn relation(&self, relation: usize, _rows: Option<usize>, pass: RelationRows<'_>) {
        let mut rows = Vec::new();
        pass(&mut |age, row| {
            let ScalarValue::Signed(value) = row.cell(0) else { panic!("an integer column") };
            rows.push((age, value));
        });
        self.0.lock().unwrap().insert(relation, rows);
    }
}

/// Two mutually recursive relations over one chain: a row of either is
/// derived from a delta row of the other.
fn alternate(relational: bool, edges: &[(i32, i32)], seeds: &[(i32,)]) -> (Vec<(i32,)>, Vec<(i32,)>) {
    macro_rules! run {
        ($($schedule:tt)*) => { cq_datalog::datalog! {
            pipeline {
                fork = cq_datalog::passes::fork(timing = none;);
                storage_rewrites = [cq_datalog_parallel::shard_storage];
                round = [$($schedule)* cq_datalog_parallel::partition(threshold = 0, grain = 2)];
                print = cq_datalog_parallel::print_rounds;
            }
            struct Alternate;
            relation Edge(x: i32, y: i32) = (edges);
            relation Seed(x: i32) = (seeds);
            relation Even(x: i32);
            relation Odd(x: i32);
            output Even, Odd;
            Even(*x) :- Seed(x);
            Odd(*y) :- Even(x), Edge(x, y);
            Even(*y) :- Odd(x), Edge(x, y);
        } };
    }
    let (even, odd) = if relational { run!(cq_datalog_schedule::schedule(relational),) } else { run!() };
    (even.into_iter().collect(), odd.into_iter().collect())
}

#[test]
fn a_relational_schedule_gives_every_row_an_age_on_the_publication_clock() {
    let edges: Vec<(i32, i32)> = (0..40).map(|at| (at, at + 1)).collect();
    let seeds = [(0,)];
    for (relational, threads) in [(false, 2), (true, 1), (true, 2), (true, 4)] {
        let seen = Arc::new(Ages::default());
        let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
        let (mut even, mut odd) = pool.install(|| with_observer(seen.clone(), || alternate(relational, &edges, &seeds)));
        even.sort_unstable();
        odd.sort_unstable();
        assert_eq!(even, (0..=40).step_by(2).map(|at| (at,)).collect::<Vec<_>>());
        assert_eq!(odd, (1..=40).step_by(2).map(|at| (at,)).collect::<Vec<_>>());
        let seen = seen.0.lock().unwrap();
        // Relations in declaration order: Edge, Seed, Even, Odd.
        let mut age: BTreeMap<i64, Age> = BTreeMap::new();
        for relation in [2, 3] {
            let rows = &seen[&relation];
            assert!(rows.is_sorted_by_key(|(age, _)| (age.published, age.horizon)), "ages ascend along a pass: {rows:?}");
            for &(row, value) in rows {
                assert!(row.published <= row.horizon, "{row:?}");
                assert!(age.insert(value, row).is_none(), "{value} passed twice");
            }
        }
        assert_eq!(age.len(), 41);
        // Row k+1 of one relation is derived from the delta row k of the
        // other: k was published first, and when k was a delta row, k+1
        // was not yet there to be met.
        for at in 0..40 {
            let (from, to) = (age[&at], age[&(at + 1)]);
            assert!(from.published < to.published, "relational {relational}, {threads} threads: {at} {from:?} then {to:?}");
            assert!(to.published > from.horizon, "relational {relational}, {threads} threads: {at} {from:?} met {to:?}");
        }
        // A row k is met by the rows published up to its horizon: none of
        // the rows derived after it, all of those before.
        for (at, row) in &age {
            for (other, met) in &age {
                if other + 1 < *at { assert!(met.published <= row.horizon, "{other} {met:?} was there for {at} {row:?}"); }
            }
        }
    }
}
