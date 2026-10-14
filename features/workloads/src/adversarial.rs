//! Seeded inputs whose size exposes one planner/storage decision at a time.
//! Complete tuples are distinct; projection intentionally drops their keys.
pub type Edge = (i32, i32);
pub const SEED: u64 = 0x6d69_6e69_6c69_6e71;

#[derive(Debug, PartialEq, Eq)]
pub struct Join {
    pub r: Vec<Edge>,
    pub s: Vec<Edge>,
    pub t: Vec<Edge>,
}

fn shuffled<T>(mut rows: Vec<T>, seed: u64) -> Vec<T> {
    // SplitMix64 plus Fisher-Yates: no dependency on a randomized host hasher.
    let mut random = cq_testkit::random::SplitMix64::new(seed);
    for i in (1..rows.len()).rev() {
        rows.swap(i, (random.next_u64() % (i as u64 + 1)) as usize);
    }
    rows
}
fn size(scale: usize) -> i32 {
    assert!(scale >= 8, "adversarial scales must be at least 8");
    i32::try_from(scale).expect("scale fits the query's i32 columns")
}

/// Symmetric fan/star skew plus a planted triangle, so correctness is nonempty.
/// A first-side candidate bucket has n rows where the other bucket has one.
pub fn triangle(scale: usize, seed: u64) -> Join {
    let n = size(scale);
    let mut fan: Vec<_> = (1..=n).flat_map(|i| [(0, i), (i, 0)]).collect();
    fan.extend([(1, 2), (2, 3), (1, 3)]);
    Join {
        r: shuffled(fan.clone(), seed),
        s: shuffled(fan.clone(), seed ^ 1),
        t: shuffled(fan, seed ^ 2),
    }
}

/// n*n dangling R/S bindings all fail at T; the planted path survives reduction.
pub fn chain(scale: usize, seed: u64) -> Join {
    let n = size(scale);
    let r = (1..=n).map(|i| (i, 0)).chain([(0, -1)]).collect();
    let s = (1..=n).map(|i| (0, i)).chain([(-1, 0)]).collect();
    Join {
        r: shuffled(r, seed),
        s: shuffled(s, seed ^ 1),
        t: vec![(0, 0)],
    }
}

/// Dropping the second column turns n distinct rows into n copies of (0,).
/// Without projection canon, those copies each enumerate n matching S rows.
pub fn projection(scale: usize, seed: u64) -> Join {
    let n = size(scale);
    Join {
        r: shuffled((0..n).map(|i| (0, i)).collect(), seed),
        s: shuffled((0..n).map(|i| (0, i)).collect(), seed ^ 1),
        t: Vec::new(),
    }
}

/// Four fixed leading-column matches amid a growing relation.
/// Opaque guards scan the host; map probes use keys 2..6 independently of n.
pub fn host_rows(scale: usize, seed: u64) -> Vec<Edge> {
    shuffled((0..size(scale)).map(|i| (i, i * 3)).collect(), seed)
}

/// Diagonal rows; Connected/Fine enumerates disconnected R/S prefixes before
/// T's connecting columns, while Elimination/Fine binds through T. Both guards
/// wait for all written relational constraints and visit n complete valuations;
/// storage enumeration exposes the quadratic versus linear order decision.
#[derive(Debug, PartialEq, Eq)]
pub struct Disconnected {
    pub r: Vec<Edge>,
    pub s: Vec<(i32,)>,
    pub t: Vec<(i32, i32, i32)>,
}
pub fn disconnected(scale: usize, seed: u64) -> Disconnected {
    let n = size(scale);
    Disconnected {
        r: shuffled((0..n).map(|i| (i, i)).collect(), seed),
        s: shuffled((0..n).map(|i| (i,)).collect(), seed ^ 1),
        t: shuffled((0..n).map(|i| (i, i, i)).collect(), seed ^ 2),
    }
}
