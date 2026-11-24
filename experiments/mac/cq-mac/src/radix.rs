//! GPU-built grouped RadixRun using the native storage reader verbatim.
use super::{IndexSpec, MAX_ROWS, MacContext, MacError, Payload, Relation, platform};
use cq_storage::{
    RadixRun, RadixRunReadState, RadixRunView, RadixSplineMetadata, RadixSplinePoint, RunCursor,
};
pub use mccl::radix::RadixJoin;
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

enum SourceIndex {
    Cpu(RadixRun<u32, u32>),
    Gpu(MacRadixRun),
}

struct IndexedSource {
    source: Relation,
    index: SourceIndex,
    values: *const i64,
    value_count: usize,
}

/// An immutable i64 relation bound to its all-row index on `KEY_COLUMN`.
///
/// Both explicit preparation paths retain the original source, index original
/// row IDs, and preserve source chronology for duplicate keys. Only indexed
/// keys must fit u32; every source field remains full-width i64. There is no
/// public attachment operation that could pair an index with a different source.
/// Clones share ownership; reads never allocate, call FFI, or retain an owner.
#[derive(Clone)]
pub struct IndexedI64Relation<const KEY_COLUMN: usize> {
    inner: Rc<IndexedSource>,
}

impl<const KEY_COLUMN: usize> fmt::Debug for IndexedI64Relation<KEY_COLUMN> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IndexedI64Relation")
            .field("key_column", &KEY_COLUMN)
            .field("rows", &self.len())
            .field("columns", &self.columns())
            .finish_non_exhaustive()
    }
}

impl<const KEY_COLUMN: usize> IndexedI64Relation<KEY_COLUMN> {
    // Only preparation calls this after checking the source and building its
    // all-row index. The mapping is immutable and pinned by the source owner.
    fn prepared(source: &Relation, values: &[i64], index: SourceIndex) -> Self {
        Self {
            inner: Rc::new(IndexedSource {
                source: source.clone(),
                index,
                values: values.as_ptr(),
                value_count: values.len(),
            }),
        }
    }

    /// Number of source rows, including duplicate keys.
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.source.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn columns(&self) -> usize {
        self.inner.source.columns()
    }

    /// Borrow the original row-major source without evaluating it again.
    #[inline]
    pub fn values(&self) -> &[i64] {
        // Preparation caches a checked, fully initialized immutable mapping.
        // The original Relation strongly owns that allocation for this entire
        // borrow, including after the caller's source and context are dropped.
        unsafe { std::slice::from_raw_parts(self.inner.values, self.inner.value_count) }
    }

    /// Borrow a full-width source row. Panics if `row_id` is out of bounds.
    #[inline]
    pub fn row(&self, row_id: u32) -> &[i64] {
        let row_id = row_id as usize;
        assert!(row_id < self.len(), "source row ID is out of bounds");
        let columns = self.columns();
        let start = row_id * columns;
        &self.values()[start..start + columns]
    }

    /// Original row IDs in source order, or `None` for exact absence.
    ///
    /// Out-of-u32-domain probes cannot match the validated source; they never
    /// truncate or panic. The native cursor is Copy and exact-size.
    #[inline]
    pub fn row_ids(&self, key: i64) -> Option<RunCursor<'_, u32>> {
        let key = u32::try_from(key).ok()?;
        let cursor = match &self.inner.index {
            SourceIndex::Cpu(index) => {
                let (begin, end) = index.range(&key);
                RunCursor::from_slice(&index.items()[begin..end])
            }
            SourceIndex::Gpu(index) => index.lookup(&key),
        };
        (cursor.len() != 0).then_some(cursor)
    }
}

impl MacContext {
    fn radix_i64_values<'a, const KEY_COLUMN: usize>(
        &self,
        source: &'a Relation,
    ) -> Result<&'a [i64], MacError> {
        if !source.inner.belongs_to(&self.inner.0) {
            return Err(MacError::ContextMismatch);
        }
        if source.len() > MAX_ROWS {
            return Err(MacError::CapacityExceeded);
        }
        if source.columns() == 0 || KEY_COLUMN >= source.columns() {
            return Err(MacError::CommandFailed(
                "i64 radix key column is outside the source shape".into(),
            ));
        }
        let value_count = source
            .len()
            .checked_mul(source.columns())
            .filter(|&n| n <= isize::MAX as usize / size_of::<i64>())
            .ok_or(MacError::CapacityExceeded)?;
        // This checks dtype and maps the source once, without inspecting keys.
        // GPU key-domain validation remains in MCCL's device projection.
        let values = source.i64_values()?;
        if values.len() != value_count {
            return Err(MacError::CommandFailed(
                "i64 radix source has an invalid shape".into(),
            ));
        }
        Ok(values)
    }

    /// Explicit GPU preparation over every source row, with device-side key
    /// validation, projection, stable grouping, spline and radix construction.
    /// Source ingestion is separate; non-key source fields are never narrowed.
    pub fn prepare_radix_i64<const KEY_COLUMN: usize>(
        &self,
        source: &Relation,
    ) -> Result<IndexedI64Relation<KEY_COLUMN>, MacError> {
        let values = self.radix_i64_values::<KEY_COLUMN>(source)?;
        let index = self.build_radix(
            source,
            IndexSpec {
                key_column: KEY_COLUMN,
                payload: Payload::RowId,
            },
            None,
        )?;
        Ok(IndexedI64Relation::prepared(
            source,
            values,
            SourceIndex::Gpu(index),
        ))
    }

    /// Explicit native CPU RadixRun baseline over exactly the same source.
    /// Validation precedes construction; pairs stream directly into the native
    /// builder without an intermediate projection buffer or pair-vector copy.
    pub fn prepare_radix_i64_on_cpu<const KEY_COLUMN: usize>(
        &self,
        source: &Relation,
    ) -> Result<IndexedI64Relation<KEY_COLUMN>, MacError> {
        let values = self.radix_i64_values::<KEY_COLUMN>(source)?;
        let columns = source.columns();
        if values
            .chunks_exact(columns)
            .any(|row| u32::try_from(row[KEY_COLUMN]).is_err())
        {
            return Err(MacError::CommandFailed(
                "i64 radix source key is outside the u32 domain".into(),
            ));
        }
        let index = RadixRun::from_pairs(
            values
                .chunks_exact(columns)
                .enumerate()
                // Both casts were checked above: all keys fit u32, and the
                // all-row payload count is bounded by MAX_ROWS.
                .map(|(row_id, row)| (row[KEY_COLUMN] as u32, row_id as u32)),
        );
        Ok(IndexedI64Relation::prepared(
            source,
            values,
            SourceIndex::Cpu(index),
        ))
    }

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

    /// Exact batched equi-join of `probes` against `run`, executed on the GPU.
    /// Pairs are `[probe ordinal, item]` in probe order, then stored
    /// occurrence order. Totals above [`MAX_ROWS`] are rejected, not truncated.
    pub fn join_radix(&self, run: &MacRadixRun, probes: &[u32]) -> Result<RadixJoin, MacError> {
        if !run.inner.index.belongs_to(&self.inner.0) {
            return Err(MacError::ContextMismatch);
        }
        if probes.len() > MAX_ROWS {
            return Err(MacError::CapacityExceeded);
        }
        let probes = self.inner.0.upload(probes).map_err(platform::error)?;
        self.inner
            .0
            .join_radix(&run.inner.index, &probes)
            .map_err(platform::error)
    }
}

/// Completed GPU join of i64 probe keys against a source-bound grouped run.
#[derive(Debug)]
pub struct RowIdJoin {
    join: RadixJoin,
    /// Caller probe ordinal of each uploaded probe; `None` when every probe
    /// was in the u32 key domain and ordinals coincide.
    ordinals: Option<Vec<u32>>,
}

impl RowIdJoin {
    pub fn len(&self) -> usize {
        self.join.len()
    }

    pub fn is_empty(&self) -> bool {
        self.join.is_empty()
    }

    /// `(probe ordinal, source row ID)` in probe order, then source chronology.
    pub fn pairs(&self) -> impl ExactSizeIterator<Item = (u32, u32)> + '_ {
        self.join.pairs().iter().map(|&[probe, row_id]| {
            let probe = self
                .ordinals
                .as_ref()
                .map_or(probe, |ordinals| ordinals[probe as usize]);
            (probe, row_id)
        })
    }
}

impl<const KEY_COLUMN: usize> IndexedI64Relation<KEY_COLUMN> {
    /// Original source row IDs matching each probe, joined on the GPU.
    ///
    /// Requires the GPU-built index. As with [`Self::row_ids`], probes outside
    /// the u32 key domain are exact misses; they are never narrowed or uploaded.
    pub fn join_row_ids(
        &self,
        context: &MacContext,
        probes: &[i64],
    ) -> Result<RowIdJoin, MacError> {
        let SourceIndex::Gpu(run) = &self.inner.index else {
            return Err(MacError::CommandFailed(
                "GPU join requires a GPU-built source index".into(),
            ));
        };
        if probes.len() > MAX_ROWS {
            return Err(MacError::CapacityExceeded);
        }
        let mut keys = Vec::with_capacity(probes.len());
        let mut ordinals: Option<Vec<u32>> = None;
        for (ordinal, &probe) in probes.iter().enumerate() {
            match u32::try_from(probe) {
                Ok(key) => {
                    if let Some(ordinals) = &mut ordinals {
                        ordinals.push(ordinal as u32);
                    }
                    keys.push(key);
                }
                Err(_) => {
                    ordinals.get_or_insert_with(|| (0..ordinal as u32).collect());
                }
            }
        }
        Ok(RowIdJoin {
            join: context.join_radix(run, &keys)?,
            ordinals,
        })
    }
}

/// Source row IDs kept on the GPU by [`IndexedI64Relation::semijoin`].
pub struct GpuRows {
    rows: mccl::U32Array,
}

impl GpuRows {
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Kept original row IDs, in input order.
    pub fn row_ids(&self) -> Result<&[u32], MacError> {
        self.rows.as_slice().map_err(platform::error)
    }
}

/// Rows entering a GPU semijoin: a row-ID join's matches or earlier kept rows.
#[derive(Clone, Copy)]
pub enum GpuRowsInput<'a> {
    Join(&'a RowIdJoin),
    Rows(&'a GpuRows),
}

impl<const KEY_COLUMN: usize> IndexedI64Relation<KEY_COLUMN> {
    /// Keep this source's rows whose `key_column` value occurs in `run`; with
    /// `item_column`, that key's group must contain the row's `item_column`
    /// value. Runs on the GPU over full-width i64 fields; values outside u32
    /// are misses. Chained filters never copy row IDs back to the host.
    pub fn semijoin(
        &self,
        context: &MacContext,
        input: GpuRowsInput<'_>,
        key_column: usize,
        item_column: Option<usize>,
        run: &MacRadixRun,
    ) -> Result<GpuRows, MacError> {
        let gpu = &context.inner.0;
        if !self.inner.source.inner.belongs_to(gpu) || !run.inner.index.belongs_to(gpu) {
            return Err(MacError::ContextMismatch);
        }
        gpu.semijoin_rows(
            input.rows(),
            &self.inner.source.inner,
            key_column,
            item_column,
            &run.inner.index,
        )
        .map(|rows| GpuRows { rows })
        .map_err(platform::error)
    }
}

impl<'a> GpuRowsInput<'a> {
    fn rows(self) -> mccl::radix::SemiJoinRows<'a> {
        match self {
            GpuRowsInput::Join(join) => mccl::radix::SemiJoinRows::Join(&join.join),
            GpuRowsInput::Rows(rows) => mccl::radix::SemiJoinRows::Rows(&rows.rows),
        }
    }
}

/// Distinct `W`-column tuples from [`IndexedI64Relation::distinct`], in
/// ascending lexicographic order. Zero-copy view of the GPU result.
pub struct GpuTuples<const W: usize> {
    tuples: mccl::radix::DistinctRows,
}

impl<const W: usize> GpuTuples<W> {
    pub fn len(&self) -> usize {
        self.tuples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tuples.is_empty()
    }

    pub fn tuples(&self) -> &[[i64; W]] {
        let (tuples, rest) = self.tuples.values().as_chunks::<W>();
        debug_assert!(rest.is_empty());
        tuples
    }
}

impl<const KEY_COLUMN: usize> IndexedI64Relation<KEY_COLUMN> {
    /// Distinct full-width tuples of the input rows' `columns`, sorted and
    /// deduplicated on the GPU; only the answers return. Every projected value
    /// must lie in the u32 domain: a wider value is an error, never narrowed.
    pub fn distinct<const W: usize>(
        &self,
        context: &MacContext,
        input: GpuRowsInput<'_>,
        columns: [usize; W],
    ) -> Result<GpuTuples<W>, MacError> {
        let gpu = &context.inner.0;
        if !self.inner.source.inner.belongs_to(gpu) {
            return Err(MacError::ContextMismatch);
        }
        gpu.distinct_rows(input.rows(), &self.inner.source.inner, &columns)
            .map(|tuples| GpuTuples { tuples })
            .map_err(platform::error)
    }
}

/// One GPU semijoin of [`IndexedI64Relation::join_semijoin_distinct`].
#[derive(Clone, Copy)]
pub struct GpuSemiJoin<'a> {
    pub key_column: usize,
    pub item_column: Option<usize>,
    pub run: &'a MacRadixRun,
}

impl<const KEY_COLUMN: usize> IndexedI64Relation<KEY_COLUMN> {
    /// [`Self::join_row_ids`], then each [`Self::semijoin`] in order, then
    /// [`Self::distinct`], as one GPU graph: only the join's pair count is read
    /// back on the host before the answers. Probes outside the u32 key domain
    /// are exact misses, never narrowed.
    pub fn join_semijoin_distinct<const W: usize>(
        &self,
        context: &MacContext,
        probes: &[i64],
        steps: &[GpuSemiJoin<'_>],
        columns: [usize; W],
    ) -> Result<GpuTuples<W>, MacError> {
        let gpu = &context.inner.0;
        let SourceIndex::Gpu(run) = &self.inner.index else {
            return Err(MacError::CommandFailed(
                "GPU join requires a GPU-built source index".into(),
            ));
        };
        if !self.inner.source.inner.belongs_to(gpu)
            || !run.inner.index.belongs_to(gpu)
            || steps.iter().any(|step| !step.run.inner.index.belongs_to(gpu))
        {
            return Err(MacError::ContextMismatch);
        }
        if probes.len() > MAX_ROWS {
            return Err(MacError::CapacityExceeded);
        }
        let keys: Vec<u32> = probes.iter().filter_map(|&p| u32::try_from(p).ok()).collect();
        let keys = gpu.upload(&keys).map_err(platform::error)?;
        let steps: Vec<_> = steps
            .iter()
            .map(|step| mccl::radix::SemiJoinStep {
                index: &step.run.inner.index,
                key_column: step.key_column,
                item_column: step.item_column,
            })
            .collect();
        gpu.join_semijoin_distinct(
            &run.inner.index,
            &keys,
            &self.inner.source.inner,
            &steps,
            &columns,
        )
        .map(|tuples| GpuTuples { tuples })
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
