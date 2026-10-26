//! Executable answer conformance: ignore enumeration order, not multiplicity.
//!
//! `assert_answers_equal` is dependency-light and also serves external targets.

use std::collections::BTreeMap;
use std::fmt::Debug;

/// Compare answer rows after the answer map. Canonical outputs compare as sets;
/// other outputs compare as multisets. Report the first differing row and counts.
pub fn assert_answers_equal<Row: Ord + Debug>(
    reference_name: &str,
    reference: impl IntoIterator<Item = Row>,
    target_name: &str,
    actual: impl IntoIterator<Item = Row>,
    canonical: bool,
) {
    fn counts<Row: Ord>(
        rows: impl IntoIterator<Item = Row>,
        canonical: bool,
    ) -> BTreeMap<Row, usize> {
        let mut counts = BTreeMap::new();
        for row in rows {
            let count = counts.entry(row).or_default();
            if canonical {
                *count = 1;
            } else {
                *count += 1;
            }
        }
        counts
    }
    let reference = counts(reference, canonical);
    let actual = counts(actual, canonical);
    for row in reference.keys().chain(actual.keys()) {
        let expected = reference.get(row).copied().unwrap_or(0);
        let observed = actual.get(row).copied().unwrap_or(0);
        assert_eq!(
            expected, observed,
            "answer divergence: {reference_name} vs {target_name}; row {row:?}; \
             {reference_name} multiplicity {expected}, {target_name} multiplicity {observed}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::assert_answers_equal;

    #[test]
    fn order_is_not_observable() {
        assert_answers_equal(
            "pull",
            [vec![1], vec![2], vec![1]],
            "tasks",
            [vec![1], vec![1], vec![2]],
            false,
        );
    }

    #[test]
    fn canonical_answers_compare_as_sets() {
        assert_answers_equal("pull", [vec![1], vec![1]], "tasks", [vec![1]], true);
    }

    #[test]
    #[should_panic(expected = "answer divergence: pull vs tasks; row [1]")]
    fn noncanonical_multiplicity_is_observable() {
        assert_answers_equal("pull", [vec![1], vec![1]], "tasks", [vec![1]], false);
    }

    #[test]
    #[should_panic(expected = "answer divergence: pull vs tasks; row [2]")]
    fn additional_answer_is_observable() {
        assert_answers_equal("pull", [vec![1]], "tasks", [vec![1], vec![2]], true);
    }
}
