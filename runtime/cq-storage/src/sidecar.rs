//! Selected sidecars observe borrowed keys during the owning index's build.
//! Publication returns the completed index and sidecars together; observing a
//! prefix is never sufficient to publish a filter for the completed index.

/// Incremental construction of one selected filter or a tuple of filters.
pub trait SidecarBuilder<K> {
    type Output;

    fn observe(&mut self, key: &K);
    fn finish(self) -> Self::Output;
}

/// Combine observations from independently built partitions before publication.
/// Both builders must use the same selected parameters and key projection.
pub trait MergeSidecarBuilder<K>: SidecarBuilder<K> {
    fn merge(&mut self, other: Self);
}

impl<K> SidecarBuilder<K> for () {
    type Output = ();

    fn observe(&mut self, _: &K) {}
    fn finish(self) {}
}

impl<K> MergeSidecarBuilder<K> for () {
    fn merge(&mut self, _: Self) {}
}

/// Borrow an unfinished observer while an exact partition is constructed.
impl<K, S: SidecarBuilder<K>> SidecarBuilder<K> for &mut S {
    type Output = ();

    fn observe(&mut self, key: &K) {
        (**self).observe(key);
    }

    fn finish(self) {}
}

/// A filter that can be attached to an exact index's construction.
pub trait BuildSidecar<K>: Sized {
    type Builder: SidecarBuilder<K, Output = Self>;

    fn sidecar_builder() -> Self::Builder;
}

/// The stored key of a key/payload construction item.
///
/// This links the observer's input to the iterator's key, rather than to an
/// independently chosen generic parameter on the construction operation.
pub trait SidecarInput {
    type Key;
}

impl<K, V> SidecarInput for (K, V) {
    type Key = K;
}

/// Select construction-time scalar work without changing the provider's
/// existing eager/lazy boundary. Direct `Build` retains its public defaults.
pub trait BuildWithStatistics<I: Iterator>: Sized {
    fn build_with_statistics<const ROWS: bool, const MAX_GROUP: bool>(source: I) -> Self;
}

/// Build an exact owner and its selected sidecars in the same source pass.
pub trait BuildWithSidecar<I: Iterator, S>: Sized
where
    I::Item: SidecarInput,
    S: SidecarBuilder<<I::Item as SidecarInput>::Key>,
{
    fn build_with_sidecar<const ROWS: bool, const MAX_GROUP: bool>(source: I, sidecar: S) -> (Self, S::Output);
}

impl<K: Ord, V, I: Iterator<Item = (K, V)>, S: SidecarBuilder<K>>
    BuildWithSidecar<I, S> for std::collections::BTreeMap<K, Vec<V>>
{
    fn build_with_sidecar<const ROWS: bool, const MAX_GROUP: bool>(source: I, mut sidecar: S) -> (Self, S::Output) {
        let index = crate::Build::build(source.inspect(|(key, _)| sidecar.observe(key)));
        (index, sidecar.finish())
    }
}

/// Borrow a selected scalar component without copying the stored composite key.
pub struct ProjectedSidecar<B, StoredKey, Scalar> {
    builder: B,
    projection: fn(&StoredKey) -> &Scalar,
}

impl<B, StoredKey, Scalar> ProjectedSidecar<B, StoredKey, Scalar> {
    pub fn new(builder: B, projection: fn(&StoredKey) -> &Scalar) -> Self {
        Self {
            builder,
            projection,
        }
    }
}

impl<B: SidecarBuilder<Scalar>, StoredKey, Scalar> SidecarBuilder<StoredKey>
    for ProjectedSidecar<B, StoredKey, Scalar>
{
    type Output = B::Output;

    #[inline]
    fn observe(&mut self, key: &StoredKey) {
        self.builder.observe((self.projection)(key));
    }

    fn finish(self) -> Self::Output {
        self.builder.finish()
    }
}

impl<B: MergeSidecarBuilder<Scalar>, StoredKey, Scalar> MergeSidecarBuilder<StoredKey>
    for ProjectedSidecar<B, StoredKey, Scalar>
{
    fn merge(&mut self, other: Self) {
        self.builder.merge(other.builder);
    }
}

// Pairs can nest to attach arbitrarily many scalar filters in one source pass.
impl<K, A: SidecarBuilder<K>, B: SidecarBuilder<K>> SidecarBuilder<K> for (A, B) {
    type Output = (A::Output, B::Output);

    #[inline]
    fn observe(&mut self, key: &K) {
        self.0.observe(key);
        self.1.observe(key);
    }

    fn finish(self) -> Self::Output {
        (self.0.finish(), self.1.finish())
    }
}

impl<K, A: MergeSidecarBuilder<K>, B: MergeSidecarBuilder<K>> MergeSidecarBuilder<K>
    for (A, B)
{
    fn merge(&mut self, other: Self) {
        self.0.merge(other.0);
        self.1.merge(other.1);
    }
}

#[cfg(test)]
mod tests {
    use super::{BuildSidecar, MergeSidecarBuilder, ProjectedSidecar, SidecarBuilder};
    use crate::{AdaptiveMembership, AttachedDenseMembership, ExactMembership};

    #[test]
    fn explicit_ordered_map_sidecar_preserves_every_occurrence() {
        use super::BuildWithSidecar;
        let rows = [(1_i64, 7), (1, 8), (9, 3)];
        let (index, filter) = std::collections::BTreeMap::<_, Vec<_>>::build_with_sidecar::<true, true>(
            rows.into_iter(), AttachedDenseMembership::<i64, 64>::sidecar_builder(),
        );
        assert_eq!(index[&1], [7, 8]);
        assert_eq!(index[&9], [3]);
        assert!(filter.contains(&1));
        assert!(filter.contains(&9));
        assert!(!filter.contains(&8));
    }

    #[test]
    fn nested_projected_sidecars_borrow_nonclone_composite_keys() {
        struct Key(i64, u64);
        let keys = [Key(-3, 10), Key(2, 20), Key(-3, 30)];
        let mut builders = (
            ProjectedSidecar::new(AttachedDenseMembership::<i64, 64>::sidecar_builder(), |k: &Key| &k.0),
            (
                ProjectedSidecar::new(AdaptiveMembership::<u64, 64>::sidecar_builder(), |k: &Key| &k.1),
                ProjectedSidecar::new(AttachedDenseMembership::<u64, 64>::sidecar_builder(), |k: &Key| &k.1),
            ),
        );
        for key in &keys {
            builders.observe(key);
        }
        let (first, (second, third)) = builders.finish();
        for key in &keys {
            assert!(first.contains(&key.0));
            assert!(second.contains(&key.1));
            assert!(third.contains(&key.1));
        }
        assert!(!first.contains(&0));
        assert!(!second.contains(&11));
        assert!(!third.contains(&11));
    }

    #[test]
    fn partition_observers_merge_before_finalizing_recursive_filters() {
        struct Key(i64, u64);
        let make = || {
            (
                ProjectedSidecar::new(AttachedDenseMembership::<i64, 64>::sidecar_builder(), |k: &Key| &k.0),
                (
                    ProjectedSidecar::new(AdaptiveMembership::<u64, 64>::sidecar_builder(), |k: &Key| &k.1),
                    (),
                ),
            )
        };
        let mut left = make();
        let mut right = make();
        // Finishing a borrowed observer does not finalize its underlying filter.
        let observer = &mut left;
        observer.observe(&Key(-3, 10));
        SidecarBuilder::<Key>::finish(observer);
        right.observe(&Key(2, 20));
        right.observe(&Key(-3, 10));
        left.merge(right);
        left.merge(make());
        let (first, (second, ())) = left.finish();
        assert!(first.is_dense());
        assert!(first.contains(&-3));
        assert!(first.contains(&2));
        assert!(!first.contains(&0));
        assert!(second.contains(&10));
        assert!(second.contains(&20));
        assert!(!second.contains(&15));
    }

    #[cfg(feature = "counters")]
    #[test]
    fn dense_sidecar_builds_bitmap_without_hashing_or_key_staging() {
        crate::counters::reset();
        let mut builder = AttachedDenseMembership::<i64, 64>::sidecar_builder();
        for key in [1, 2, 1] {
            builder.observe(&key);
        }
        let filter = builder.finish();
        let counts = crate::counters::snapshot();
        assert_eq!(counts.index_builds, 1);
        assert_eq!(counts.distinct_insertions, 3);
        assert_eq!(counts.bytes_retained_current, filter.retained_bytes() as u64);
        assert_eq!(counts.keyset_staging_bytes_peak, 0);
        assert_eq!(counts.hash_computations, 0);
        assert!(counts.bytes_retained_peak >= counts.bytes_retained_current);
        drop(filter);
        assert_eq!(crate::counters::snapshot().bytes_retained_current, 0);
    }
}
