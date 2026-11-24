//! Capabilities the Sequence level reads through (`doc/formal/backend_ir.tex`).
//!
//! The level records only what a read requires; a provider or codec type is
//! an opaque token that must implement the matching trait. Each trait states
//! the law its implementation owes; the storage-law tests below check them
//! for the crate's own types.
//!
//! - [`RangeCursor`]: a provider whose bucket for a key is one contiguous
//!   range of a stored item sequence (rule BE-Bind's `rangeCursor` premise):
//!   `lookup(k)` enumerates exactly `items()[range(k)]`, so a flatten of
//!   lookups can be kept as offsets (BE-Offsets).
//! - [`SequenceCodec`]: a value refinement (BE-Code): `decode(encode(v)) == v`
//!   on the domain; `Code: Eq` with an injective decoder licenses equality on
//!   codes, and a monotone encoder (`Code: Ord`, checked by [`monotone`]) licenses
//!   order reads on codes. Existing membership and exact-bucket reads keep
//!   [`crate::ExactMembership`] and [`crate::IndexRead`].

use crate::{IndexRead, KeyCodec, OrderKey, RadixRun, SortedRun};

/// An allocation-free choice between streams with the same item type.
/// Only the selected iterator is constructed and advanced.
pub enum Choice<L, R> {
    Left(L),
    Right(R),
}

impl<L: Iterator, R: Iterator<Item = L::Item>> Iterator for Choice<L, R> {
    type Item = L::Item;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Left(rows) => rows.next(),
            Self::Right(rows) => rows.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::Left(rows) => rows.size_hint(),
            Self::Right(rows) => rows.size_hint(),
        }
    }

    fn fold<B, F: FnMut(B, Self::Item) -> B>(self, init: B, f: F) -> B {
        match self {
            Self::Left(rows) => rows.fold(init, f),
            Self::Right(rows) => rows.fold(init, f),
        }
    }
}

/// A provider whose bucket for `key` is the contiguous range
/// `range(key)` of `items()`. Implementations keep items in key order.
pub trait RangeCursor<K: ?Sized> {
    type Item: Copy;
    /// Every stored item, bucket by bucket, in key order.
    fn items(&self) -> &[Self::Item];
    /// Half-open position range of `key`'s bucket in `items()`; empty when absent.
    fn range(&self, key: &K) -> (usize, usize);
}

impl<K: Ord + Copy, V: Copy, C: KeyCodec<K>> RangeCursor<K> for SortedRun<K, V, C> {
    type Item = V;
    fn items(&self) -> &[V] {
        self.items()
    }
    fn range(&self, key: &K) -> (usize, usize) {
        self.range(key)
    }
}

impl<K: OrderKey<Ordinal = u64> + Copy, V: Copy, C: crate::ItemColumns<Row = V>> RangeCursor<K>
    for RadixRun<K, V, C>
{
    type Item = V;
    fn items(&self) -> &[V] {
        self.items()
    }
    fn range(&self, key: &K) -> (usize, usize) {
        self.range(key)
    }
}

/// A value refinement: `decode(encode(v)) == v` for every `v` in the domain.
/// `encode` returns `None` outside the domain (a literal outside it matches no
/// row). `Code: Eq` must agree with value equality (injective decoder).
pub trait SequenceCodec<T> {
    type Code: Copy + Eq;
    fn encode(&self, value: &T) -> Option<Self::Code>;
    // DECODE ?: SequenceCodec exposes a generic logical boundary without caller Rust-read or publication facts.
    fn decode(&self, code: Self::Code) -> T;
}

/// The identity codec on `Copy` values.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Identity;
impl<T: Copy + Eq> SequenceCodec<T> for Identity {
    type Code = T;
    fn encode(&self, value: &T) -> Option<T> {
        Some(*value)
    }
    // DECODE ?: Identity implements the generic decode boundary as a no-op; caller purpose is not recorded.
    fn decode(&self, code: T) -> T {
        code
    }
}

/// A dense, order-preserving dictionary of one domain: value `i` of the sorted
/// distinct domain encodes as `i`. Satisfies BE-Code's injective and monotone
/// premises and the dense-directory premise (`codes are 0..len`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dense<T> {
    domain: Vec<T>,
}
impl<T: Ord + Clone> Dense<T> {
    pub fn new(values: impl IntoIterator<Item = T>) -> Self {
        let mut domain: Vec<T> = values.into_iter().collect();
        domain.sort();
        domain.dedup();
        Self { domain }
    }
    pub fn len(&self) -> usize {
        self.domain.len()
    }
    pub fn is_empty(&self) -> bool {
        self.domain.is_empty()
    }
}
impl<T: Ord + Clone> SequenceCodec<T> for Dense<T> {
    type Code = u32;
    fn encode(&self, value: &T) -> Option<u32> {
        self.domain.binary_search(value).ok().map(|i| i as u32)
    }
    // DECODE ?: Dense resolves a dictionary ordinal to T; mixed generic callers supply no boundary evidence.
    fn decode(&self, code: u32) -> T {
        self.domain[code as usize].clone()
    }
}

/// Check BE-Code's premises on a finite sample of the domain: round trip
/// (injective decoder) and, when `Code: Ord`, monotone encoding.
pub fn check_codec<T: Ord + Clone, C: SequenceCodec<T>>(
    codec: &C,
    sample: &[T],
) -> Result<(), &'static str>
where
    C::Code: Ord,
{
    let mut sorted = sample.to_vec();
    sorted.sort();
    for value in &sorted {
        let Some(code) = codec.encode(value) else {
            return Err("sample value outside the codec domain");
        };
        // DECODE P: check_codec exposes the reconstructed value only to its round-trip diagnostic equality oracle.
        if codec.decode(code) != *value {
            return Err("decode(encode(v)) != v");
        }
    }
    if !monotone(codec, &sorted) {
        return Err("encoder is not monotone on the sample");
    }
    Ok(())
}

/// `encode` is order-preserving on `sorted` (ascending values give ascending codes).
pub fn monotone<T, C: SequenceCodec<T>>(codec: &C, sorted: &[T]) -> bool
where
    C::Code: Ord,
{
    sorted.windows(2).all(
        |pair| match (codec.encode(&pair[0]), codec.encode(&pair[1])) {
            (Some(a), Some(b)) => a <= b,
            _ => false,
        },
    )
}

/// Storage law of [`RangeCursor`]: the range read equals the exact-bucket read.
pub fn check_range_cursor<K, P>(provider: &P, keys: &[K]) -> Result<(), &'static str>
where
    P: RangeCursor<K>,
    for<'a> &'a P: IndexRead<Key = K, Item = <P as RangeCursor<K>>::Item>,
    <P as RangeCursor<K>>::Item: PartialEq,
{
    let items = provider.items();
    for key in keys {
        let (begin, end) = provider.range(key);
        if begin > end || end > items.len() {
            return Err("range outside items");
        }
        let by_range = &items[begin..end];
        let by_lookup: Vec<_> = provider.lookup(key).collect();
        if by_range != by_lookup.as_slice() {
            return Err("range read differs from bucket read");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NoDense;

    #[test]
    fn choice_streams_are_lazy_and_preserve_nested_parts() {
        for left in [true, false] {
            let visits = std::cell::Cell::new(0);
            let rows = if left {
                Choice::Left((0..4).inspect(|_| visits.set(visits.get() + 1)))
            } else {
                Choice::Right((0..8).step_by(2).inspect(|_| visits.set(visits.get() + 1)))
            };
            assert_eq!(visits.get(), 0);
            assert_eq!(rows.size_hint(), (4, Some(4)));
            assert_eq!(rows.take(1).count(), 1);
            assert_eq!(visits.get(), 1);

            let parts = if left {
                Choice::Left([0..2, 2..4].into_iter().map(Choice::Left))
            } else {
                Choice::Right(
                    [0..4]
                        .into_iter()
                        .map(|part| Choice::Right(part.step_by(1))),
                )
            };
            assert_eq!(parts.flatten().sum::<i32>(), 6);
        }
    }

    #[test]
    fn sorted_run_range_equals_lookup_including_absent_keys() {
        let run: SortedRun<i32, (i32, i32), NoDense> = SortedRun::from_pairs(
            [(3, (3, 0)), (1, (1, 0)), (3, (3, 1)), (2, (2, 0))],
            NoDense,
        );
        check_range_cursor(&run, &[0, 1, 2, 3, 4]).unwrap();
        assert_eq!(run.range(&3), (2, 4));
        assert_eq!(run.range(&9), (4, 4));
    }

    #[test]
    fn canonical_key_order_agrees_across_membership_and_run_readers() {
        use crate::{
            AdaptiveKeySet, DenseMembership, ExactIntervalMembership, ExactMembership, I64Codec,
        };

        fn check<K: OrderKey<Ordinal = u64> + Copy + std::hash::Hash + std::fmt::Debug>(
            stored: &[K],
            probes: &[K],
        ) {
            let membership = DenseMembership::<_, 256>::from_dense_keys(stored.iter().copied());
            let intervals =
                AdaptiveKeySet::<_, _, 256>::from_keys(stored.iter().copied(), I64Codec);
            let sorted =
                SortedRun::from_pairs(stored.iter().copied().map(|key| (key, key)), I64Codec);
            let radix = RadixRun::from_pairs(stored.iter().copied().map(|key| (key, key)));
            check_range_cursor(&sorted, probes).unwrap();
            check_range_cursor(&radix, probes).unwrap();
            for key in probes {
                let present = stored.contains(key);
                assert_eq!(membership.contains(key), present);
                assert_eq!(intervals.contains(key), present);
                assert_eq!(sorted.contains(key), present);
                assert_eq!(radix.contains(key), present);
                assert_eq!(
                    (&sorted).lookup(key).collect::<Vec<_>>(),
                    (&radix).lookup(key).collect::<Vec<_>>()
                );
                for hi in probes {
                    assert_eq!(
                        intervals.any_in(key, hi),
                        stored.iter().any(|value| key <= value && value <= hi),
                    );
                }
            }
        }

        check(
            &[-64i32, -1, 0, 63, -1],
            &[i32::MIN, -65, -64, -2, -1, 0, 1, 63, 64, i32::MAX],
        );
        check(
            &[i64::MIN, -1, 0, i64::MAX],
            &[i64::MIN, i64::MIN + 1, -2, -1, 0, 1, i64::MAX],
        );
        check(
            &[0u32, 63, 65, u32::MAX],
            &[0u32, 1, 63, 64, 65, u32::MAX - 1, u32::MAX],
        );
        check(
            &[0u64, 63, 65, u64::MAX],
            &[0u64, 1, 63, 64, 65, u64::MAX - 1, u64::MAX],
        );
        let values = [-64i32, -1, 0, 63];
        check(
            &[(&values[0],), (&values[2],)],
            &[(&values[0],), (&values[1],), (&values[2],), (&values[3],)],
        );
    }

    #[test]
    fn dense_codec_round_trips_and_is_monotone_and_rejects_outside_domain() {
        let codec = Dense::new(["ASIA", "EUROPE", "AFRICA", "ASIA"]);
        assert_eq!(codec.len(), 3);
        // DECODE P: this diagnostic invokes check_codec to expose every dense sample for round-trip assertions.
        check_codec(&codec, &["AFRICA", "ASIA", "EUROPE"]).unwrap();
        assert_eq!(codec.encode(&"ASIA"), Some(1));
        assert_eq!(codec.encode(&"MARS"), None);
        // DECODE P: this identity-codec diagnostic exposes sample values through check_codec's equality oracle.
        assert_eq!(check_codec(&Identity, &[5i64, -1, 7]), Ok(()));
    }
}
