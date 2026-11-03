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
//! Construction-attached dense sidecars set bitmap words directly as the exact
//! owner consumes its source. Only an over-budget span switches to a sparse set
//! of injective ordinals, reconstructed from the bitmap, never from original keys.
//!
//! DESIGN contract8-adaptive-membership
//! Contract 8 §§8/9: index membership belongs to a grouped, byte-addressable run and exact directory.
//! Sources: doc/STORAGE.md §§6,8,9,12; Data Blocks §3.3; Kipf et al., RadixSpline (aiDM 2020).
//! Known deviation: AdaptiveKeySet stages Vec keys and uses Arc fallback; off-design (Plan P4).
//! DO NOT extend this staging into the default index-role provider or build an eager unbounded bitmap.
//! P4 replaces it with read-in-place grouped runs/lazy bounded membership; copies need recorded reasons.
//! Codec/role choices still require per-target measurements; Mac evidence is not CPU/CUDA evidence.
use crate::instrument::HashSet;
use crate::{
    Build, BuildSidecar, ExactIntervalMembership, ExactMembership, KeyCodec, MergeSidecarBuilder,
    OrderKey, OrdinalWord, SidecarBuilder,
};
use std::{collections::VecDeque, hash::Hash, sync::Arc};

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

enum MembershipForm<K: OrderKey, const B: usize> {
    Dense {
        min: K::Ordinal,
        max: K::Ordinal,
        words: Vec<u64>,
        _allocation: crate::counters_memory::Allocation,
    },
    Hash(AdaptiveMembership<K, B>),
}

/// Contains-only exact hash membership, requiring only `Eq + Hash`, without a
/// sorted copy or shared cells.
///
/// Formal rule STR-AdaptiveMembership: structure selection records
/// [`DenseMembership`] (bounded dense bitmap over the total order-preserving
/// [`OrderKey`] law) for a one-column key and this hash provider for wider keys;
/// a dense key lacking `OrderKey` is a rustc error, never a hash fallback.
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

/// Incremental hash sidecar; only this selected filter owns the cloned keys.
pub struct AdaptiveMembershipBuilder<K, const MAX_BITS: usize> {
    membership: AdaptiveMembership<K, MAX_BITS>,
}

impl<K: Eq + Hash + Clone, const B: usize> BuildSidecar<K> for AdaptiveMembership<K, B> {
    type Builder = AdaptiveMembershipBuilder<K, B>;

    fn sidecar_builder() -> Self::Builder {
        AdaptiveMembershipBuilder {
            membership: Self {
                keys: HashSet::default(),
                bytes: 0,
                _allocation: crate::counters_memory::Allocation::new(0),
            },
        }
    }
}

impl<K: Eq + Hash + Clone, const B: usize> SidecarBuilder<K>
    for AdaptiveMembershipBuilder<K, B>
{
    type Output = AdaptiveMembership<K, B>;

    fn observe(&mut self, key: &K) {
        crate::count!(distinct_insertions);
        // Do not clone owned scalar keys for duplicate witnesses.
        if self.membership.keys.contains(key) {
            return;
        }
        self.membership.keys.insert(key.clone());
        let bytes = self.membership.keys.capacity() * (std::mem::size_of::<K>() + 1);
        if bytes != self.membership.bytes {
            self.membership._allocation.grow(bytes - self.membership.bytes);
            self.membership.bytes = bytes;
        }
    }

    fn finish(self) -> Self::Output {
        // Partition observers count one selected filter only when published.
        crate::count!(index_builds);
        #[cfg(feature = "counters")]
        crate::counters::build_cardinality(self.membership.keys.len());
        self.membership
    }
}

impl<K: Eq + Hash + Clone, const B: usize> MergeSidecarBuilder<K>
    for AdaptiveMembershipBuilder<K, B>
{
    fn merge(&mut self, other: Self) {
        for key in other.membership.keys {
            crate::count!(distinct_insertions);
            self.membership.keys.insert(key);
            let bytes = self.membership.keys.capacity() * (std::mem::size_of::<K>() + 1);
            if bytes != self.membership.bytes {
                self.membership._allocation.grow(bytes - self.membership.bytes);
                self.membership.bytes = bytes;
            }
        }
    }
}

/// Recorded dense membership (`dense_membership`): a bounded bitmap over
/// [`OrderKey`] ordinals, kept as exact hash membership when the span exceeds
/// `MAX_BITS`.
///
/// Both the bitmap and sparse hash forms use static ordinal capability bounds;
/// no function pointer or runtime key-type inspection is needed by a probe.
pub struct DenseMembership<K: OrderKey, const MAX_BITS: usize> {
    form: MembershipForm<K, MAX_BITS>,
}

impl<K: OrderKey, const B: usize> DenseMembership<K, B> {
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

impl<K: OrderKey + Hash, const B: usize> DenseMembership<K, B> {
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
            let ordinal = key.ordinal();
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
                let offset: usize = (key.ordinal() - min)
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

enum AttachedMembershipForm<K: OrderKey> {
    Dense {
        min: K::Ordinal,
        max: K::Ordinal,
        first_bit: usize,
        words: Vec<u64>,
        _allocation: crate::counters_memory::Allocation,
    },
    Hash {
        keys: HashSet<u128>,
        _allocation: crate::counters_memory::Allocation,
    },
}

/// Explicitly selected construction-attached membership.
///
/// This direct bitmap/ordinal-set representation is separate from the legacy
/// projected [`DenseMembership`] build, including its hot-loop enum and probes.
pub struct AttachedDenseMembership<K: OrderKey, const MAX_BITS: usize> {
    form: AttachedMembershipForm<K>,
}

impl<K: OrderKey, const B: usize> AttachedDenseMembership<K, B> {
    pub fn is_dense(&self) -> bool {
        matches!(&self.form, AttachedMembershipForm::Dense { .. })
    }

    /// Owned capacity bytes. Bitmaps retain their word-aligned growth capacity.
    pub fn retained_bytes(&self) -> usize {
        match &self.form {
            AttachedMembershipForm::Dense { words, .. } => {
                words.capacity() * std::mem::size_of::<u64>()
            }
            AttachedMembershipForm::Hash { keys, .. } => {
                keys.capacity() * (std::mem::size_of::<u128>() + 1)
            }
        }
    }
}

impl<K: OrderKey, const B: usize> ExactMembership<K> for AttachedDenseMembership<K, B> {
    #[inline]
    fn contains(&self, key: &K) -> bool {
        crate::count!(lookups);
        match &self.form {
            AttachedMembershipForm::Hash { keys, .. } => keys.contains(&key.ordinal().into()),
            AttachedMembershipForm::Dense {
                min, max, first_bit, words, ..
            } => {
                let ordinal = key.ordinal();
                if ordinal < *min || ordinal > *max {
                    return false;
                }
                let offset = TryInto::<usize>::try_into(ordinal - *min)
                    .ok()
                    .expect("bounded dense ordinal span");
                let bit = offset % 64 + *first_bit;
                words[offset / 64 + bit / 64] & (1 << (bit % 64)) != 0
            }
        }
    }
}

/// Incremental word-aligned bitmap with exact ordinal-set fallback.
///
/// No original keys are copied or collected. The bitmap grows at either end
/// only while its observed span fits the selected budget. A larger span converts
/// its set bits to injective ordinals, so sparse fallback needs no inverse codec.
pub struct AttachedDenseMembershipBuilder<K: OrderKey, const MAX_BITS: usize> {
    range: Option<(K::Ordinal, K::Ordinal)>,
    words: VecDeque<u64>,
    sparse: Option<HashSet<u128>>,
    bytes: usize,
    allocation: crate::counters_memory::Allocation,
}

impl<K: OrderKey, const B: usize> BuildSidecar<K> for AttachedDenseMembership<K, B> {
    type Builder = AttachedDenseMembershipBuilder<K, B>;

    fn sidecar_builder() -> Self::Builder {
        AttachedDenseMembershipBuilder {
            range: None,
            words: VecDeque::new(),
            sparse: None,
            bytes: 0,
            allocation: crate::counters_memory::Allocation::new(0),
        }
    }
}

impl<K: OrderKey, const B: usize> AttachedDenseMembershipBuilder<K, B> {
    fn account_capacity(&mut self, bytes: usize) {
        if bytes > self.bytes {
            self.allocation.grow(bytes - self.bytes);
        } else if bytes < self.bytes {
            self.allocation.shrink(self.bytes - bytes);
        }
        self.bytes = bytes;
    }

    fn insert_sparse(&mut self, ordinal: u128) {
        let keys = self.sparse.as_mut().expect("sparse ordinal representation");
        keys.insert(ordinal);
        let bytes = keys.capacity() * (std::mem::size_of::<u128>() + 1);
        if bytes != self.bytes {
            self.account_capacity(bytes);
        }
    }

    fn make_sparse(&mut self) {
        let mut keys = HashSet::default();
        if let Some((min, _)) = self.range {
            let first_word = Into::<u128>::into(min) >> 6;
            for (index, &word) in self.words.iter().enumerate() {
                let base = (first_word + index as u128) << 6;
                let mut bits = word;
                while bits != 0 {
                    keys.insert(base | u128::from(bits.trailing_zeros()));
                    bits &= bits - 1;
                }
            }
        }
        // Charge the overlap while the new sparse representation replaces words.
        let bytes = keys.capacity() * (std::mem::size_of::<u128>() + 1);
        self.allocation.grow(bytes);
        self.words = VecDeque::new();
        self.allocation.shrink(self.bytes);
        self.bytes = bytes;
        self.sparse = Some(keys);
    }

    fn extend_range(&mut self, min: K::Ordinal, max: K::Ordinal) {
        let (min, max) = self.range.map_or((min, max), |(lo, hi)| {
            (lo.min(min), hi.max(max))
        });
        if self.sparse.is_none() {
            let span = TryInto::<usize>::try_into(max - min)
                .ok()
                .and_then(|distance| distance.checked_add(1));
            if !span.is_some_and(|span| span <= B) {
                self.make_sparse();
            } else {
                let first_word = Into::<u128>::into(min) >> 6;
                let last_word = Into::<u128>::into(max) >> 6;
                let length = usize::try_from(last_word - first_word)
                    .expect("bounded bitmap word span")
                    + 1;
                if length > self.words.capacity() {
                    // Amortized growth, capped by the budget plus word alignment.
                    let max_words = (B - 1) / 64 + 2;
                    let capacity = length
                        .checked_next_power_of_two()
                        .unwrap_or(length)
                        .min(max_words);
                    self.words.reserve_exact(capacity - self.words.len());
                    self.account_capacity(self.words.capacity() * std::mem::size_of::<u64>());
                }
                if let Some((old_min, _)) = self.range {
                    let prepend = usize::try_from((Into::<u128>::into(old_min) >> 6) - first_word)
                        .expect("bounded bitmap word span");
                    for _ in 0..prepend {
                        self.words.push_front(0);
                    }
                }
                self.words.resize(length, 0);
            }
        }
        self.range = Some((min, max));
    }
}

impl<K: OrderKey, const B: usize> SidecarBuilder<K> for AttachedDenseMembershipBuilder<K, B> {
    type Output = AttachedDenseMembership<K, B>;

    #[inline]
    fn observe(&mut self, key: &K) {
        crate::count!(distinct_insertions);
        let ordinal = key.ordinal();
        if !self.range.is_some_and(|(min, max)| min <= ordinal && ordinal <= max) {
            self.extend_range(ordinal, ordinal);
        }
        if self.sparse.is_some() {
            self.insert_sparse(ordinal.into());
        } else {
            let min = self.range.expect("observed ordinal range").0;
            let offset = TryInto::<usize>::try_into(ordinal - min)
                .ok()
                .expect("bounded dense ordinal span");
            let bit = offset % 64 + (Into::<u128>::into(min) as usize & 63);
            self.words[offset / 64 + bit / 64] |= 1 << (bit % 64);
        }
    }

    fn finish(self) -> Self::Output {
        crate::count!(index_builds);
        if let Some(keys) = self.sparse {
            #[cfg(feature = "counters")]
            crate::counters::build_cardinality(keys.len());
            return AttachedDenseMembership {
                form: AttachedMembershipForm::Hash {
                    keys,
                    _allocation: self.allocation,
                },
            };
        }
        let Some((min, max)) = self.range else {
            #[cfg(feature = "counters")]
            crate::counters::build_cardinality(0);
            return AttachedDenseMembership {
                form: AttachedMembershipForm::Hash {
                    keys: HashSet::default(),
                    _allocation: self.allocation,
                },
            };
        };
        #[cfg(feature = "counters")]
        crate::counters::build_cardinality(
            self.words.iter().map(|word| word.count_ones() as usize).sum(),
        );
        // VecDeque -> Vec preserves the allocation, rotating words in place.
        let words = Vec::from(self.words);
        AttachedDenseMembership {
            form: AttachedMembershipForm::Dense {
                min,
                max,
                first_bit: Into::<u128>::into(min) as usize & 63,
                words,
                _allocation: self.allocation,
            },
        }
    }
}

impl<K: OrderKey, const B: usize> MergeSidecarBuilder<K> for AttachedDenseMembershipBuilder<K, B> {
    fn merge(&mut self, other: Self) {
        let Some((min, max)) = other.range else {
            return;
        };
        self.extend_range(min, max);
        if let Some(keys) = other.sparse {
            for ordinal in keys {
                self.insert_sparse(ordinal);
            }
        } else {
            let first_word = Into::<u128>::into(min) >> 6;
            if self.sparse.is_some() {
                for (index, &word) in other.words.iter().enumerate() {
                    let base = (first_word + index as u128) << 6;
                    let mut bits = word;
                    while bits != 0 {
                        self.insert_sparse(base | u128::from(bits.trailing_zeros()));
                        bits &= bits - 1;
                    }
                }
            } else {
                let own_min = self.range.expect("merged ordinal range").0;
                let offset = usize::try_from(first_word - (Into::<u128>::into(own_min) >> 6))
                    .expect("bounded bitmap word span");
                for (word, other) in self.words.iter_mut().skip(offset).zip(other.words.iter()) {
                    *word |= *other;
                }
            }
        }
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

impl<K: OrderKey + Hash, const B: usize, V, I: Iterator<Item = (K, V)>> Build<I>
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

impl<K: OrderKey + Hash, const B: usize> ExactMembership<K> for DenseMembership<K, B> {
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
                bitmap_contains(*min, *max, words, key.ordinal())
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

#[cfg(test)]
mod sidecar_tests {
    use super::{AdaptiveMembership, AttachedDenseMembership};
    use crate::{BuildSidecar, ExactMembership, MergeSidecarBuilder, OrderKey, SidecarBuilder};

    #[test]
    fn dense_sidecar_deduplicates_without_false_negatives() {
        let mut builder = AttachedDenseMembership::<i64, 128>::sidecar_builder();
        for key in [-64, -64, -1, 0, 63] {
            builder.observe(&key);
        }
        let filter = builder.finish();
        assert!(filter.is_dense());
        assert_eq!(filter.retained_bytes(), 16);
        for key in -65..=64 {
            assert_eq!(filter.contains(&key), [-64, -1, 0, 63].contains(&key));
        }
    }

    #[test]
    fn dense_sidecar_handles_empty_zero_budget_and_full_width_extremes() {
        let empty = AttachedDenseMembership::<i64, 64>::sidecar_builder().finish();
        assert!(!empty.is_dense());
        assert!(!empty.contains(&0));

        let mut zero = AttachedDenseMembership::<i64, 0>::sidecar_builder();
        zero.observe(&7);
        let zero = zero.finish();
        assert!(!zero.is_dense());
        assert!(zero.contains(&7));
        assert!(!zero.contains(&8));

        let mut wide = AttachedDenseMembership::<u128, 128>::sidecar_builder();
        for key in [u128::MIN, 7, u128::MAX, u128::MIN] {
            wide.observe(&key);
        }
        let wide = wide.finish();
        assert!(!wide.is_dense());
        for key in [u128::MIN, 7, u128::MAX] {
            assert!(wide.contains(&key));
        }
        assert!(!wide.contains(&1));

        let mut narrow = AttachedDenseMembership::<i64, 2>::sidecar_builder();
        narrow.observe(&i64::MAX);
        narrow.observe(&(i64::MAX - 1));
        let narrow = narrow.finish();
        assert!(narrow.is_dense());
        assert!(narrow.contains(&i64::MAX));
        assert!(narrow.contains(&(i64::MAX - 1)));
        assert!(!narrow.contains(&i64::MIN));
    }

    #[test]
    fn direct_bitmap_grows_at_both_ends_and_preserves_unaligned_words() {
        let keys = [130_i64, 63, 0, 191, -1, 64, 129, 130];
        let mut builder = AttachedDenseMembership::<i64, 256>::sidecar_builder();
        for key in keys {
            builder.observe(&key);
        }
        let capacity = builder.words.capacity();
        let filter = builder.finish();
        assert!(filter.is_dense());
        assert_eq!(filter.retained_bytes(), capacity * 8);
        for key in -2..=192 {
            assert_eq!(filter.contains(&key), keys.contains(&key), "key {key}");
        }

        // The exact key span, not rounded word occupancy, controls the budget.
        let mut boundary = AttachedDenseMembership::<u64, 2>::sidecar_builder();
        boundary.observe(&64);
        boundary.observe(&63);
        let boundary = boundary.finish();
        assert!(boundary.is_dense());
        for key in 62..=65 {
            assert_eq!(boundary.contains(&key), key == 63 || key == 64);
        }
    }

    #[test]
    fn direct_sidecar_never_needs_to_clone_original_keys() {
        #[derive(Eq, PartialEq, Ord, PartialOrd)]
        struct Key(u64);
        impl OrderKey for Key {
            type Ordinal = u64;

            fn ordinal(&self) -> Self::Ordinal {
                self.0
            }
        }
        let mut dense = AttachedDenseMembership::<Key, 128>::sidecar_builder();
        dense.observe(&Key(63));
        dense.observe(&Key(64));
        let dense = dense.finish();
        assert!(dense.is_dense());
        assert!(dense.contains(&Key(63)));
        assert!(dense.contains(&Key(64)));
        assert!(!dense.contains(&Key(65)));

        let mut sparse = AttachedDenseMembership::<Key, 2>::sidecar_builder();
        sparse.observe(&Key(63));
        sparse.observe(&Key(64));
        sparse.observe(&Key(u64::MAX));
        let sparse = sparse.finish();
        assert!(!sparse.is_dense());
        for key in [63, 64, u64::MAX] {
            assert!(sparse.contains(&Key(key)));
        }
        assert!(!sparse.contains(&Key(65)));
    }

    #[test]
    fn direct_sidecar_merge_unions_dense_sparse_and_empty_partitions() {
        fn check<const B: usize>(left: &[u128], right: &[u128], dense: bool) {
            for (left, right) in [(left, right), (right, left)] {
                let mut a = AttachedDenseMembership::<u128, B>::sidecar_builder();
                let mut b = AttachedDenseMembership::<u128, B>::sidecar_builder();
                for key in left {
                    a.observe(key);
                }
                for key in right {
                    b.observe(key);
                }
                a.merge(b);
                a.merge(AttachedDenseMembership::<u128, B>::sidecar_builder());
                let filter = a.finish();
                assert_eq!(filter.is_dense(), dense);
                for key in left.iter().chain(right) {
                    assert!(filter.contains(key));
                }
                for key in [0, 1, 62, 63, 64, 65, 127, 128, 129, 191, u128::MAX] {
                    assert_eq!(filter.contains(&key), left.contains(&key) || right.contains(&key));
                }
            }
        }
        check::<256>(&[63, 128, 191], &[64, 128, 129], true);
        check::<2>(&[63, 64], &[127, 128], false);
        check::<128>(&[63, 64], &[0, u128::MAX], false);
        check::<128>(&[], &[u128::MAX - 1, u128::MAX], true);
        check::<0>(&[63, 64], &[64, u128::MAX], false);
        check::<128>(&[], &[], false);
    }

    #[cfg(feature = "counters")]
    #[test]
    fn direct_bitmap_build_and_merge_have_no_hash_or_staging_cost() {
        crate::counters::reset();
        let mut a = AttachedDenseMembership::<i64, 256>::sidecar_builder();
        let mut b = AttachedDenseMembership::<i64, 256>::sidecar_builder();
        for key in [130, 63, 0, -1] {
            a.observe(&key);
        }
        for key in [64, 191, 0] {
            b.observe(&key);
        }
        a.merge(b);
        let filter = a.finish();
        let counts = crate::counters::snapshot();
        assert_eq!(counts.index_builds, 1);
        assert_eq!(counts.distinct_insertions, 7);
        assert_eq!(counts.hash_computations, 0);
        assert_eq!(counts.keyset_staging_bytes_peak, 0);
        assert_eq!(counts.bytes_retained_current, filter.retained_bytes() as u64);
        drop(filter);
        assert_eq!(crate::counters::snapshot().bytes_retained_current, 0);
    }

    #[cfg(feature = "counters")]
    #[test]
    fn ordinal_fallback_releases_bitmap_and_all_retained_memory() {
        crate::counters::reset();
        let mut builder = AttachedDenseMembership::<u128, 128>::sidecar_builder();
        builder.observe(&63);
        builder.observe(&64);
        builder.observe(&u128::MAX);
        let filter = builder.finish();
        assert!(!filter.is_dense());
        let counts = crate::counters::snapshot();
        assert_eq!(counts.index_builds, 1);
        assert_eq!(counts.distinct_insertions, 3);
        assert!(counts.hash_computations > 0);
        assert_eq!(counts.keyset_staging_bytes_peak, 0);
        assert_eq!(counts.bytes_retained_current, filter.retained_bytes() as u64);
        assert!(counts.bytes_retained_peak >= counts.bytes_retained_current);
        drop(filter);
        assert_eq!(crate::counters::snapshot().bytes_retained_current, 0);
    }

    #[test]
    fn generic_sidecar_hash_collisions_remain_exact() {
        #[derive(Clone, Eq, PartialEq)]
        struct Colliding(&'static str);
        impl std::hash::Hash for Colliding {
            fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
                state.write_u8(0);
            }
        }
        let mut builder = AdaptiveMembership::<Colliding, 64>::sidecar_builder();
        for key in [Colliding("a"), Colliding("b"), Colliding("a")] {
            builder.observe(&key);
        }
        let filter = builder.finish();
        assert!(filter.contains(&Colliding("a")));
        assert!(filter.contains(&Colliding("b")));
        assert!(!filter.contains(&Colliding("c")));
    }
}
