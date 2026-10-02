// Axes: planner, join, project, intersect, host, distinct, index provider.
// Nine planner/split choices x three hosts requires at least 27 pairwise rows.
// Every pair is checked against the full product, including rejected merges.
pub type Config = [usize; 7];
pub const AXES: [&[&str]; 7] = [
    &[
        "cq_to_ra",
        "cq_to_ra_generic",
        "cq_to_ra_hypergraph",
        "cq_to_ra_prefix_connected",
        "cq_to_ra_prefix_connected_fine",
        "cq_to_ra_prefix_greedy",
        "cq_to_ra_prefix_greedy_fine",
        "cq_to_ra_elimination",
        "cq_to_ra_elimination_fine",
    ],
    &["probe", "merge"],
    &["keep", "canon"],
    &["first", "smallest"],
    &["slice", "hashmap", "btreemap"],
    &["hash_distinct", "ordered_distinct"],
    &["default", "custom"],
];
pub const FULL: [&str; 4] = ["triangle", "oriented-chain", "claw", "retail_h03"];
pub const COVERING: [Config; 27] = [
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
    let mut rows = vec![[0; 7]];
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
    rows
}

pub fn configs(name: &str) -> Vec<Config> {
    if FULL.contains(&name) {
        product()
    } else {
        COVERING.to_vec()
    }
}

pub fn values(c: Config) -> Vec<&'static str> {
    AXES.iter().zip(c).map(|(axis, v)| axis[v]).collect()
}

pub fn verify_cover() {
    assert_eq!(product().len(), 864);
    for c in product() {
        for a in 0..AXES.len() {
            for b in a + 1..AXES.len() {
                assert!(
                    COVERING.iter().any(|r| r[a] == c[a] && r[b] == c[b]),
                    "uncovered axes {a}/{b}: {c:?}"
                );
            }
        }
    }
}

pub fn planner(p: usize) -> cq_order::Options {
    use cq_order::{Heuristic as H, Ranking as R, Split as S};
    let (heuristic, split) = match p {
        0 | 2 => (H::Written, S::Coarse),
        1 => (H::Written, S::Fine),
        3 => (H::Prefix(R::Connected), S::Coarse),
        4 => (H::Prefix(R::Connected), S::Fine),
        5 => (H::Prefix(R::GreedyWidth), S::Coarse),
        6 => (H::Prefix(R::GreedyWidth), S::Fine),
        7 => (H::Elimination, S::Coarse),
        8 => (H::Elimination, S::Fine),
        _ => panic!("unknown planner {p}"),
    };
    cq_order::Options { heuristic, split }
}
