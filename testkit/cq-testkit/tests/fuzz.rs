use cq_testkit::fuzz::{Coverage, FEATURE_TAGS, generate};
use std::collections::BTreeSet;

#[test]
fn public_generator_and_printers_are_seeded_and_cover_all_tags_in_2048_seeds() {
    let mut coverage = Coverage::default();
    for seed in 0..2048 {
        let first = generate(seed);
        let second = generate(seed);
        assert_eq!(first, second);
        assert_eq!(first.cq(), second.cq());
        assert_eq!(first.written_cq(), second.written_cq());
        assert_eq!(first.sql(), second.sql());
        coverage.observe(&first);
    }
    assert_eq!(coverage.count("cases"), 2048);
    let advertised: BTreeSet<_> = FEATURE_TAGS.iter().copied().collect();
    let observed: BTreeSet<_> = coverage.counts().keys().map(String::as_str).collect();
    assert_eq!(advertised, observed, "tag inventory must be complete");
    for feature in FEATURE_TAGS {
        assert!(coverage.count(feature) > 30, "{feature}: {coverage}");
    }
    assert_eq!(coverage.count("never-observed"), 0);
}

#[test]
fn large_seeds_preserve_determinism_and_partial_lane() {
    for seed in [u32::MAX as u64 + 1, 1 << 63, u64::MAX - 1, u64::MAX] {
        let case = generate(seed);
        assert_eq!(case, generate(seed));
        assert_eq!(case.has_partial_terms(), seed & 1 == 0);
    }
}
