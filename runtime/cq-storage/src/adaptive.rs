//! Exact adaptive membership. [`AdaptiveMembership`] is contains-only: its hash
//! fallback owns keys directly, without ordering, shared cells or a sorted copy.
//! [`AdaptiveKeySet`] additionally supports interval existence: dense contains is
//! O(1), interval existence is O(1 + interval words). Its hash contains is expected
//! O(1); interval existence is O(log distinct keys), using a sorted array.
//! Total codecs first stage keys contiguously, without hashing or per-key allocation.
//! Dense finalization costs O(input keys + bitmap words); fallback costs
//! expected O(input keys) staging and O(distinct keys log distinct keys)
//! hash-array sorting. Dense retained memory is 8 * ceil(span / 64) bytes;
//! hash memory includes table capacity, shared key cells, and the sorted array.
//! Hash keys are shared between the table and array without requiring Clone.
//! Duplicate input witnesses are tested before allocating their shared key cell.
//! Retained bytes count owned capacities and key cells, excluding allocator
//! overhead and allocations owned by K. Staging bytes are reported separately.
//!
//! DESIGN contract8-adaptive-membership
//! Contract 8 §§8/9: index membership belongs to a grouped, byte-addressable run and exact directory.
//! Sources: doc/STORAGE.md §§6,8,9,12; Data Blocks §3.3; Kipf et al., RadixSpline (aiDM 2020).
//! Known deviation: AdaptiveKeySet stages Vec keys and uses Arc fallback; off-design (Plan P4).
//! DO NOT extend this staging into the default index-role provider or build an eager unbounded bitmap.
//! P4 replaces it with read-in-place grouped runs/lazy bounded membership; copies need recorded reasons.
//! Codec/role choices still require per-target measurements; Mac evidence is not CPU/CUDA evidence.
use crate::instrument::HashSet;
use crate::{Build, ExactIntervalMembership, ExactMembership};
use std::{hash::Hash, sync::Arc};

/// Some ordinals must be injective and order-preserving for all supported keys.
/// Returning None disables dense representation for the entire set.
pub trait KeyCodec<K>: Copy {
    // Encoding only: this capability maps logical keys to ordinals and provides no ordinal-to-key decode.
    fn ordinal(self, key: &K) -> Option<u128>;
    /// Certifies that ordinal is Some for every K, including query endpoints.
    /// Partial codecs retain the hash form so interval queries remain exact.
    fn total(self) -> bool {
        false
    }
}
#[derive(Clone, Copy, Default)]
pub struct NoDense;
impl<K> KeyCodec<K> for NoDense {
    fn ordinal(self, _: &K) -> Option<u128> {
        None
    }
}
/// Full-width signed encoding, also supports TPC's i32 keys.
#[derive(Clone, Copy, Default)]
pub struct I64Codec;
impl KeyCodec<i64> for I64Codec {
    fn total(self) -> bool {
        true
    }
    fn ordinal(self, key: &i64) -> Option<u128> {
        Some((*key as u64 ^ (1 << 63)) as u128)
    }
}
impl KeyCodec<i32> for I64Codec {
    fn total(self) -> bool {
        true
    }
    fn ordinal(self, key: &i32) -> Option<u128> {
        self.ordinal(&i64::from(*key))
    }
}
impl KeyCodec<u32> for I64Codec {
    fn total(self) -> bool {
        true
    }
    fn ordinal(self, key: &u32) -> Option<u128> {
        self.ordinal(&i64::from(*key))
    }
}
#[derive(Clone, Copy, Default)]
pub struct BorrowedI64TupleCodec;
impl KeyCodec<(&i64,)> for BorrowedI64TupleCodec {
    fn total(self) -> bool {
        true
    }
    fn ordinal(self, key: &(&i64,)) -> Option<u128> {
        I64Codec.ordinal(key.0)
    }
}
impl KeyCodec<(&i32,)> for BorrowedI64TupleCodec {
    fn total(self) -> bool {
        true
    }
    fn ordinal(self, key: &(&i32,)) -> Option<u128> {
        I64Codec.ordinal(key.0)
    }
}
impl KeyCodec<(&u32,)> for BorrowedI64TupleCodec {
    fn total(self) -> bool {
        true
    }
    fn ordinal(self, key: &(&u32,)) -> Option<u128> {
        I64Codec.ordinal(key.0)
    }
}
enum Form<K> {
    Dense {
        min: u128,
        max: u128,
        words: Vec<u64>,
    },
    Hash {
        keys: HashSet<Arc<K>>,
        sorted: Vec<Arc<K>>,
    },
}
pub struct AdaptiveKeySet<K, C, const MAX_BITS: usize> {
    form: Form<K>,
    codec: C,
    bytes: usize,
    _allocation: crate::counters_memory::Allocation,
}
impl<K: Eq + Hash + Ord, C: KeyCodec<K>, const B: usize> AdaptiveKeySet<K, C, B> {
    pub fn from_keys<I: IntoIterator<Item = K>>(keys: I, codec: C) -> Self {
        if codec.total() && B > 0 {
            let keys: Vec<K> = keys.into_iter().collect();
            let stage_bytes = keys.capacity() * std::mem::size_of::<K>();
            let mut allocation = crate::counters_memory::Allocation::new(stage_bytes);
            #[cfg(feature = "counters")]
            crate::counters::record(|c| {
                c.keyset_staging_bytes_peak = c.keyset_staging_bytes_peak.max(stage_bytes as u64);
            });
            let mut range: Option<(u128, u128)> = None;
            let mut encodable = true;
            for key in &keys {
                match codec.ordinal(key) {
                    Some(value) => {
                        range = Some(
                            range.map_or((value, value), |(lo, hi)| (lo.min(value), hi.max(value))),
                        );
                    }
                    None => {
                        encodable = false;
                        break;
                    }
                }
            }
            if let Some((min, max)) = range.filter(|(min, max)| {
                encodable
                    && max
                        .checked_sub(*min)
                        .and_then(|n| n.checked_add(1))
                        .is_some_and(|n| n <= B as u128)
            }) {
                let mut words = vec![0u64; ((max - min + 1) as usize).div_ceil(64)];
                for key in &keys {
                    crate::count!(distinct_insertions);
                    let offset = (codec.ordinal(key).expect("checked codec") - min) as usize;
                    words[offset / 64] |= 1 << (offset % 64);
                }
                #[cfg(feature = "counters")]
                {
                    crate::counters::build_cardinality(
                        words.iter().map(|word| word.count_ones() as usize).sum(),
                    );
                }
                let bytes = words.capacity() * std::mem::size_of::<u64>();
                allocation.grow(bytes);
                drop(keys);
                allocation.shrink(stage_bytes);
                return Self {
                    form: Form::Dense { min, max, words },
                    codec,
                    bytes,
                    _allocation: allocation,
                };
            }
            // Keep the staging allocation accounted while fallback consumes it.
            let result = Self::from_staged_keys(keys, codec);
            drop(allocation);
            return result;
        }
        Self::from_staged_keys(keys, codec)
    }

    fn from_staged_keys<I: IntoIterator<Item = K>>(keys: I, codec: C) -> Self {
        let mut staging = HashSet::default();
        let mut range: Option<(u128, u128)> = None;
        let mut ordinal = true;
        for key in keys {
            crate::count!(distinct_insertions);
            // Membership-only builds often receive many witnesses per key.
            // Do not allocate and immediately discard a shared cell for each.
            if staging.contains(&key) {
                continue;
            }
            match codec.ordinal(&key) {
                Some(x) => range = Some(range.map_or((x, x), |(lo, hi)| (lo.min(x), hi.max(x)))),
                None => ordinal = false,
            }
            staging.insert(Arc::new(key));
        }
        #[cfg(feature = "counters")]
        crate::counters::build_cardinality(staging.len());
        let stage_bytes = staging.capacity() * (std::mem::size_of::<Arc<K>>() + 1)
            + staging.len() * (std::mem::size_of::<K>() + 2 * std::mem::size_of::<usize>());
        let mut allocation = crate::counters_memory::Allocation::new(stage_bytes);
        let span = range.and_then(|(lo, hi)| hi.checked_sub(lo)?.checked_add(1));
        let form = if codec.total() && ordinal && span.is_some_and(|n| n <= B as u128) {
            let (min, max) = range.unwrap();
            let mut words = vec![0u64; (span.unwrap() as usize).div_ceil(64)];
            for key in &staging {
                let offset = (codec.ordinal(key).unwrap() - min) as usize;
                words[offset / 64] |= 1 << (offset % 64);
            }
            Form::Dense { min, max, words }
        } else {
            let mut sorted: Vec<_> = staging.iter().cloned().collect();
            sorted.sort_unstable();
            Form::Hash {
                keys: staging,
                sorted,
            }
        };
        let bytes = match &form {
            Form::Dense { words, .. } => words.capacity() * 8,
            Form::Hash { sorted, .. } => {
                stage_bytes + sorted.capacity() * std::mem::size_of::<Arc<K>>()
            }
        };
        // Staging and final bitmap coexist; hash array shares the staging keys.
        let extra = match &form {
            Form::Dense { .. } => bytes,
            Form::Hash { .. } => bytes - stage_bytes,
        };
        allocation.grow(extra);
        allocation.shrink(stage_bytes + extra - bytes);
        #[cfg(feature = "counters")]
        crate::counters::record(|c| {
            c.keyset_staging_bytes_peak = c.keyset_staging_bytes_peak.max(stage_bytes as u64)
        });
        Self {
            form,
            codec,
            bytes,
            _allocation: allocation,
        }
    }
    pub fn retained_bytes(&self) -> usize {
        self.bytes
    }
}
impl<K: Eq + Hash + Ord, C: KeyCodec<K>, const B: usize> ExactMembership<K>
    for AdaptiveKeySet<K, C, B>
{
    /// The dense test inlines into scan loops; the hash form stays out of line
    /// so it does not bloat them.
    #[inline]
    fn contains(&self, key: &K) -> bool {
        crate::count!(lookups);
        #[inline(never)]
        fn hashed<K: Eq + Hash>(keys: &HashSet<Arc<K>>, key: &K) -> bool {
            keys.contains(key)
        }
        match &self.form {
            Form::Hash { keys, .. } => hashed(keys, key),
            Form::Dense { min, max, words } => self
                .codec
                .ordinal(key)
                .is_some_and(|value| bitmap_contains(*min, *max, words, value)),
        }
    }
}
impl<K: Eq + Hash + Ord, C: KeyCodec<K>, const B: usize> ExactIntervalMembership<K>
    for AdaptiveKeySet<K, C, B>
{
    fn any_in(&self, lo: &K, hi: &K) -> bool {
        crate::count!(lookups);
        if lo > hi {
            return false;
        }
        match &self.form {
            Form::Hash { sorted, .. } => {
                let i = sorted.partition_point(|key| key.as_ref() < lo);
                sorted.get(i).is_some_and(|key| key.as_ref() <= hi)
            }
            Form::Dense { min, max, words } => {
                let (Some(lo), Some(hi)) = (self.codec.ordinal(lo), self.codec.ordinal(hi)) else {
                    // A lawful codec supports interval endpoints whenever it supports stored keys.
                    panic!("dense codec must encode interval endpoints");
                };
                let lo = lo.max(*min);
                let hi = hi.min(*max);
                if lo > hi {
                    return false;
                }
                let a = (lo - min) as usize;
                let b = (hi - min) as usize;
                let first = u64::MAX << (a % 64);
                let last = u64::MAX >> (63 - b % 64);
                if a / 64 == b / 64 {
                    return words[a / 64] & first & last != 0;
                }
                words[a / 64] & first != 0
                    || words[b / 64] & last != 0
                    || words[a / 64 + 1..b / 64].iter().any(|&w| w != 0)
            }
        }
    }
}
impl<K: Eq + Hash + Ord, C: KeyCodec<K> + Default, const B: usize, V, I: Iterator<Item = (K, V)>>
    Build<I> for AdaptiveKeySet<K, C, B>
{
    fn build(source: I) -> Self {
        crate::count!(index_builds);
        Self::from_keys(source.map(|(k, _)| k), C::default())
    }
}

/// Unsigned ordinal arithmetic in the representation's native word width.
///
/// Ordering and subtraction must be exact integer operations; conversion to
/// usize succeeds exactly when the difference is representable. A dense bitmap
/// is built only after its complete span passes that checked conversion.
pub trait OrdinalWord:
    Copy + Ord + Send + Sync + std::ops::Sub<Output = Self> + TryInto<usize>
{
}

impl OrdinalWord for u64 {}
impl OrdinalWord for u128 {}

/// A total, injective encoding for exact dense membership.
///
/// Implementations must return the same ordinal exactly when keys are equal,
/// including query keys not present during construction. Unlike [`KeyCodec`],
/// ordering is unnecessary: this capability supports contains-only storage.
pub trait DenseOrdinal: Eq {
    type Ordinal: OrdinalWord;
    // Encoding only: dense membership compares ordinals; DenseOrdinal has no inverse logical conversion.
    fn dense_ordinal(&self) -> Self::Ordinal;
}

impl DenseOrdinal for i64 {
    type Ordinal = u64;

    #[inline]
    fn dense_ordinal(&self) -> Self::Ordinal {
        *self as u64 ^ (1 << 63)
    }
}

impl DenseOrdinal for i32 {
    type Ordinal = u64;

    #[inline]
    fn dense_ordinal(&self) -> Self::Ordinal {
        i64::from(*self).dense_ordinal()
    }
}

impl DenseOrdinal for u32 {
    type Ordinal = u64;

    #[inline]
    fn dense_ordinal(&self) -> Self::Ordinal {
        u64::from(*self)
    }
}

impl<T: DenseOrdinal + ?Sized> DenseOrdinal for &T {
    type Ordinal = T::Ordinal;

    #[inline]
    fn dense_ordinal(&self) -> Self::Ordinal {
        T::dense_ordinal(*self)
    }
}

impl<T: DenseOrdinal> DenseOrdinal for (T,) {
    type Ordinal = T::Ordinal;

    #[inline]
    fn dense_ordinal(&self) -> Self::Ordinal {
        self.0.dense_ordinal()
    }
}

enum MembershipForm<K: DenseOrdinal, const B: usize> {
    Dense {
        min: K::Ordinal,
        max: K::Ordinal,
        words: Vec<u64>,
        _allocation: crate::counters_memory::Allocation,
    },
    Hash(AdaptiveMembership<K, B>),
}

/// Contains-only exact membership factory with a bounded dense bitmap.
///
/// [`crate::build!`] returns [`DenseMembership`] when the callsite proves
/// [`DenseOrdinal`], allowing ordinal calls to inline into probes. Otherwise it
/// returns this exact hash provider. The ordinary [`Build`] implementation also
/// uses hashing, requiring only `Eq + Hash`, without a sorted copy or shared cells.
///
/// Formal rule STR-AdaptiveMembership: Rust capability dispatch selects dense storage only with a total injective ordinal; generic equality/hash keys retain exact hash membership.
pub struct AdaptiveMembership<K, const MAX_BITS: usize> {
    keys: HashSet<K>,
    bytes: usize,
    _allocation: crate::counters_memory::Allocation,
}

impl<K, const B: usize> AdaptiveMembership<K, B> {
    /// The generic fallback always retains a hash table.
    pub fn is_dense(&self) -> bool {
        false
    }

    /// Owned capacity bytes, excluding allocator overhead and allocations owned
    /// by K.
    pub fn retained_bytes(&self) -> usize {
        self.bytes
    }
}

impl<K: Eq + Hash, const B: usize> AdaptiveMembership<K, B> {
    fn from_hash_keys(keys: impl IntoIterator<Item = K>) -> Self {
        let mut stored = HashSet::default();
        for key in keys {
            crate::count!(distinct_insertions);
            stored.insert(key);
        }
        #[cfg(feature = "counters")]
        crate::counters::build_cardinality(stored.len());
        let bytes = stored.capacity() * (std::mem::size_of::<K>() + 1);
        Self {
            keys: stored,
            bytes,
            _allocation: crate::counters_memory::Allocation::new(bytes),
        }
    }
}

/// Dense-capable result of [`crate::build!`] for [`AdaptiveMembership`] factories.
///
/// Both the bitmap and sparse hash forms use static ordinal capability bounds;
/// no function pointer or runtime key-type inspection is needed by a probe.
pub struct DenseMembership<K: DenseOrdinal, const MAX_BITS: usize> {
    form: MembershipForm<K, MAX_BITS>,
}

impl<K: DenseOrdinal, const B: usize> DenseMembership<K, B> {
    pub fn is_dense(&self) -> bool {
        matches!(&self.form, MembershipForm::Dense { .. })
    }

    /// Owned capacity bytes, excluding allocator overhead and allocations owned
    /// by K. Dense values retain only `8 * ceil(span / 64)` heap bytes.
    pub fn retained_bytes(&self) -> usize {
        match &self.form {
            MembershipForm::Dense { words, .. } => words.capacity() * std::mem::size_of::<u64>(),
            MembershipForm::Hash(keys) => keys.retained_bytes(),
        }
    }
}

impl<K: DenseOrdinal + Hash, const B: usize> DenseMembership<K, B> {
    fn from_hash_keys(keys: impl IntoIterator<Item = K>) -> Self {
        Self {
            form: MembershipForm::Hash(AdaptiveMembership::from_hash_keys(keys)),
        }
    }

    /// Build with the ordinal capability explicitly available. Keys are
    /// staged without hashing; sparse sets retain their original keys in a hash
    /// table. Empty sets and a zero bitmap budget also use the hash form.
    pub fn from_dense_keys(keys: impl IntoIterator<Item = K>) -> Self {
        if B == 0 {
            return Self::from_hash_keys(keys);
        }
        let keys: Vec<K> = keys.into_iter().collect();
        let stage_bytes = keys.capacity() * std::mem::size_of::<K>();
        let mut allocation = crate::counters_memory::Allocation::new(stage_bytes);
        #[cfg(feature = "counters")]
        crate::counters::record(|c| {
            c.keyset_staging_bytes_peak = c.keyset_staging_bytes_peak.max(stage_bytes as u64);
        });
        let mut range: Option<(K::Ordinal, K::Ordinal)> = None;
        for key in &keys {
            let ordinal = key.dense_ordinal();
            range = Some(range.map_or((ordinal, ordinal), |(lo, hi)| {
                (lo.min(ordinal), hi.max(ordinal))
            }));
        }
        if let Some((min, max, span)) = range.and_then(|(min, max)| {
            let distance: usize = (max - min).try_into().ok()?;
            let span = distance.checked_add(1)?;
            (span <= B).then_some((min, max, span))
        }) {
            let mut words = vec![0u64; span.div_ceil(64)];
            for key in &keys {
                crate::count!(distinct_insertions);
                let offset: usize = (key.dense_ordinal() - min)
                    .try_into()
                    .ok()
                    .expect("bounded dense ordinal span");
                words[offset / 64] |= 1 << (offset % 64);
            }
            #[cfg(feature = "counters")]
            crate::counters::build_cardinality(
                words.iter().map(|word| word.count_ones() as usize).sum(),
            );
            let bytes = words.capacity() * std::mem::size_of::<u64>();
            allocation.grow(bytes);
            drop(keys);
            allocation.shrink(stage_bytes);
            return Self {
                form: MembershipForm::Dense {
                    min,
                    max,
                    words,
                    _allocation: allocation,
                },
            };
        }
        // Keep staging charged while the hash table consumes its keys.
        let result = Self::from_hash_keys(keys);
        drop(allocation);
        result
    }
}

impl<K: Eq + Hash, const B: usize, V, I: Iterator<Item = (K, V)>> Build<I>
    for AdaptiveMembership<K, B>
{
    fn build(source: I) -> Self {
        crate::count!(index_builds);
        Self::from_hash_keys(source.map(|(key, _)| key))
    }
}

impl<K: DenseOrdinal + Hash, const B: usize, V, I: Iterator<Item = (K, V)>> Build<I>
    for DenseMembership<K, B>
{
    fn build(source: I) -> Self {
        crate::count!(index_builds);
        Self::from_dense_keys(source.map(|(key, _)| key))
    }
}

impl<K: Eq + Hash, const B: usize> ExactMembership<K> for AdaptiveMembership<K, B> {
    #[inline]
    fn contains(&self, key: &K) -> bool {
        crate::count!(lookups);
        self.keys.contains(key)
    }
}

impl<K: DenseOrdinal + Hash, const B: usize> ExactMembership<K> for DenseMembership<K, B> {
    #[inline]
    fn contains(&self, key: &K) -> bool {
        #[inline(never)]
        fn hashed<K: Eq + Hash, const B: usize>(keys: &AdaptiveMembership<K, B>, key: &K) -> bool {
            keys.contains(key)
        }
        match &self.form {
            MembershipForm::Hash(keys) => hashed(keys, key),
            MembershipForm::Dense {
                min, max, words, ..
            } => {
                crate::count!(lookups);
                bitmap_contains(*min, *max, words, key.dense_ordinal())
            }
        }
    }
}

#[inline]
fn bitmap_contains<O: OrdinalWord>(min: O, max: O, words: &[u64], value: O) -> bool {
    if value < min || value > max {
        return false;
    }
    let offset: usize = (value - min)
        .try_into()
        .ok()
        .expect("bounded dense ordinal span");
    words[offset / 64] & (1 << (offset % 64)) != 0
}
