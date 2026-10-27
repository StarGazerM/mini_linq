//! Append-only ownership of completed MCCL runs and immutable query snapshots.

use std::iter::FusedIterator;
use std::rc::Rc;

use cq_storage::{ExactMembership, IndexRead};

use crate::backend::{IndexSpec, MAX_ROWS, MacContext, MacError, MacRun, Relation, RunCursor};

// Every nonempty batch contributes at least one row. The total row limit thus
// bounds binary-carry ranks as well as buffer sizes.
const MAX_RUNS: usize = MAX_ROWS.ilog2() as usize + 1;

/// Compaction policy for chronological occurrence bags.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MaintenancePolicy {
    /// Merge every accepted batch into one complete run.
    Flat,
    /// Carry equal batch-count ranks, retaining at most one run per level.
    BinaryCarry,
}

#[derive(Clone)]
struct RankedRun {
    run: MacRun,
    first_batch: u64,
    last_batch: u64,
    // Binary-carry rank counts batches, not rows. Unused by flat compaction.
    level: u32,
}

struct Manifest {
    epoch: u64,
    len: usize,
    // Chronological order, which is descending rank for binary carries.
    runs: Vec<RankedRun>,
}

/// An immutable, cheaply cloned manifest owning all of its completed runs.
///
/// Snapshots remain readable after subsequent appends, compaction, or disposal
/// of both the writer and the context that built their buffers.
#[derive(Clone)]
pub struct Snapshot {
    manifest: Rc<Manifest>,
}

impl Snapshot {
    /// Number of successfully published nonempty batches.
    pub fn epoch(&self) -> u64 {
        self.manifest.epoch
    }

    /// Total number of occurrences, including duplicate keys and payloads.
    pub fn len(&self) -> usize {
        self.manifest.len
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn run_count(&self) -> usize {
        self.manifest.runs.len()
    }

    /// Look up every occupied run once and concatenate matches chronologically.
    ///
    /// The cursor borrows this pinned snapshot. Probing neither allocates nor
    /// clones reference-counted owners, and iteration performs no more searches.
    pub fn lookup(&self, key: &u32) -> SnapshotCursor<'_> {
        let mut cursor = SnapshotCursor {
            segments: [None; MAX_RUNS],
            position: 0,
            remaining: 0,
        };
        let mut segment_count = 0;
        for ranked in &self.manifest.runs {
            let segment = ranked.run.lookup(key);
            let len = segment.len();
            if len != 0 {
                cursor.segments[segment_count] = Some(segment);
                segment_count += 1;
                // Matching occurrences are disjoint subsets of manifest.len,
                // whose addition was checked before publication.
                cursor.remaining += len;
            }
        }
        cursor
    }

    pub fn contains(&self, key: &u32) -> bool {
        self.manifest
            .runs
            .iter()
            .any(|ranked| ranked.run.contains(key))
    }
}

impl<'a> IndexRead for &'a Snapshot {
    type Key = u32;
    type Item = u32;
    type Cursor = SnapshotCursor<'a>;

    fn lookup(&self, key: &u32) -> Self::Cursor {
        Snapshot::lookup(self, key)
    }
}

impl ExactMembership<u32> for Snapshot {
    fn contains(&self, key: &u32) -> bool {
        Snapshot::contains(self, key)
    }
}

/// A bounded collection of borrowed run segments, not a contiguous range.
#[derive(Clone, Copy)]
pub struct SnapshotCursor<'a> {
    segments: [Option<RunCursor<'a>>; MAX_RUNS],
    position: usize,
    remaining: usize,
}

impl Iterator for SnapshotCursor<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let segment = self.segments[self.position]
            .as_mut()
            .expect("remaining occurrences have an occupied segment");
        let item = segment.next().expect("occupied segments are nonempty");
        self.remaining -= 1;
        if segment.len() == 0 {
            self.position += 1;
        }
        Some(item)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }

    fn count(self) -> usize {
        self.remaining
    }
}

impl ExactSizeIterator for SnapshotCursor<'_> {
    fn len(&self) -> usize {
        self.remaining
    }
}

impl FusedIterator for SnapshotCursor<'_> {}

/// The sole publisher of an append-only sequence of immutable manifests.
pub struct MaintainedIndex {
    policy: MaintenancePolicy,
    current: Snapshot,
}

impl MaintainedIndex {
    pub fn new(policy: MaintenancePolicy) -> Self {
        Self {
            policy,
            current: Snapshot {
                manifest: Rc::new(Manifest {
                    epoch: 0,
                    len: 0,
                    runs: Vec::new(),
                }),
            },
        }
    }

    /// Pin the current manifest with one reference-counted clone.
    pub fn snapshot(&self) -> Snapshot {
        self.current.clone()
    }

    /// Build and compact synchronously, publishing only after every GPU call
    /// succeeds. Empty batches are no-ops and retain the same manifest and epoch.
    /// Nonempty appends must use the originating context of every retained run,
    /// even on a binary-carry step that would not merge. Mismatch leaves the
    /// current manifest unchanged; empty appends remain unconditional no-ops.
    pub fn append(
        &mut self,
        context: &MacContext,
        source: &Relation,
        spec: IndexSpec,
    ) -> Result<Snapshot, MacError> {
        if !source.is_empty()
            && self
                .current
                .manifest
                .runs
                .iter()
                .any(|run| !context.owns_run(&run.run))
        {
            return Err(MacError::ContextMismatch);
        }
        self.prepare_append(
            source.len(),
            || context.build(source, spec, None),
            |older, newer| context.merge(older, newer),
        )
    }

    // Production callers always use the real context above. Keeping preparation
    // private and parameterized lets unit tests inject a failure between real
    // completed carries, without a fake backend or a public bypass.
    fn prepare_append<B, M>(
        &mut self,
        count: usize,
        build: B,
        mut merge: M,
    ) -> Result<Snapshot, MacError>
    where
        B: FnOnce() -> Result<MacRun, MacError>,
        M: FnMut(&MacRun, &MacRun) -> Result<MacRun, MacError>,
    {
        if count == 0 {
            return Ok(self.snapshot());
        }
        let current = &self.current.manifest;
        let epoch = current
            .epoch
            .checked_add(1)
            .ok_or(MacError::CapacityExceeded)?;
        let len = current
            .len
            .checked_add(count)
            .ok_or(MacError::CapacityExceeded)?;
        if len > MAX_ROWS {
            return Err(MacError::CapacityExceeded);
        }

        // Assign the accepted batch's interval and rank exactly once. All
        // subsequent work owns only unpublished runs, never mutable old buffers.
        let mut pending = RankedRun {
            run: build()?,
            first_batch: epoch,
            last_batch: epoch,
            level: 0,
        };
        let mut retained = current.runs.len();
        match self.policy {
            MaintenancePolicy::Flat => {
                if let Some(older) = current.runs.first() {
                    pending.run = merge(&older.run, &pending.run)?;
                    pending.first_batch = older.first_batch;
                    retained = 0;
                }
            }
            MaintenancePolicy::BinaryCarry => {
                while retained != 0 {
                    let older = &current.runs[retained - 1];
                    if older.level != pending.level {
                        break;
                    }
                    debug_assert_eq!(older.last_batch.checked_add(1), Some(pending.first_batch));
                    pending.run = merge(&older.run, &pending.run)?;
                    pending.first_batch = older.first_batch;
                    pending.level += 1;
                    retained -= 1;
                }
            }
        }

        // Clone only surviving run owners, after every fallible GPU operation.
        // No old manifest is changed, even if any earlier carry failed.
        let mut runs = Vec::with_capacity(retained + 1);
        runs.extend_from_slice(&current.runs[..retained]);
        runs.push(pending);
        let snapshot = Snapshot {
            manifest: Rc::new(Manifest { epoch, len, runs }),
        };
        self.current = snapshot.clone();
        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "macos")]
    fn failed_second_carry_preserves_publication_and_retry_order() {
        let context = MacContext::new().expect("a Metal device is required");
        let mut index = MaintainedIndex::new(MaintenancePolicy::BinaryCarry);
        let spec = IndexSpec {
            key_column: 0,
            payload: crate::Payload::Column(1),
        };
        let first = context
            .relation_u32(3, |i| [[7, 10], [7, 10], [3, 90]][i])
            .unwrap();
        let second = context.relation_u32(1, |_| [7, 20]).unwrap();
        let third = context.relation_u32(2, |i| [[9, 1], [7, 30]][i]).unwrap();
        index.append(&context, &first, spec).unwrap();
        index.append(&context, &second, spec).unwrap();
        index.append(&context, &third, spec).unwrap();
        let pinned = index.snapshot();
        let retired = Rc::downgrade(&pinned.manifest);
        let mut retained_cursor = pinned.lookup(&7);
        assert_eq!(retained_cursor.next(), Some(10));
        let fourth = context.relation_u32(2, |_| [7, 40]).unwrap();
        let mut merge_calls = 0;
        let failed = index.prepare_append(
            fourth.len(),
            || context.build(&fourth, spec, None),
            |older, newer| {
                merge_calls += 1;
                if merge_calls == 2 {
                    return Err(MacError::CapacityExceeded);
                }
                context.merge(older, newer)
            },
        );
        assert!(matches!(failed, Err(MacError::CapacityExceeded)));
        assert_eq!(
            merge_calls, 2,
            "the first real carry must complete before failure"
        );
        assert!(Rc::ptr_eq(&index.current.manifest, &pinned.manifest));
        assert_eq!(index.snapshot().epoch(), 3);
        assert_eq!(index.snapshot().len(), 6);
        assert_eq!(pinned.lookup(&7).collect::<Vec<_>>(), [10, 10, 20, 30]);
        assert_eq!(pinned.lookup(&3).collect::<Vec<_>>(), [90]);
        assert_eq!(pinned.lookup(&9).collect::<Vec<_>>(), [1]);

        let latest = index.append(&context, &fourth, spec).unwrap();
        assert_eq!(latest.epoch(), 4);
        assert_eq!(latest.len(), 8);
        assert_eq!(latest.run_count(), 1);
        assert_eq!(
            latest.lookup(&7).collect::<Vec<_>>(),
            [10, 10, 20, 30, 40, 40]
        );
        drop(index);
        drop(context);
        assert_eq!(retained_cursor.collect::<Vec<_>>(), [10, 20, 30]);
        assert_eq!(pinned.lookup(&7).collect::<Vec<_>>(), [10, 10, 20, 30]);
        assert!(retired.upgrade().is_some());
        drop(pinned);
        assert!(
            retired.upgrade().is_none(),
            "the old manifest is no longer retained"
        );
        assert_eq!(
            latest.lookup(&7).collect::<Vec<_>>(),
            [10, 10, 20, 30, 40, 40]
        );
    }

    #[test]
    fn empty_append_is_identical_even_at_capacity() {
        let mut index = MaintainedIndex::new(MaintenancePolicy::BinaryCarry);
        index.current.manifest = Rc::new(Manifest {
            epoch: u64::MAX,
            len: usize::MAX,
            runs: Vec::new(),
        });
        let before = index.snapshot();
        let after = index
            .prepare_append(
                0,
                || panic!("empty append must not build"),
                |_, _| panic!("empty append must not merge"),
            )
            .unwrap();
        assert!(Rc::ptr_eq(&before.manifest, &after.manifest));
        assert_eq!(after.epoch(), u64::MAX);
    }

    #[test]
    fn capacity_checks_precede_gpu_work_and_publication() {
        for policy in [MaintenancePolicy::Flat, MaintenancePolicy::BinaryCarry] {
            for (epoch, len) in [(u64::MAX, 0), (0, usize::MAX), (0, MAX_ROWS)] {
                let mut index = MaintainedIndex::new(policy);
                index.current.manifest = Rc::new(Manifest {
                    epoch,
                    len,
                    runs: Vec::new(),
                });
                let before = index.snapshot();
                let result = index.prepare_append(
                    1,
                    || panic!("capacity must be checked before build"),
                    |_, _| panic!("capacity must be checked before merge"),
                );
                assert!(matches!(result, Err(MacError::CapacityExceeded)));
                assert!(Rc::ptr_eq(&before.manifest, &index.current.manifest));
            }
        }
    }
}
