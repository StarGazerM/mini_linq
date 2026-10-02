//! One execution path shared by the gated growth tests and xtask bench.
//! All answers are checked against independent ordered-set relational oracles.
use crate::adversarial::{self, Edge, Join};
use crate::measurement::{self as counters, Snapshot};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

type Answers = BTreeSet<Vec<i32>>;

// The selected passes write Physical and Structures decisions before printing.
macro_rules! configuration {
    ($module:ident, { $($functions:tt)* }) => {
        mod $module {
            use super::{Answers, Edge};
            $($functions)*
        }
    };
}

configuration!(keep, {
    pub(super) fn triangle(r: &[Edge], s: &[Edge], t: &[Edge]) -> Answers {
        ::mini_linq::compile! {
            pipeline { structures = [cq_rewrites::retype(dedup => crate::measurement::Dedup<_>)]; }
            struct Triangle;
            relation r(a:i32,b:i32); relation s(b:i32,c:i32); relation t(a:i32,c:i32);
            answer(a,b,c) :- r(a,b),s(b,c),t(a,c);
        }
        .map(|(a, b, c)| vec![*a, *b, *c])
        .collect()
    }
    pub(super) fn chain(r: &[Edge], s: &[Edge], t: &[Edge]) -> Answers {
        ::mini_linq::compile! {
            pipeline { structures = [cq_rewrites::retype(dedup => crate::measurement::Dedup<_>)]; }
            struct Chain;
            relation r(a:i32,b:i32); relation s(b:i32,c:i32); relation t(c:i32,d:i32);
            answer(a,b,c,d) :- r(a,b),s(b,c),t(c,d);
        }
        .map(|(a, b, c, d)| vec![*a, *b, *c, *d])
        .collect()
    }
    pub(super) fn projection(r: &[Edge], s: &[Edge]) -> Answers {
        ::mini_linq::passes::index_demands! {
            pipeline { mini_linq::passes::lower, mini_linq::passes::select, cq_rewrites::retype(dedup => crate::measurement::Dedup<_>), mini_linq::passes::print_pull };
            relational {
                relation r(a:i32,dropped:i32); relation s(a:i32,b:i32);
                r0 = rename r { a -> a, dropped -> dropped };
                prefix = project r0 keep { a };
                s0 = rename s { a -> a, b -> b };
                joined = natural_join prefix with s0;
                result = project joined keep { b };
                output result as answer(b);
            }
        }.map(|(b,)| vec![*b]).collect()
    }
    pub(super) fn range_slice(r: &[Edge]) -> Answers {
        ::mini_linq::compile! {
            pipeline { structures = [cq_rewrites::retype(dedup => crate::measurement::Dedup<_>)]; }
            struct Range;
            relation r(a:i32,b:i32);
            answer(a,b) :- r(a,b), if * a >= 2, if * a < 6;
        }
        .map(|(a, b)| vec![*a, *b])
        .collect()
    }
    pub(super) fn range_map(r: &std::collections::BTreeMap<(i32,), Vec<(i32,)>>) -> Answers {
        ::mini_linq::compile! {
            pipeline { structures = [cq_rewrites::retype(dedup => crate::measurement::Dedup<_>)]; }
            struct Range;
            relation r(a:i32,b:i32);
            answer(a,b) :- r(a,b), if * a >= 2, if * a < 6;
        }
        .map(|(a, b)| vec![*a, *b])
        .collect()
    }
    pub(super) fn probe_slice(l: &[(i32,)], r: &[Edge]) -> Answers {
        ::mini_linq::compile! {
            pipeline { structures = [cq_rewrites::retype(dedup => crate::measurement::Dedup<_>)]; }
            struct Probe;
            relation l(a:i32); relation r(a:i32,b:i32);
            answer(a,b) :- l(a),r(a,b);
        }
        .map(|(a, b)| vec![*a, *b])
        .collect()
    }
    pub(super) fn probe_map(
        l: &[(i32,)],
        r: &crate::measurement::HashMap<(i32,), Vec<(i32,)>>,
    ) -> Answers {
        ::mini_linq::compile! {
            pipeline { structures = [cq_rewrites::retype(dedup => crate::measurement::Dedup<_>)]; }
            struct Probe;
            relation l(a:i32); relation r(a:i32,b:i32);
            answer(a,b) :- l(a),r(a,b);
        }
        .map(|(a, b)| vec![*a, *b])
        .collect()
    }
    pub(super) fn disconnected(
        r: &[Edge],
        s: &[(i32,)],
        t: &[(i32, i32, i32)],
    ) -> (Answers, usize) {
        let visits = std::cell::Cell::new(0);
        let observe = |_: &i32, _: &i32| {
            visits.set(visits.get() + 1);
            true
        };
        let answers = ::mini_linq::compile! {
            pipeline { plan = cq_planners::order(prefix connected, fine); structures = [cq_rewrites::retype(dedup => crate::measurement::Dedup<_>)]; }
            struct Disconnected;
            relation r(b:i32,a:i32); relation s(c:i32); relation t(b:i32,c:i32,k:i32);
            answer(k) :- r(b,a),s(c),t(b,c,k),if observe(b,c);
        }.map(|(k,)| vec![*k]).collect();
        (answers, visits.get())
    }
    pub(super) fn elimination(r: &[Edge], s: &[(i32,)], t: &[(i32, i32, i32)]) -> (Answers, usize) {
        let visits = std::cell::Cell::new(0);
        let observe = |_: &i32, _: &i32| {
            visits.set(visits.get() + 1);
            true
        };
        let answers = ::mini_linq::compile! {
            pipeline { plan = cq_planners::order(elimination, fine); structures = [cq_rewrites::retype(dedup => crate::measurement::Dedup<_>)]; }
            struct Disconnected;
            relation r(b:i32,a:i32); relation s(c:i32); relation t(b:i32,c:i32,k:i32);
            answer(k) :- r(b,a),s(c),t(b,c,k),if observe(b,c);
        }.map(|(k,)| vec![*k]).collect();
        (answers, visits.get())
    }
});

configuration!(canon, {
    pub(super) fn triangle(r: &[Edge], s: &[Edge], t: &[Edge]) -> Answers {
        ::mini_linq::compile! {
            pipeline { plan = cq_planners::order(written, fine); physical = [cq_rewrites::canon_prefixes]; structures = [cq_rewrites::retype(dedup => crate::measurement::Dedup<_>)]; }
            struct Triangle;
            relation r(a:i32,b:i32); relation s(b:i32,c:i32); relation t(a:i32,c:i32);
            answer(a,b,c) :- r(a,b),s(b,c),t(a,c);
        }.map(|(a,b,c)| vec![*a,*b,*c]).collect()
    }
    pub(super) fn projection(r: &[Edge], s: &[Edge]) -> Answers {
        ::mini_linq::passes::index_demands! {
            pipeline { mini_linq::passes::lower, cq_rewrites::canon_prefixes, mini_linq::passes::select, cq_rewrites::retype(dedup => crate::measurement::Dedup<_>), mini_linq::passes::print_pull };
            relational {
                relation r(a:i32,dropped:i32); relation s(a:i32,b:i32);
                r0 = rename r { a -> a, dropped -> dropped };
                prefix = project r0 keep { a };
                s0 = rename s { a -> a, b -> b };
                joined = natural_join prefix with s0;
                result = project joined keep { b };
                output result as answer(b);
            }
        }.map(|(b,)| vec![*b]).collect()
    }
});

configuration!(smallest, {
    pub(super) fn triangle(r: &[Edge], s: &[Edge], t: &[Edge]) -> Answers {
        ::mini_linq::compile! {
            pipeline { plan = cq_planners::order(written, fine); physical = [cq_rewrites::canon_prefixes, cq_rewrites::smallest_intersections]; structures = [cq_rewrites::retype(dedup => crate::measurement::Dedup<_>)]; }
            struct Triangle;
            relation r(a:i32,b:i32); relation s(b:i32,c:i32); relation t(a:i32,c:i32);
            answer(a,b,c) :- r(a,b),s(b,c),t(a,c);
        }.map(|(a,b,c)| vec![*a,*b,*c]).collect()
    }
});

configuration!(reduced, {
    pub(super) fn chain(r: &[Edge], s: &[Edge], t: &[Edge]) -> Answers {
        ::mini_linq::compile! {
            pipeline { plan = cq_planners::order(hypergraph); structures = [cq_rewrites::retype(dedup => crate::measurement::Dedup<_>)]; }
            struct Chain;
            relation r(a:i32,b:i32); relation s(b:i32,c:i32); relation t(c:i32,d:i32);
            answer(a,b,c,d) :- r(a,b),s(b,c),t(c,d);
        }.map(|(a,b,c,d)| vec![*a,*b,*c,*d]).collect()
    }
});

#[derive(Clone, Debug)]
pub struct Observation {
    pub query: &'static str,
    pub config: &'static str,
    pub scale: usize,
    pub counts: Snapshot,
    pub elapsed: Duration,
    /// Guard invocations after all written relational prerequisites.
    pub guard_visits: usize,
}

fn measure(
    query: &'static str,
    config: &'static str,
    scale: usize,
    oracle: &Answers,
    execute: impl FnOnce() -> (Answers, usize),
) -> Observation {
    counters::reset();
    let started = Instant::now();
    let (actual, guard_visits) = execute();
    let elapsed = started.elapsed();
    let counts = counters::snapshot();
    assert_eq!(&actual, oracle, "{query}/{config} at scale {scale}");
    Observation {
        query,
        config,
        scale,
        counts,
        elapsed,
        guard_visits,
    }
}

/// Independent natural join with ordered indexes, retaining all four columns.
fn chain_oracle(data: &Join) -> Answers {
    let mut s = BTreeMap::<_, Vec<_>>::new();
    let mut t = BTreeMap::<_, Vec<_>>::new();
    for &(b, c) in &data.s {
        s.entry(b).or_default().push(c);
    }
    for &(c, d) in &data.t {
        t.entry(c).or_default().push(d);
    }
    let mut answers = Answers::new();
    for &(a, b) in &data.r {
        for &c in s.get(&b).into_iter().flatten() {
            for &d in t.get(&c).into_iter().flatten() {
                answers.insert(vec![a, b, c, d]);
            }
        }
    }
    answers
}
fn triangle_oracle(data: &Join) -> Answers {
    let t: BTreeSet<_> = data.t.iter().copied().collect();
    let mut answers = Answers::new();
    for &(a, b) in &data.r {
        for &(sb, c) in &data.s {
            if b == sb && t.contains(&(a, c)) {
                answers.insert(vec![a, b, c]);
            }
        }
    }
    answers
}
fn projection_oracle(data: &Join) -> Answers {
    let keys: BTreeSet<_> = data.r.iter().map(|&(a, _)| a).collect();
    data.s
        .iter()
        .filter(|(a, _)| keys.contains(a))
        .map(|&(_, b)| vec![b])
        .collect()
}
fn disconnected_oracle(data: &adversarial::Disconnected) -> Answers {
    let r: BTreeSet<_> = data.r.iter().map(|&(b, _)| b).collect();
    let s: BTreeSet<_> = data.s.iter().map(|&(c,)| c).collect();
    data.t
        .iter()
        .filter(|(b, c, _)| r.contains(b) && s.contains(c))
        .map(|&(_, _, k)| vec![k])
        .collect()
}

/// Fixture and oracle setup happen outside measurement; every config starts a
/// fresh generated query, and its initialization is included in the counters.
pub fn run(scale: usize, seed: u64) -> Vec<Observation> {
    let mut results = Vec::new();
    let data = adversarial::triangle(scale, seed);
    let expected = triangle_oracle(&data);
    assert!(!expected.is_empty());
    for (config, execute) in [
        (
            "binary/first",
            keep::triangle as fn(&[Edge], &[Edge], &[Edge]) -> Answers,
        ),
        ("generic/first", canon::triangle),
        ("generic/smallest", smallest::triangle),
    ] {
        results.push(measure("triangle", config, scale, &expected, || {
            (execute(&data.r, &data.s, &data.t), 0)
        }));
    }
    let data = adversarial::chain(scale, seed);
    let expected = chain_oracle(&data);
    assert_eq!(expected.len(), 1);
    for (config, execute) in [
        (
            "none",
            keep::chain as fn(&[Edge], &[Edge], &[Edge]) -> Answers,
        ),
        ("yannakakis", reduced::chain),
    ] {
        results.push(measure("chain", config, scale, &expected, || {
            (execute(&data.r, &data.s, &data.t), 0)
        }));
    }
    let data = adversarial::projection(scale, seed);
    let expected = projection_oracle(&data);
    for (config, execute) in [
        ("keep", keep::projection as fn(&[Edge], &[Edge]) -> Answers),
        ("canon", canon::projection),
    ] {
        results.push(measure("projection", config, scale, &expected, || {
            (execute(&data.r, &data.s), 0)
        }));
    }
    let rows = adversarial::host_rows(scale, seed);
    let expected: Answers = rows
        .iter()
        .filter(|(a, _)| (2..6).contains(a))
        .map(|&(a, b)| vec![a, b])
        .collect();
    let ordered: BTreeMap<_, _> = rows.iter().map(|&(a, b)| ((a,), vec![(b,)])).collect();
    let hash: counters::HashMap<_, _> = rows.iter().map(|&(a, b)| ((a,), vec![(b,)])).collect();
    let probes = [(2,), (3,), (4,), (5,)];
    results.push(measure("range", "btree/direct", scale, &expected, || {
        (keep::range_map(&ordered), 0)
    }));
    results.push(measure("range", "slice/scan", scale, &expected, || {
        (keep::range_slice(&rows), 0)
    }));
    results.push(measure("probe", "map/direct", scale, &expected, || {
        (keep::probe_map(&probes, &hash), 0)
    }));
    results.push(measure("probe", "slice/build", scale, &expected, || {
        (keep::probe_slice(&probes, &rows), 0)
    }));
    let data = adversarial::disconnected(scale, seed);
    let expected = disconnected_oracle(&data);
    results.push(measure(
        "disconnected",
        "connected/fine",
        scale,
        &expected,
        || keep::disconnected(&data.r, &data.s, &data.t),
    ));
    results.push(measure(
        "disconnected",
        "elimination/fine",
        scale,
        &expected,
        || keep::elimination(&data.r, &data.s, &data.t),
    ));
    results
}

/// Rows at runtime storage boundaries distinguish O(n) from O(n*n) work;
/// point lookups distinguish O(1) from O(n). Relational ranges seek on the ordered host and scan the slice.
/// Multiplication avoids floats.
/// The slack admits fixed setup while separating asymptotic predictions (2/4,
/// or 1/2). Both Fine orders enumerate the disconnected query linearly;
/// their guards still run after every written relational prerequisite.
pub fn assert_growth(runs: &[Vec<Observation>]) {
    assert_growth_where(runs, |_| true);
}

/// Check the independent remaining predictions while the generated fan is blocked.
/// The complete suite above still requires every original fan assertion.
pub fn assert_non_fan_growth(runs: &[Vec<Observation>]) {
    assert_growth_where(runs, |observation| observation.query != "triangle");
}

fn assert_growth_where(runs: &[Vec<Observation>], include: impl Fn(&Observation) -> bool) {
    assert_eq!(runs.len(), 3);
    assert!(runs.iter().all(|observations| observations.len() == 13));
    for pair in runs.windows(2) {
        for (small, large) in pair[0]
            .iter()
            .zip(&pair[1])
            .filter(|(small, _)| include(small))
        {
            assert_eq!((small.query, small.config), (large.query, large.config));
            assert_eq!(large.scale, small.scale * 2);
            let (numerator, denominator, winner) = match (small.query, small.config) {
                ("triangle", "generic/smallest")
                | ("chain", "yannakakis")
                | ("projection", "canon")
                | ("disconnected", _) => (5, 2, true),
                ("probe", "map/direct") | ("range", "btree/direct") => (5, 4, true),
                ("range", _) | ("probe", _) => (17, 10, false),
                _ => (3, 1, false),
            };
            let a = small.counts.rows_enumerated;
            let b = large.counts.rows_enumerated;
            assert!(a > 0);
            if winner {
                assert!(
                    b * denominator < a * numerator,
                    "winner exceeds bound: {small:?} -> {large:?}"
                );
            } else {
                assert!(
                    b * denominator > a * numerator,
                    "loser below bound: {small:?} -> {large:?}"
                );
            }
        }
    }
    for observations in runs {
        let written = observations
            .iter()
            .find(|o| o.query == "disconnected" && o.config == "connected/fine")
            .unwrap();
        let elimination = observations
            .iter()
            .find(|o| o.query == "disconnected" && o.config == "elimination/fine")
            .unwrap();
        assert_eq!(written.guard_visits, written.scale);
        assert_eq!(elimination.guard_visits, elimination.scale);
        for observation in observations
            .iter()
            .filter(|o| o.query == "range" || o.query == "probe")
        {
            assert_eq!(
                observation.counts.index_builds,
                u64::from(observation.config == "slice/build")
            );
        }
    }
}

pub fn table(observations: &[Observation]) -> String {
    use std::fmt::Write;
    let mut output = format!(
        "query\tconfig\tscale\thashes\tbuilds\tlookups\tcursor_steps\trows\tdistinct_insertions\tguard_visits\t{}\n",
        if counters::INSTRUMENTED {
            "count_run_ms"
        } else {
            "timing_ms"
        },
    );
    for o in observations {
        let c = o.counts;
        writeln!(
            output,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{:.3}",
            o.query,
            o.config,
            o.scale,
            c.hash_computations,
            c.index_builds,
            c.lookups,
            c.cursor_steps,
            c.rows_enumerated,
            c.distinct_insertions,
            o.guard_visits,
            o.elapsed.as_secs_f64() * 1000.0
        )
        .unwrap();
    }
    output
}
