//! GPU-built grouped RadixRun using the native storage reader verbatim.
use super::{IndexSpec, MAX_ROWS, MacContext, MacError, Relation, platform};
use cq_storage::{
    RadixRunReadState, RadixRunView, RadixSplineMetadata, RadixSplinePoint, RunCursor,
};
use std::{fmt, rc::Rc};

struct Completed {
    index: mccl::radix::RadixIndex,
    reads: RadixRunReadState,
}

/// Completed GPU-built distinct keys, offsets, payloads and error-bounded spline.
/// Clones pin immutable storage; CPU reads use the same view as native RadixRun.
/// Keys and payloads follow MCCL's explicit u32 domain. Dense membership uses
/// the native lazy bitmap policy, including its first-use initialization cost.
#[derive(Clone)]
pub struct MacRadixRun {
    inner: Rc<Completed>,
}

impl fmt::Debug for MacRadixRun {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MacRadixRun")
            .field("rows", &self.len())
            .field("distinct_keys", &self.keys().len())
            .field("spline_points", &self.inner.index.points().len())
            .finish_non_exhaustive()
    }
}

impl MacRadixRun {
    fn completed(index: mccl::radix::RadixIndex) -> Self {
        let reads = RadixRunReadState::new(index.keys());
        Self {
            inner: Rc::new(Completed { index, reads }),
        }
    }

    pub fn len(&self) -> usize {
        self.inner.index.len()
    }
    pub fn is_empty(&self) -> bool {
        self.inner.index.is_empty()
    }
    pub fn keys(&self) -> &[u32] {
        self.inner.index.keys()
    }
    pub fn offsets(&self) -> &[usize] {
        self.inner.index.offsets()
    }
    pub fn items(&self) -> &[u32] {
        self.inner.index.items()
    }
    pub fn spline_point_count(&self) -> usize {
        self.inner.index.points().len()
    }

    /// Borrow the exact native reader; constructing a view does not allocate,
    /// retain an owner, scan arrays or execute GPU/FFI work.
    #[inline]
    pub fn view(&self) -> RadixRunView<'_, u32, u32> {
        const {
            assert!(size_of::<RadixSplinePoint>() == size_of::<[u64; 2]>());
        }
        const {
            assert!(align_of::<RadixSplinePoint>() == align_of::<[u64; 2]>());
        }
        let index = &self.inner.index;
        // MCCL publishes only initialized immutable prefixes after completion.
        // The u64 key/rank ABI is identical to native repr(C) points on arm64.
        // GPU corridor construction uses exact integer orientation with global
        // ranks; directory and offsets are built against these same arrays.
        unsafe {
            let points = std::slice::from_raw_parts(
                index.points().as_ptr().cast::<RadixSplinePoint>(),
                index.points().len(),
            );
            RadixRunView::from_parts(
                index.keys(),
                index.offsets(),
                index.items(),
                points,
                index.radix(),
                RadixSplineMetadata {
                    min_key: index.min_key(),
                    max_key: index.max_key(),
                    len: index.keys().len(),
                    shift: index.shift(),
                    search_error: index.search_error(),
                },
                &self.inner.reads,
            )
        }
    }

    #[inline]
    pub fn lookup(&self, key: &u32) -> RunCursor<'_, u32> {
        self.view().lookup(key)
    }
    #[inline]
    pub fn contains(&self, key: &u32) -> bool {
        self.view().contains(key)
    }
    #[inline]
    pub fn range(&self, key: &u32) -> (usize, usize) {
        self.view().range(key)
    }
}

impl MacContext {
    /// GPU projection, stable sort, grouping, parallel spline and radix build.
    /// Source ingestion is separate; no CPU grouping or spline finishing pass.
    pub fn build_radix(
        &self,
        source: &Relation,
        spec: IndexSpec,
        selection: Option<&Relation>,
    ) -> Result<MacRadixRun, MacError> {
        if !source.inner.belongs_to(&self.inner.0)
            || selection.is_some_and(|s| !s.inner.belongs_to(&self.inner.0))
        {
            return Err(MacError::ContextMismatch);
        }
        if source.len() > MAX_ROWS {
            return Err(MacError::CapacityExceeded);
        }
        self.inner
            .0
            .build_radix_index(
                &source.inner,
                platform::projection(spec),
                selection.map(|s| &s.inner),
            )
            .map(MacRadixRun::completed)
            .map_err(platform::error)
    }

    /// Stable GPU merge and parallel reconstruction of the grouped spline index.
    /// Equal-key occurrences from older precede newer; pinned inputs never mutate.
    pub fn merge_radix(
        &self,
        older: &MacRadixRun,
        newer: &MacRadixRun,
    ) -> Result<MacRadixRun, MacError> {
        if !older.inner.index.belongs_to(&self.inner.0)
            || !newer.inner.index.belongs_to(&self.inner.0)
        {
            return Err(MacError::ContextMismatch);
        }
        older
            .len()
            .checked_add(newer.len())
            .filter(|&n| n <= MAX_ROWS)
            .ok_or(MacError::CapacityExceeded)?;
        self.inner
            .0
            .merge_radix_indexes(&older.inner.index, &newer.inner.index)
            .map(MacRadixRun::completed)
            .map_err(platform::error)
    }
}

impl<'a> cq_storage::IndexRead for &'a MacRadixRun {
    type Key = u32;
    type Item = u32;
    type Cursor = RunCursor<'a, u32>;
    #[inline]
    fn lookup(&self, key: &u32) -> Self::Cursor {
        MacRadixRun::lookup(self, key)
    }
}
impl cq_storage::ExactMembership<u32> for MacRadixRun {
    #[inline]
    fn contains(&self, key: &u32) -> bool {
        MacRadixRun::contains(self, key)
    }
}
impl cq_storage::Keys for MacRadixRun {
    type Key = u32;
    type Iter<'a> = std::slice::Iter<'a, u32>;
    fn key_count(&self) -> usize {
        self.keys().len()
    }
    fn distinct_keys(&self) -> Self::Iter<'_> {
        self.keys().iter()
    }
}
