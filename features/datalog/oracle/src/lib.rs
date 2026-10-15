//! Complete-answer differential tests against sequential Ascent.
//! `datalog-oracle golden --write` is the only golden regeneration command.
mod golden;
pub type Result<T, E = Box<dyn std::error::Error>> = std::result::Result<T, E>;
pub mod programs;
pub mod strings;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub type Rows = Vec<Vec<i32>>;
pub fn canonical(rows: Rows) -> Rows {
    rows.into_iter().collect::<BTreeSet<_>>().into_iter().collect()
}
#[derive(Clone, Debug, Default)]
pub struct Data {
    pub nodes: Vec<(i32,)>,
    pub edge: Vec<(i32, i32)>,
    pub seed: Vec<(i32,)>,
    pub extra: Vec<(i32,)>,
    pub address_of: Vec<(i32, i32)>,
    pub assign: Vec<(i32, i32)>,
    pub load: Vec<(i32, i32)>,
    pub store: Vec<(i32, i32)>,
    pub dereference: Vec<(i32, i32)>,
}

pub const SCENARIOS: &[&str] = &[
    "empty", "self_loop", "chain", "cycle", "complete", "duplicates", "disconnected", "random",
];
pub const RANDOM_SEEDS: u64 = 24;

// Fixed wrapping LCG, not a library RNG whose algorithm could change.
pub fn generate(scenario: &str, n: i32, mut state: u64) -> Data {
    assert!(n > 0);
    let mut d = Data::default();
    if scenario == "empty" { return d; }
    let n = if scenario == "self_loop" { 1 } else { n };
    d.nodes = (0..n).map(|x| (x,)).collect();
    d.seed = vec![(0,)];
    d.extra = vec![(n - 1,)];
    d.edge = match scenario {
        "self_loop" => vec![(0, 0)],
        "chain" | "duplicates" => (0..n-1).map(|x| (x, x+1)).collect(),
        "cycle" => (0..n).map(|x| (x, (x+1)%n)).collect(),
        "complete" => (0..n).flat_map(|x| (0..n).map(move |y| (x,y))).collect(),
        "disconnected" => (0..n-1).filter(|x| *x != n/2-1).map(|x| (x,x+1)).collect(),
        "random" => {
            let mut edges = Vec::new();
            for x in 0..n { for y in 0..n {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                if (state >> 32) % 5 == 0 { edges.push((x,y)); }
            }}
            edges
        }
        _ => panic!("unknown scenario: {scenario}"),
    };
    // Constraint relations preserve graph shape but occupy distinct role domains.
    // Identity address facts plus graph-shaped assignments exercise propagation;
    // loads/stores/dereferences include nonidentity facts, not empty placeholders.
    d.address_of = d.nodes.iter().map(|&(x,)| (x,x)).collect();
    d.assign = d.edge.clone();
    d.load = d.edge.iter().map(|&(x,y)| (y,x)).collect();
    d.store = d.edge.clone();
    d.dereference = d.load.clone();
    if scenario == "random" {
        d.address_of = random_relation(n, &mut state, 1, 5);
        d.assign = random_relation(n, &mut state, 1, 5);
        d.load = random_relation(n, &mut state, 1, 5);
        d.store = random_relation(n, &mut state, 1, 5);
        d.dereference = random_relation(n, &mut state, 1, 5);
    }
    if scenario == "duplicates" {
        d.edge.extend(d.edge.clone()); d.seed.extend(d.seed.clone());
        d.extra.extend(d.extra.clone()); d.nodes.extend(d.nodes.clone());
        d.address_of.extend(d.address_of.clone()); d.assign.extend(d.assign.clone());
        d.load.extend(d.load.clone()); d.store.extend(d.store.clone());
        d.dereference.extend(d.dereference.clone());
    }
    d
}

fn random_relation(n: i32, state: &mut u64, numerator: u64, denominator: u64) -> Vec<(i32, i32)> {
    let mut rows = Vec::new();
    for x in 0..n { for y in 0..n {
        *state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        if (*state >> 32) % denominator < numerator { rows.push((x, y)); }
    }}
    rows
}

/// Independent Bernoulli draws for each relation, in a fixed stream order.
pub fn random_data(n: i32, mut state: u64, numerator: u64, denominator: u64) -> Data {
    assert!(n > 0 && numerator <= denominator && denominator > 0);
    Data {
        nodes: (0..n).map(|x| (x,)).collect(),
        seed: vec![(0,)],
        extra: vec![(n - 1,)],
        edge: random_relation(n, &mut state, numerator, denominator),
        address_of: random_relation(n, &mut state, numerator, denominator),
        assign: random_relation(n, &mut state, numerator, denominator),
        load: random_relation(n, &mut state, numerator, denominator),
        store: random_relation(n, &mut state, numerator, denominator),
        dereference: random_relation(n, &mut state, numerator, denominator),
    }
}

pub struct Case {
    pub name: &'static str,
    pub arity: usize,
    pub mini: fn(&Data) -> (Rows, Duration),
    pub oracle: fn(&Data) -> (Rows, Duration),
}
macro_rules! cases {
    ($($name:ident : $arity:literal),* $(,)?) => {
        pub const CASES: &[Case] = &[$(Case {name: stringify!($name), arity: $arity,
            mini: programs::$name::mini, oracle: programs::$name::oracle}),*];
    }
}
cases! { reach:1, tc_left:2, tc_right:2, tc_nonlinear:2, same_generation:2,
    mutual:3, multiple_seeds:1, guard:1, unreachable:1, nonedges:2,
    recursive_upper:2, andersen:2, cspa:3 }

// Independent dense Boolean closure, not a join/delta implementation.
pub fn naive(case: &str, d: &Data) -> Option<Rows> {
    let n = d.nodes.iter().map(|&(x,)| x as usize + 1).max().unwrap_or(0);
    let mut tc = vec![vec![false; n]; n];
    for &(x,y) in &d.edge { tc[x as usize][y as usize] = true; }
    for k in 0..n { for x in 0..n { for y in 0..n {
        tc[x][y] |= tc[x][k] && tc[k][y];
    }}}
    match case {
        "reach" => Some((0..n).filter(|&y| d.seed.iter().any(|&(x,)| x as usize == y || tc[x as usize][y])).map(|y| vec![y as i32]).collect()),
        "tc_left" | "tc_right" | "tc_nonlinear" => Some((0..n).flat_map(|x| (0..n).filter_map({let tc=&tc; move |y| tc[x][y].then_some(vec![x as i32,y as i32])})).collect()),
        "same_generation" => {
            // Walk the product graph from sibling pairs using a work queue.
            let mut seen = BTreeSet::new();
            let mut pending = std::collections::VecDeque::new();
            for &(p,x) in &d.edge { for &(q,y) in &d.edge {
                if p == q && seen.insert((x,y)) { pending.push_back((x,y)); }
            }}
            while let Some((a,b)) = pending.pop_front() {
                for &(p,x) in &d.edge { if p != a {continue;} for &(q,y) in &d.edge {
                    if q == b && seen.insert((x,y)) { pending.push_back((x,y)); }
                }}
            }
            Some(seen.into_iter().map(|(x,y)| vec![x,y]).collect())
        }
        _ => None,
    }
}

pub fn golden_root() -> PathBuf { Path::new(env!("CARGO_MANIFEST_DIR")).join("golden") }
pub fn goldens(write: bool, mini: bool) -> Result<BTreeSet<PathBuf>> {
    let mut checked = BTreeSet::new();
    for c in CASES { for scenario in SCENARIOS {
        let data = generate(scenario, 6, 0x5eed);
        let rows = if mini { (c.mini)(&data).0 } else { (c.oracle)(&data).0 };
        let path = golden_root().join(c.name).join(format!("{scenario}.csv"));
        if write {
            std::fs::create_dir_all(path.parent().unwrap())?;
            std::fs::write(&path, golden::canonical_bytes(&rows, c.arity)?)?;
        } else { golden::compare(&path, &rows, c.arity)?; }
        println!("{}\t{}\t{}", c.name, scenario, rows.len());
        checked.insert(path);
    }}
    Ok(checked)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn differential_matrix() {
        for c in CASES {
            for scenario in SCENARIOS { check(c, scenario, 0x5eed); }
            for seed in 0..RANDOM_SEEDS { check(c, "random", seed); }
        }
    }
    fn check(c: &Case, scenario: &str, seed: u64) {
        let d = generate(scenario, 6, seed);
        check_data(c, &d, &format!("{scenario} / seed={seed}"));
    }
    fn check_data(c: &Case, d: &Data, context: &str) {
        let expected = (c.oracle)(d).0;
        assert_eq!((c.mini)(d).0, expected, "{} / {context}", c.name);
        if let Some(naive) = naive(c.name, d) {
            assert_eq!(expected, naive, "naive {} / {context}", c.name);
        }
    }
    #[test]
    fn larger_differential_matrix() {
        for c in CASES {
            for n in [20, 50] {
                for scenario in SCENARIOS {
                    let d = generate(scenario, n, 0x5eed);
                    check_data(c, &d, &format!("{scenario} / n={n}"));
                }
            }
            for n in [6, 20, 50] {
                for (num, den) in [(1, 5), (2, n as u64)] {
                    for seed in 0..2 {
                        let d = random_data(n, seed, num, den);
                        check_data(c, &d, &format!("random / n={n} / p={num}/{den} / seed={seed}"));
                    }
                }
            }
        }
        for n in [6, 20, 50] {
            for (num, den) in [(1, 5), (2, n as u64)] {
                for seed in 0..2 { strings::check(&random_data(n, seed, num, den)); }
            }
        }
        for n in [20, 50] {
            for scenario in SCENARIOS { strings::check(&generate(scenario, n, 0x5eed)); }
        }
    }
    #[test]
    fn checked_in_goldens() { goldens(false, true).unwrap(); }
    #[test]
    fn oracle_goldens() { goldens(false, false).unwrap(); }
    #[test]
    fn golden_inventory() {
        let checked = goldens(false, false).unwrap();
        assert_eq!(golden::inventory(&golden_root()).unwrap(), checked);
    }
    #[test]
    fn owned_strings() {
        for scenario in SCENARIOS {
            strings::check(&generate(scenario, 6, 0x5eed));
        }
        for seed in 0..RANDOM_SEEDS { strings::check(&generate("random", 6, seed)); }
    }
}
