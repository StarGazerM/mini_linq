//! Resident, exact Boolean Datalog publication.
//!
//! The baseline rebuilds sets and full snapshots at each publication. It does
//! not retain a spine or share indexes across separately compiled rule holes.
//! All rule holes in an SCC must read the old `full`/`delta` handles; prepare
//! every member's successor before replacing any member. Even empty deltas are
//! published. Tuple data never crosses the device boundary here. One checked
//! batch downloads every member's delta count and ancestor error status before
//! allocating exact-live-sized full snapshots. Their copies are stream ordered;
//! the next batch (including terminal empty batches) checks their statuses.
use crate::*;

thread_local! {
    static NONEMPTY_ROUND_COUNT: Cell<usize> = const { Cell::new(0) };
    static CHECKED_ROUND_COUNT: Cell<usize> = const { Cell::new(0) };
}

/// Reset this thread's benchmark count of successful nonempty convergence
/// checks. Generated Datalog calls check once per SCC publication batch.
pub fn reset_round_count() {
    NONEMPTY_ROUND_COUNT.with(|count| count.set(0));
    CHECKED_ROUND_COUNT.with(|count| count.set(0));
}

/// Nonempty SCC publication batches observed since reset on this thread.
/// Includes nonrecursive and recursive seeds; excludes terminal empty batches
/// and failed checks. Calling `RoundStatus::check` repeatedly counts repeatedly.
/// The diagnostic counter saturates rather than affecting query semantics.
pub fn round_count() -> usize {
    NONEMPTY_ROUND_COUNT.with(Cell::get)
}

/// All successful SCC checks, including terminal empty batches.
pub fn checked_round_count() -> usize {
    CHECKED_ROUND_COUNT.with(Cell::get)
}

pub struct Relation<'data, Row> {
    full: DeviceColumns<'data, Row>,
    delta: DeviceColumns<'data, Row>,
    rows: usize,
}
impl<Row> Clone for Relation<'_, Row> {
    fn clone(&self) -> Self {
        Self {
            full: self.full.clone(),
            delta: self.delta.clone(),
            rows: self.rows,
        }
    }
}
impl<'data, Row: RowCodec<'data>> Relation<'data, Row> {
    pub fn empty(
        pool: Arc<Pool>,
        dictionary: Arc<Dictionary<'data>>,
        width: usize,
    ) -> Result<Self> {
        if width != Row::WIDTH {
            return Err(failure(
                "CQ_GPU_SCHEMA",
                "empty relation width differs from row type",
            ));
        }
        with_runtime_for(Some(pool), |runtime| {
            let mut execution = runtime.execution();
            execution.dictionary.merge(&dictionary)?;
            let id = execution.empty_table(width)?;
            let full = execution.into_columns(id, &(0..width).collect::<Vec<_>>())?;
            Ok(Self {
                delta: full.clone(),
                full,
                rows: 0,
            })
        })
    }

    /// Canonicalize an EDB once. Compressed input codecs are decoded on device;
    /// dictionary IDs must be compatible with every subsequently bound table.
    pub fn from_columns<C>(input: &DeviceColumns<'data, Row, C>) -> Result<Self> {
        with_runtime_for(Some(input.pool().clone()), |runtime| {
            let mut execution = runtime.execution();
            let id = execution.bind_columns(input)?;
            if execution.table(id)?.columns.len() != Row::WIDTH {
                return Err(failure(
                    "CQ_GPU_SCHEMA",
                    "resident schema differs from row type",
                ));
            }
            let id = execution.dedup(id)?;
            let rows = execution.checked_rows(id)?;
            let id = if execution.table(id)?.capacity == rows {
                id
            } else {
                execution.union_with_capacity(&[id], rows)?
            };
            let full = execution.into_columns(id, &(0..Row::WIDTH).collect::<Vec<_>>())?;
            Ok(Self {
                delta: full.clone(),
                full,
                rows,
            })
        })
    }

    pub fn full(&self) -> &DeviceColumns<'data, Row> {
        &self.full
    }
    pub fn delta(&self) -> &DeviceColumns<'data, Row> {
        &self.delta
    }
    pub fn into_full(self) -> DeviceColumns<'data, Row> {
        self.full
    }

    /// Queue exact `delta' = distinct(union(candidates)) \\ full` and register
    /// its count/status in the shared round. No host wait occurs here.
    pub fn prepare<C>(
        &self,
        candidates: &[DeviceColumns<'data, Row, C>],
        round: &mut RoundStatus,
    ) -> Result<PreparedRelation<'data, Row>> {
        with_runtime_for(Some(self.full.pool().clone()), |runtime| {
            let mut execution = runtime.execution();
            let full = execution.bind_columns(&self.full)?;
            let candidates = candidates
                .iter()
                .map(|input| execution.bind_columns(input))
                .collect::<Result<Vec<_>>>()?;
            let candidates = if candidates.is_empty() {
                execution.empty_table(Row::WIDTH)?
            } else {
                execution.union_all(&candidates)?
            };
            let candidates = execution.dedup(candidates)?;
            let delta = execution.difference(candidates, full)?;
            let delta = execution.into_columns(delta, &(0..Row::WIDTH).collect::<Vec<_>>())?;
            let slot = round.tables.len();
            round.include(&delta)?;
            Ok(PreparedRelation {
                previous: self.clone(),
                delta,
                slot,
            })
        })
    }
}

/// A resident delta awaiting the round's shared checked cardinality readback.
pub struct PreparedRelation<'data, Row> {
    previous: Relation<'data, Row>,
    delta: DeviceColumns<'data, Row>,
    slot: usize,
}
impl<'data, Row: RowCodec<'data>> PreparedRelation<'data, Row> {
    /// Prepare the immutable full successor. Finish every member before
    /// replacing any source handle. Only the matching checked batch is accepted.
    pub fn finish(self, checked: &CheckedRound) -> Result<Relation<'data, Row>> {
        let _nvtx = nvtx::range(c"admission/full-copy");
        let Some((metadata, added)) = checked.members.get(self.slot) else {
            return Err(failure("CQ_GPU_ROUND", "missing checked member"));
        };
        if !Arc::ptr_eq(metadata, &self.delta.storage().metadata) {
            return Err(failure(
                "CQ_GPU_ROUND",
                "checked batch belongs to another member",
            ));
        }
        let rows = self
            .previous
            .rows
            .checked_add(*added)
            .ok_or_else(|| failure("CQ_GPU_OUTPUT_ROWS", "full cardinality overflow"))?;
        with_runtime_for(Some(self.delta.pool().clone()), |runtime| {
            let mut execution = runtime.execution();
            let full = execution.bind_columns(&self.previous.full)?;
            let delta = execution.bind_columns(&self.delta)?;
            // The batch checked every ancestor. Drop historical chains, but
            // retain current owners and any new full-copy status.
            for table in &mut execution.tables {
                table.statuses.clear();
            }
            // The shared barrier already checked the exact delta count. Keep
            // that capacity exact too: carrying candidate upper bounds into
            // the next round compounds slack through recursive joins/unions.
            let delta = if execution.table(delta)?.capacity == *added {
                delta
            } else {
                execution.union_with_capacity(&[delta], *added)?
            };
            let full = if *added == 0 {
                full
            } else {
                execution.union_with_capacity(&[full, delta], rows)?
            };
            let columns: Vec<_> = (0..Row::WIDTH).collect();
            let full: DeviceColumns<'data, Row> = execution.into_columns(full, &columns)?;
            let delta: DeviceColumns<'data, Row> = execution.into_columns(delta, &columns)?;
            let mut table = delta.storage().clone();
            table.statuses.push(full.storage().metadata.clone());
            let delta = unsafe {
                DeviceColumns::from_parts(
                    table,
                    Arc::new(delta.dictionary().clone()),
                    delta.pool().clone(),
                )
            };
            Ok(Relation { full, delta, rows })
        })
    }
}

/// Checked cardinalities bound to the precise immutable metadata owners.
pub struct CheckedRound {
    members: Vec<(Arc<Buffer<OutputCount>>, usize)>,
}
impl CheckedRound {
    pub fn any(&self) -> bool {
        self.members.iter().any(|(_, rows)| *rows != 0)
    }
}

/// Heterogeneous SCC convergence. All owners are retained until `check` completes;
/// failures in empty members and their ancestors are never treated as quiescence.
pub struct RoundStatus {
    pool: Arc<Pool>,
    tables: Vec<Table>,
}
impl RoundStatus {
    pub fn new(pool: Arc<Pool>) -> Self {
        Self {
            pool,
            tables: Vec::new(),
        }
    }
    pub fn include<Row, C>(&mut self, input: &DeviceColumns<'_, Row, C>) -> Result<()> {
        if !Arc::ptr_eq(&self.pool, input.pool()) {
            return Err(failure(
                "CQ_GPU_CONTEXT",
                "round status inputs must share a pool and stream",
            ));
        }
        self.tables.push(input.storage().clone());
        Ok(())
    }
    pub fn check(&self) -> Result<CheckedRound> {
        let _nvtx = nvtx::range(c"admission/count-check");
        with_runtime_for(Some(self.pool.clone()), |runtime| {
            let rows = crate::sets::checked_table_rows(runtime, &self.tables)?;
            let checked = CheckedRound {
                members: self
                    .tables
                    .iter()
                    .zip(rows)
                    .map(|(table, rows)| (table.metadata.clone(), rows))
                    .collect(),
            };
            CHECKED_ROUND_COUNT.with(|count| count.set(count.get().saturating_add(1)));
            if checked.any() {
                NONEMPTY_ROUND_COUNT.with(|count| count.set(count.get().saturating_add(1)));
            }
            Ok(checked)
        })
    }
}

impl Execution<'_, '_> {
    fn empty_table(&mut self, width: usize) -> Result<TableId> {
        let output = allocate_output(self.runtime, width, 0)?;
        let table = self.complete_output(output, vec![Bounds::Empty; width])?;
        Ok(self.add_table(table))
    }
}
