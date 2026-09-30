//! Measured extension execution. Setup and independent path/set oracles are
//! outside the timed region. Guard visits witness anti-membership checks;
//! the recursion observer counts nonempty delta rounds and delta rows.
//! Snapshot covers core storage/distinct operations; the internal hashing of
//! `Answers`/`GroupedAnswers` is not included in its hash counter.
use std::{
    cell::Cell,
    collections::BTreeSet,
    fmt::Write,
    time::{Duration, Instant},
};

use cq_storage::counters::{self, Snapshot};

macro_rules! policy { ($($ix:tt)*) => { ::mini_linq::lower_physical! { $($ix)* policy {} } }; }
pub(crate) use policy as physical_policy;
#[allow(unused_imports)]
mod mini_linq {
    pub(crate) use super::physical_policy;
    pub(crate) use crate::costs::{distinct_state, index_provider};
    pub(crate) use ::mini_linq::mini_linq as negation_hole;
    pub(crate) use ::mini_linq::mini_linq as recursion_hole;
    pub(crate) use ::mini_linq::{
        cq_to_ra, index_demands as choose_index_policy, select_structures, structures_to_rust,
    };
}

#[derive(Clone, Debug)]
pub struct Observation {
    pub query: &'static str,
    pub scale: usize,
    pub counts: Snapshot,
    pub rounds: usize,
    pub delta_rows: usize,
    pub negative_checks: usize,
    pub elapsed: Duration,
}

pub fn run(scale: usize) -> Vec<Observation> {
    let seed = [(0,)];
    let edges: Vec<_> = (0..scale as i32).map(|i| (i, i + 1)).collect();
    let reachable: BTreeSet<_> = (1..=scale as i32).map(|i| (i,)).collect();
    let rounds = Cell::new(0);
    let delta_rows = Cell::new(0);
    counters::reset();
    let start = Instant::now();
    let actual: BTreeSet<_> = ::cq_ext_recursion::recursion! {
        observe = (|_: usize, rows: usize| { rounds.set(rounds.get()+1); delta_rows.set(delta_rows.get()+rows); });
        struct Path; relation seed(x:i32); relation edges(x:i32,y:i32); relation reach(x:i32);
        reach(*y) :- seed(x), edges(x,y);
        reach(*z) :- reach(y), edges(y,z);
    }.collect();
    let elapsed = start.elapsed();
    let counts = counters::snapshot();
    assert_eq!(actual, reachable);
    assert_eq!(rounds.get(), scale);
    assert_eq!(delta_rows.get(), scale);
    let recursion = Observation {
        query: "recursion-path",
        scale,
        counts,
        rounds: rounds.get(),
        delta_rows: delta_rows.get(),
        negative_checks: 0,
        elapsed,
    };

    let candidates: Vec<_> = (0..scale as i32).map(|i| (i,)).collect();
    let blocked: Vec<_> = (0..scale as i32)
        .filter(|i| i % 2 == 0)
        .map(|i| (i, 99))
        .collect();
    let allowed: BTreeSet<_> = (0..scale as i32)
        .filter(|i| i % 2 != 0)
        .map(|i| (i,))
        .collect();
    let visits = Cell::new(0);
    let observe = |_: &i32| {
        visits.set(visits.get() + 1);
        true
    };
    counters::reset();
    let start = Instant::now();
    let actual: BTreeSet<_> = ::cq_ext_negation::negation! {
        struct Anti; relation candidates(x:i32); relation blocked(x:i32,y:i32); relation answer(x:i32);
        answer(*x) :- candidates(x), if observe(x), neg!{blocked(x,_)};
    }.collect();
    let elapsed = start.elapsed();
    let counts = counters::snapshot();
    assert_eq!(actual, allowed);
    assert_eq!(visits.get(), scale);
    let negation = Observation {
        query: "negation-key",
        scale,
        counts,
        rounds: 0,
        delta_rows: 0,
        negative_checks: visits.get(),
        elapsed,
    };
    vec![recursion, negation]
}
pub fn table(observations: &[Observation]) -> String {
    let mut output = String::from(
        "extension\tscale\thashes\tbuilds\tlookups\tcursor_steps\trows\tdistinct_insertions\trounds\tdelta_rows\tnegative_checks\twall_ms\n",
    );
    for o in observations {
        let c = o.counts;
        writeln!(
            output,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{:.3}",
            o.query,
            o.scale,
            c.hash_computations,
            c.index_builds,
            c.lookups,
            c.cursor_steps,
            c.rows_enumerated,
            c.distinct_insertions,
            o.rounds,
            o.delta_rows,
            o.negative_checks,
            o.elapsed.as_secs_f64() * 1000.
        )
        .unwrap();
    }
    output
}
