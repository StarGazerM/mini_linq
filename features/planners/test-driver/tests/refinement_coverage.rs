//! Model-only coverage of the exact seed registry emitted beside compiled shards.
use cq_testkit::fuzz;

include!(concat!(env!("OUT_DIR"), "/refinement_seeds.rs"));

#[test]
fn shards_cover_exactly_the_original_200_rust_seeds() {
    assert_eq!(SHARD_SEEDS.len(), 8);
    let mut seeds = Vec::new();
    let mut coverage = fuzz::Coverage::default();
    for (shard, shard_seeds) in SHARD_SEEDS.iter().enumerate() {
        assert_eq!(shard_seeds.len(), 25);
        for &seed in *shard_seeds {
            assert_eq!(seed % 2, 0);
            assert_eq!((seed / 2) as usize % 8, shard);
            seeds.push(seed);
            coverage.observe(&fuzz::generate(seed));
        }
    }
    seeds.sort_unstable();
    // Compare the multiset, not just its union: duplicates and omissions both fail.
    assert_eq!(seeds, (0..200).map(|ordinal| ordinal * 2).collect::<Vec<_>>());
    assert_eq!(coverage.count("cases"), 200);
    assert_eq!(coverage.count("rust-term"), 200);
    for feature in [
        "rust-divide",
        "rust-remainder",
        "rust-bounded-add",
        "rust-bounded-subtract",
        "rust-bounded-multiply",
        "rust-unwrap",
        "rust-guard",
        "rust-let",
        "rust-argument",
        "rust-head",
        "relational-constraint",
        "relational-parameter",
        "empty-used-relation",
        "duplicate-rows",
    ] {
        assert!(coverage.count(feature) > 0, "missing feature {feature}");
    }
}
