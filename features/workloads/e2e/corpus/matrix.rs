// Axes select pass pipelines, printer and native host representations.
// Positional advice is a separate axis, valid only for the slice host.
// Every pair is checked against the full product, including rejected merges.
pub type Config = [usize; 10];
pub const AXES: [&[&str]; 10] = [
    &[
        "written_coarse",
        "written_fine",
        "hypergraph",
        "prefix_connected_coarse",
        "prefix_connected_fine",
        "prefix_greedy_coarse",
        "prefix_greedy_fine",
        "elimination_coarse",
        "elimination_fine",
    ],
    &["probe", "merge"],
    &["keep", "canon"],
    &["first", "smallest"],
    &["slice", "hashmap", "btreemap"],
    &["hash_dedup", "ordered_dedup"],
    &["default", "sorted"],
    &["eager", "lazy"],
    &["pull", "push"],
    &["unadvised", "positional"],
];
pub const FULL: [&str; 4] = ["triangle", "oriented-chain", "claw", "retail_h03"];
/// Test crates the accepted configurations are dealt into (about 100 each).
pub const SHARDS: usize = 128;
const PREVIOUS_COVERING: [[usize; 7]; 27] = [
    [0, 0, 0, 1, 0, 0, 0],
    [0, 1, 1, 0, 1, 1, 1],
    [0, 0, 1, 1, 2, 0, 1],
    [1, 0, 1, 0, 0, 0, 1],
    [1, 1, 0, 1, 1, 1, 0],
    [1, 1, 0, 1, 2, 0, 0],
    [2, 0, 1, 1, 0, 0, 1],
    [2, 1, 0, 0, 1, 1, 0],
    [2, 1, 0, 0, 2, 1, 1],
    [3, 0, 1, 0, 0, 1, 1],
    [3, 1, 0, 1, 1, 0, 0],
    [3, 1, 0, 0, 2, 0, 0],
    [4, 0, 1, 0, 0, 0, 0],
    [4, 1, 0, 1, 1, 1, 1],
    [4, 0, 1, 1, 2, 0, 0],
    [5, 0, 0, 1, 0, 1, 1],
    [5, 1, 1, 0, 1, 0, 0],
    [5, 0, 1, 1, 2, 1, 0],
    [6, 0, 1, 1, 0, 0, 1],
    [6, 1, 0, 0, 1, 1, 0],
    [6, 1, 0, 1, 2, 0, 0],
    [7, 1, 1, 1, 0, 1, 1],
    [7, 0, 0, 0, 1, 0, 0],
    [7, 1, 0, 1, 2, 0, 1],
    [8, 0, 1, 0, 0, 1, 1],
    [8, 1, 0, 1, 1, 0, 0],
    [8, 0, 0, 1, 2, 1, 0],
];

pub fn product() -> Vec<Config> {
    let mut rows = vec![[0; 10]];
    for (axis, choices) in AXES.iter().enumerate() {
        rows = rows
            .into_iter()
            .flat_map(|row| {
                (0..choices.len()).map(move |value| {
                    let mut next = row;
                    next[axis] = value;
                    next
                })
            })
            .collect();
    }
    rows.into_iter()
        .filter(|row| row[4] == 0 || row[9] == 0)
        .collect()
}

pub fn configs(name: &str) -> Vec<Config> {
    if FULL.contains(&name) {
        product()
    } else {
        covering().to_vec()
    }
}

pub fn values(c: Config) -> Vec<&'static str> {
    AXES.iter().zip(c).map(|(axis, v)| axis[v]).collect()
}

pub fn verify_cover() {
    assert_eq!(product().len(), 4608);
    for c in product() {
        for a in 0..AXES.len() {
            for b in a + 1..AXES.len() {
                assert!(
                    covering().iter().any(|r| r[a] == c[a] && r[b] == c[b]),
                    "uncovered axes {a}/{b}: {c:?}"
                );
            }
        }
    }
}

// Preserve every previous pairwise row, then cover the added pass/printer axes.
// Greedy set cover is deterministic and checked against the complete product.
pub fn covering() -> &'static [Config] {
    static ROWS: std::sync::OnceLock<Vec<Config>> = std::sync::OnceLock::new();
    ROWS.get_or_init(|| {
        use std::collections::BTreeSet;
        let pairs = |row: Config| {
            (0..AXES.len())
                .flat_map(move |a| (a + 1..AXES.len()).map(move |b| (a, row[a], b, row[b])))
                .collect::<BTreeSet<_>>()
        };
        let all = product();
        let mut missing: BTreeSet<_> = all.iter().flat_map(|&r| pairs(r)).collect();
        let mut rows = Vec::new();
        for (i, old) in PREVIOUS_COVERING.iter().enumerate() {
            let mut row = [0; 10];
            row[..7].copy_from_slice(old);
            row[7] = i % 2;
            row[8] = (i / 2) % 2;
            for p in pairs(row) {
                missing.remove(&p);
            }
            rows.push(row);
        }
        while !missing.is_empty() {
            let row = *all
                .iter()
                .max_by_key(|&&r| pairs(r).intersection(&missing).count())
                .unwrap();
            for p in pairs(row) {
                missing.remove(&p);
            }
            rows.push(row);
        }
        rows
    })
}

/// The `Planner::ALL` position of a matrix planner name.
pub fn planner(c: Config) -> usize {
    cq_workloads::pipelines::Planner::ALL
        .iter()
        .position(|p| p.name() == AXES[0][c[0]])
        .expect("matrix planner names a Planner")
}

pub fn pipeline(c: Config) -> cq_workloads::pipelines::Pipeline {
    cq_workloads::pipelines::Pipeline {
        plan: planner(c),
        merge: c[1] == 1,
        canon: c[2] == 1,
        smallest: c[3] == 1,
        ordered_dedup: c[5] == 1,
        sorted_payload: c[6] == 1,
        lazy: c[7] == 1,
        push: c[8] == 1,
        positional: c[9] == 1,
    }
}
