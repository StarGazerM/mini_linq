//! Payload-exact clustered runs. Binary search is the default directory;
//! a total, injective, order-preserving `KeyCodec` enables bitmap rank when
//! the checked inclusive span is at most 1,048,576 bits. On 64-bit hosts the
//! bitmap and per-word usize prefixes together use at most 256 KiB.
//! No occupancy assumption is made: every rank lookup first tests presence.

use crate::{Build, ExactMembership, IndexRead, KeyCodec, NoDense, rows::Rows};

const MAX_RANK_BITS: usize = 1 << 20;

struct RankDirectory {
    min: u128,
    span: usize,
    words: Vec<u64>,
    prefixes: Vec<usize>,
}

/// A stable sorted representation of `(key, item)` occurrences.
/// Keys require a total order consistent with Leibniz equality. Codecs obey
/// `KeyCodec`'s injectivity/order premises; `NoDense` uses binary search only.
/// Lookup handles borrow this value for their host lifetime, not the lifetime
/// of a temporary reference passed to `IndexRead::lookup`.
pub struct SortedRun<K, V, C = NoDense> {
    keys: Vec<K>,
    offsets: Vec<usize>,
    items: Vec<V>,
    rank: Option<RankDirectory>,
    codec: C,
}

impl<K: Ord + Copy, V: Copy, C: KeyCodec<K>> SortedRun<K, V, C> {
    /// Formal rule STR-SortedRun:
    /// Stable sorting preserves each key's input occurrence order and its
    /// multiplicity. Unique keys and adjacent offsets partition all items;
    /// presence plus rank selects precisely the same range as binary search.
    pub fn from_pairs(source: impl IntoIterator<Item = (K, V)>, codec: C) -> Self {
        crate::count!(index_builds);
        let mut pairs: Vec<_> = source.into_iter().collect();
        pairs.sort_by_key(|pair| pair.0);
        let distinct =
            usize::from(!pairs.is_empty()) + pairs.windows(2).filter(|p| p[0].0 != p[1].0).count();
        let mut keys = Vec::with_capacity(distinct);
        let mut offsets = Vec::with_capacity(distinct + 1);
        let mut items = Vec::with_capacity(pairs.len());
        for (key, item) in pairs {
            if keys.last() != Some(&key) {
                keys.push(key);
                offsets.push(items.len());
                crate::count!(distinct_insertions);
            }
            items.push(item);
            crate::count!(rows_copied);
        }
        offsets.push(items.len());
        #[cfg(feature = "counters")]
        crate::counters::build_cardinality(keys.len());
        let rank = Self::rank_directory(&keys, codec);
        Self {
            keys,
            offsets,
            items,
            rank,
            codec,
        }
    }

    /// Number of distinct stored keys, also used by recorded join filters.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Borrow the unique keys in increasing order without enumerating payloads.
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.keys.iter()
    }

    fn rank_directory(keys: &[K], codec: C) -> Option<RankDirectory> {
        if !codec.total() {
            return None;
        }
        let min = codec.ordinal(keys.first()?)?;
        let max = codec.ordinal(keys.last()?)?;
        let span = max.checked_sub(min)?.checked_add(1)?;
        if span > MAX_RANK_BITS as u128 {
            return None;
        }
        let span = span as usize;
        let mut words = vec![0u64; span.div_ceil(64)];
        for key in keys {
            let offset = usize::try_from(codec.ordinal(key)?.checked_sub(min)?).ok()?;
            if offset >= span {
                return None;
            }
            words[offset / 64] |= 1 << (offset % 64);
        }
        let mut count = 0;
        let prefixes = words
            .iter()
            .map(|word| {
                let prefix = count;
                count += word.count_ones() as usize;
                prefix
            })
            .collect();
        Some(RankDirectory {
            min,
            span,
            words,
            prefixes,
        })
    }

    fn slot(&self, key: &K) -> Option<usize> {
        if let Some(rank) = &self.rank {
            let offset = usize::try_from(self.codec.ordinal(key)?.checked_sub(rank.min)?).ok()?;
            if offset >= rank.span {
                return None;
            }
            let word = rank.words[offset / 64];
            let bit = 1u64 << (offset % 64);
            if word & bit == 0 {
                return None;
            }
            Some(rank.prefixes[offset / 64] + (word & (bit - 1)).count_ones() as usize)
        } else {
            self.keys.binary_search(key).ok()
        }
    }
}

impl<K: Ord + Copy, V: Copy, C: KeyCodec<K> + Default, I: Iterator<Item = (K, V)>> Build<I>
    for SortedRun<K, V, C>
{
    fn build(source: I) -> Self {
        Self::from_pairs(source, C::default())
    }
}

/// Copying a cursor snapshots its position without allocating or copying rows.
#[derive(Clone, Copy)]
pub struct RunCursor<'a, V> {
    items: &'a [V],
    position: usize,
}

impl<V: Copy> Iterator for RunCursor<'_, V> {
    type Item = V;
    fn next(&mut self) -> Option<V> {
        crate::count!(cursor_steps);
        let item = self.items.get(self.position).copied();
        if item.is_some() {
            self.position += 1;
            crate::count!(rows_enumerated);
        }
        item
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.items.len() - self.position;
        (remaining, Some(remaining))
    }
}
impl<V: Copy> ExactSizeIterator for RunCursor<'_, V> {}
impl<V: Copy> std::iter::FusedIterator for RunCursor<'_, V> {}

impl<'a, K: Ord + Copy, V: Copy, C: KeyCodec<K>> IndexRead for &'a SortedRun<K, V, C> {
    type Key = K;
    type Item = V;
    type Cursor = RunCursor<'a, V>;
    fn lookup(&self, key: &K) -> Self::Cursor {
        crate::count!(lookups);
        let items = self.slot(key).map_or(&[][..], |slot| {
            &self.items[self.offsets[slot]..self.offsets[slot + 1]]
        });
        RunCursor { items, position: 0 }
    }
}

impl<K: Ord + Copy, V: Copy, C: KeyCodec<K>> ExactMembership<K> for SortedRun<K, V, C> {
    fn contains(&self, key: &K) -> bool {
        crate::count!(lookups);
        self.slot(key).is_some()
    }
}

impl<K: Copy, V: Copy, C> Rows for &SortedRun<K, V, C> {
    type Row = (K, V);
    fn rows(self) -> impl Iterator<Item = Self::Row> {
        self.keys.iter().enumerate().flat_map(move |(slot, &key)| {
            self.items[self.offsets[slot]..self.offsets[slot + 1]]
                .iter()
                .map(move |&item| (key, item))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::I64Codec;

    #[test]
    fn rank_boundaries_holes_and_budget() {
        for input in [
            vec![],
            vec![i64::MIN],
            vec![i64::MAX],
            vec![i64::MIN, i64::MAX],
            vec![-65, -64, -1, 0, 1, 63, 64, 65],
            vec![-7, MAX_RANK_BITS as i64 - 8],
            vec![-7, MAX_RANK_BITS as i64 - 7],
        ] {
            let run = SortedRun::from_pairs(input.iter().map(|&k| (k, k)), I64Codec);
            let span = input
                .first()
                .zip(input.last())
                .map(|(&min, &max)| max as i128 - min as i128 + 1);
            assert_eq!(
                run.rank.is_some(),
                span.is_some_and(|n| n <= MAX_RANK_BITS as i128)
            );
            for key in input.iter().copied().chain([
                i64::MIN,
                i64::MAX,
                -66,
                -65,
                -64,
                -63,
                -8,
                -7,
                -6,
                -1,
                0,
                1,
                2,
                62,
                63,
                64,
                65,
                66,
                MAX_RANK_BITS as i64 - 9,
                MAX_RANK_BITS as i64 - 8,
                MAX_RANK_BITS as i64 - 7,
            ]) {
                assert_eq!(
                    (&run).lookup(&key).collect::<Vec<_>>(),
                    input
                        .iter()
                        .copied()
                        .filter(|&k| k == key)
                        .collect::<Vec<_>>()
                );
            }
        }
    }

    #[test]
    fn borrowed_integer_keys_and_partial_codecs() {
        use crate::BorrowedI64TupleCodec;
        let keys = [-65i64, -1, 0, 65];
        let run = SortedRun::from_pairs(keys.iter().map(|k| ((k,), *k)), BorrowedI64TupleCodec);
        assert!(run.rank.is_some());
        assert_eq!((&run).lookup(&(&-1,)).collect::<Vec<_>>(), [-1]);
        assert!(!run.contains(&(&1,)));
        let run = SortedRun::from_pairs([((&-1i32,), 'a'), ((&65,), 'b')], BorrowedI64TupleCodec);
        assert!(run.rank.is_some());
        assert_eq!((&run).lookup(&(&65,)).collect::<Vec<_>>(), ['b']);
        #[derive(Clone, Copy)]
        struct Partial;
        impl KeyCodec<i64> for Partial {
            fn ordinal(self, key: &i64) -> Option<u128> {
                (*key >= 0).then_some(*key as u128)
            }
        }
        let run = SortedRun::from_pairs([(-1, 'a'), (1, 'b')], Partial);
        assert!(run.rank.is_none());
        assert_eq!((&run).lookup(&-1).collect::<Vec<_>>(), ['a']);
    }

    #[test]
    fn rows_preserve_bag_and_stable_order_and_cursors_outlive_handles() {
        let pairs = [(2, 'a'), (1, 'z'), (2, 'b'), (1, 'z'), (2, 'a')];
        let run = SortedRun::<_, _>::build(pairs.into_iter());
        assert!(run.rank.is_none());
        assert_eq!(
            (&run).rows().collect::<Vec<_>>(),
            [(1, 'z'), (1, 'z'), (2, 'a'), (2, 'b'), (2, 'a')]
        );
        let mut cursor = {
            let handle = &run;
            handle.lookup(&2)
        };
        assert_eq!(cursor.next(), Some('a'));
        let snapshot = cursor;
        assert_eq!(cursor.collect::<Vec<_>>(), ['b', 'a']);
        assert_eq!(snapshot.collect::<Vec<_>>(), ['b', 'a']);
        let words = SortedRun::<_, _>::build([("b", 1), ("a", 2)].into_iter());
        assert_eq!((&words).lookup(&"a").collect::<Vec<_>>(), [2]);
    }
}
