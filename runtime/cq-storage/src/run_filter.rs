//! Allocation-free exact point and interval filters over published run keys.

/// Borrow the existing sorted key arrays, without merging runs or reading items.
/// Each slice must be ordered by the projection selected for a filter view.
/// Implementations must expose every represented key and no additional keys.
pub trait RunFilterSource {
    type Key;

    fn key_runs(&self) -> impl Iterator<Item = &[Self::Key]>;
}

impl<K> RunFilterSource for [K] {
    type Key = K;

    fn key_runs(&self) -> impl Iterator<Item = &[K]> {
        std::iter::once(self)
    }
}

// DESIGN run-filter-publication
/// Formal rule RT-RunFilter:
/// A borrowed, ordered scalar projection of completed immutable run storage.
///
/// `project` must preserve nondecreasing order within every source key slice.
/// A scalar key or a leading lexicographic component satisfies this law; an
/// arbitrary nonleading tuple component does not. The selected representation
/// must establish this premise before constructing a view. No validation scan,
/// new key collection, bitmap initialization, or payload materialization occurs.
///
/// Implements `ReadPredicate` directly to retain exact interval pruning without
/// overlapping the blanket `ExactMembership` to `ConservativeMembership` impl.
pub struct RunFilterView<'a, S: RunFilterSource + ?Sized, K: ?Sized> {
    source: &'a S,
    project: fn(&S::Key) -> &K,
}

impl<'a, S: RunFilterSource + ?Sized, K: Ord + ?Sized> RunFilterView<'a, S, K> {
    pub fn new(source: &'a S, project: fn(&S::Key) -> &K) -> Self {
        Self { source, project }
    }

    /// Exact membership in the projected scalar key domain.
    #[inline]
    pub fn test(&self, key: &K) -> bool {
        crate::count!(lookups);
        self.source.key_runs().any(|keys| {
            let slot = keys.partition_point(|stored| (self.project)(stored) < key);
            keys.get(slot).is_some_and(|stored| (self.project)(stored) == key)
        })
    }

    /// Exact existence of a projected key in the closed interval.
    #[inline]
    pub fn any_in(&self, lo: &K, hi: &K) -> bool {
        crate::count!(lookups);
        lo <= hi && self.source.key_runs().any(|keys| {
            let slot = keys.partition_point(|stored| (self.project)(stored) < lo);
            keys.get(slot).is_some_and(|stored| (self.project)(stored) <= hi)
        })
    }
}

impl<S: RunFilterSource + ?Sized, K: Ord + ?Sized, Q: std::borrow::Borrow<K>>
    crate::ReadPredicate<Q> for RunFilterView<'_, S, K>
{
    fn test(&self, key: &Q) -> bool {
        self.test(key.borrow())
    }

    fn any_in(&self, lo: &Q, hi: &Q) -> bool {
        self.any_in(lo.borrow(), hi.borrow())
    }
}

/// Optional borrowed run predicate at the recorded activation boundary.
/// An unavailable or abandoned view is identity, including for block pruning.
pub struct RunFilterPredicate<'a, P: ?Sized>(pub Option<&'a P>);

impl<K, P: crate::ReadPredicate<K> + ?Sized> crate::ReadPredicate<K>
    for RunFilterPredicate<'_, P>
{
    #[inline]
    fn is_identity(&self) -> bool {
        self.0.is_none_or(|filter| filter.is_identity())
    }

    #[inline]
    fn test(&self, key: &K) -> bool {
        let Some(filter) = self.0 else {
            return true;
        };
        crate::count!(join_filter_tested);
        let keep = filter.test(key);
        if keep {
            crate::count!(join_filter_passed);
        }
        keep
    }

    fn any_in(&self, lo: &K, hi: &K) -> bool {
        self.0.is_none_or(|filter| filter.any_in(lo, hi))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NoDense, RadixRun, SortedRun};

    #[test]
    fn nonclone_borrowed_scalar_and_unsized_keys_need_no_owned_projection() {
        #[derive(Eq, PartialEq, Ord, PartialOrd)]
        struct Key(i64);
        let keys = [Key(i64::MIN), Key(7), Key(i64::MAX)];
        let run = SortedRun::from_pairs(keys.iter().map(|key| ((key,), ())), NoDense);
        let view = RunFilterView::new(&run, |key| key.0);
        assert!(crate::ReadPredicate::test(&view, &Key(7)));
        assert!(!crate::ReadPredicate::test(&view, &Key(8)));
        assert!(crate::ReadPredicate::any_in(&view, &Key(i64::MIN), &Key(i64::MIN)));
        assert!(!crate::ReadPredicate::any_in(&view, &Key(8), &Key(9)));
        let owned = [String::from("alpha"), String::from("omega")];
        let view = RunFilterView::new(owned.as_slice(), |key| key.as_str());
        assert!(view.test("omega"));
        assert!(!view.any_in("beta", "delta"));
    }

    #[test]
    fn optional_run_predicate_preserves_identity_and_exact_intervals() {
        use crate::ReadPredicate;
        let run = SortedRun::from_pairs([((1, ), ()), ((5, ), ())], NoDense);
        let view = RunFilterView::new(&run, |key| &key.0);
        let mut predicate = RunFilterPredicate(Some(&view));
        assert!(!ReadPredicate::<i32>::is_identity(&predicate));
        assert!(predicate.test(&1));
        assert!(!predicate.test(&2));
        assert!(!predicate.any_in(&2, &4));
        assert!(predicate.any_in(&4, &5));
        predicate.0 = None;
        assert!(ReadPredicate::<i32>::is_identity(&predicate));
        assert!(predicate.test(&2));
        assert!(predicate.any_in(&2, &4));
        let identity = RunFilterPredicate(Some(&crate::NoJoinFilter));
        assert!(<RunFilterPredicate<'_, _> as ReadPredicate<i32>>::is_identity(&identity));
    }

    #[test]
    fn leading_projection_handles_duplicate_scalars_and_nonmonotone_other_columns() {
        let run = SortedRun::from_pairs(
            [((1, 90), ()), ((1, 100), ()), ((5, -100), ()), ((9, 0), ())],
            NoDense,
        );
        let view = RunFilterView::new(&run, |key| &key.0);
        assert_eq!(run.stat_keys(), Some(4));
        for key in 0..11 {
            assert_eq!(view.test(&key), [1, 5, 9].contains(&key));
            for hi in 0..11 {
                assert_eq!(view.any_in(&key, &hi), [1, 5, 9].iter().any(|k| key <= *k && *k <= hi));
            }
        }
    }

    #[test]
    fn exact_filters_match_occurrence_oracle_for_empty_extreme_and_skewed_inputs() {
        for seed in 0_u64..32 {
            let mut input: Vec<_> = (0..seed * 7)
                .map(|i| (((i.wrapping_mul(6364136223846793005) ^ seed) % 31) as i64 - 15, i))
                .collect();
            if seed != 0 {
                input.extend([(i64::MIN, 0), (i64::MAX, 0)]);
                input.extend((0..seed * 9).map(|i| (3, i)));
            }
            let sorted_len = input.len();
            let sorted = SortedRun::from_pairs(input.iter().map(|&(k, v)| ((k,), v)), NoDense);
            let mut radix = RadixRun::from_pairs(input.iter().map(|&(k, v)| ((k,), v)));
            if seed != 0 {
                radix.merge([((3,), 0), ((19,), 0)]);
                input.extend([(3, 0), (19, 0)]);
            }
            let sorted_view = RunFilterView::new(&sorted, |key| &key.0);
            let radix_view = RunFilterView::new(&radix, |key| &key.0);
            let probes: Vec<_> = [i64::MIN, i64::MAX].into_iter().chain(-20..=20).collect();
            for &lo in &probes {
                assert_eq!(radix_view.test(&lo), input.iter().any(|&(key, _)| key == lo));
                assert_eq!(sorted_view.test(&lo), input[..sorted_len].iter().any(|&(key, _)| key == lo));
                for &hi in &probes {
                    assert_eq!(radix_view.any_in(&lo, &hi), input.iter().any(|&(key, _)| lo <= key && key <= hi));
                    assert_eq!(sorted_view.any_in(&lo, &hi), input[..sorted_len].iter().any(|&(key, _)| lo <= key && key <= hi));
                }
            }
        }
    }

    #[test]
    fn selected_sidecars_observe_source_occurrences_during_one_exact_build() {
        use crate::BuildWithSidecar;
        struct Observed<'a> {
            consumed: &'a std::cell::Cell<usize>,
            seen: usize,
        }
        impl crate::SidecarBuilder<(i64,)> for Observed<'_> {
            type Output = usize;

            fn observe(&mut self, _key: &(i64,)) {
                self.seen += 1;
                assert_eq!(self.consumed.get(), self.seen);
            }

            fn finish(self) -> usize {
                self.seen
            }
        }
        let input = [((7, ), 1), ((3, ), 2), ((7, ), 1), ((i64::MIN, ), 0)];
        let consumed = std::cell::Cell::new(0);
        let source = input.into_iter().inspect(|_| consumed.set(consumed.get() + 1));
        let (sorted, seen) = SortedRun::<_, _, NoDense>::build_with_sidecar::<true, true>(
            source, Observed { consumed: &consumed, seen: 0 },
        );
        assert_eq!(seen, input.len());
        assert_eq!(sorted.stat_rows(), Some(4));
        assert_eq!(sorted.stat_keys(), Some(3));
        assert_eq!(sorted.stat_max_group(), Some(2));
        consumed.set(0);
        let source = input.into_iter().inspect(|_| consumed.set(consumed.get() + 1));
        let (radix, seen) = RadixRun::<_, _>::build_with_sidecar::<true, true>(
            source, Observed { consumed: &consumed, seen: 0 },
        );
        assert_eq!(seen, input.len());
        assert_eq!(radix.stat_rows(), Some(4));
        assert_eq!(radix.stat_keys(), Some(3));
        assert_eq!(radix.stat_max_group(), Some(2));
    }

    #[test]
    fn sidecars_and_borrowed_filters_do_not_require_maximum_group_statistics() {
        use crate::{BuildWithSidecar, IndexRead};

        struct SeenKeys(Vec<i64>);
        impl crate::SidecarBuilder<i64> for SeenKeys {
            type Output = Vec<i64>;

            fn observe(&mut self, key: &i64) {
                self.0.push(*key);
            }

            fn finish(self) -> Self::Output {
                self.0
            }
        }

        let input = [(7, 1), (3, 2), (7, 1), (i64::MIN, 0)];
        let (sorted, sorted_keys) = SortedRun::<_, _>::build_with_sidecar::<true, false>(
            input.into_iter(), SeenKeys(Vec::new()),
        );
        let (radix, radix_keys) = RadixRun::<_, _>::build_with_sidecar::<true, false>(
            input.into_iter(), SeenKeys(Vec::new()),
        );
        let expected: Vec<_> = input.iter().map(|pair| pair.0).collect();
        assert_eq!(sorted_keys, expected);
        assert_eq!(radix_keys, expected);
        assert_eq!(sorted.stat_rows(), Some(4));
        assert_eq!(radix.stat_rows(), Some(4));
        assert_eq!(sorted.stat_keys(), Some(3));
        assert_eq!(radix.stat_keys(), Some(3));
        assert_eq!((&sorted).lookup(&7).collect::<Vec<_>>(), [1, 1]);
        assert_eq!((&radix).lookup(&7).collect::<Vec<_>>(), [1, 1]);
        let sorted_view = RunFilterView::new(&sorted, |key| key);
        let radix_view = RunFilterView::new(&radix, |key| key);
        for lo in [i64::MIN, 2, 3, 4, 7, 8, i64::MAX] {
            assert_eq!(sorted_view.test(&lo), input.iter().any(|pair| pair.0 == lo));
            assert_eq!(radix_view.test(&lo), input.iter().any(|pair| pair.0 == lo));
            assert_eq!(sorted_view.any_in(&lo, &7), input.iter().any(|pair| lo <= pair.0 && pair.0 <= 7));
            assert_eq!(radix_view.any_in(&lo, &7), input.iter().any(|pair| lo <= pair.0 && pair.0 <= 7));
        }
        assert_eq!(sorted.stat_max_group(), None);
        assert_eq!(radix.stat_max_group(), None);
    }

    #[cfg(feature = "counters")]
    #[test]
    fn borrowed_filters_build_once_and_never_collect_keys_or_copy_payloads() {
        crate::counters::reset();
        let run = SortedRun::from_pairs((0..128).map(|key| ((key,), key)), NoDense);
        let built = crate::counters::snapshot();
        let view = RunFilterView::new(&run, |key| &key.0);
        for key in -1..130 {
            assert_eq!(view.test(&key), (0..128).contains(&key));
            let _ = view.any_in(&key, &key);
        }
        let read = crate::counters::snapshot();
        assert_eq!(built.index_builds, 1);
        assert_eq!(read.index_builds, built.index_builds);
        assert_eq!(read.rows_copied, built.rows_copied);
        assert_eq!(read.rows_enumerated, built.rows_enumerated);
        assert_eq!(read.cursor_steps, built.cursor_steps);
        assert_eq!(read.distinct_insertions, built.distinct_insertions);
    }
}
