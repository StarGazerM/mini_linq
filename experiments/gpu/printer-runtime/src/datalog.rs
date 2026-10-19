//! Resident, exact Boolean Datalog publication.
//!
//! The baseline rebuilds sets and full snapshots at each publication. It does
//! not retain a spine or share indexes across separately compiled rule holes.
//! All rule holes in an SCC must read the old `full`/`delta` handles; prepare
//! every member's successor before replacing any member. Even empty deltas are
//! published. Tuple data never crosses the device boundary here. Publication
//! reads one checked 16-byte delta summary to bound full allocation by live
//! cardinality; `RoundStatus` reads one checked 16-byte SCC convergence summary.
use crate::*;

thread_local! {
    static NONEMPTY_ROUND_COUNT: Cell<usize> = const { Cell::new(0) };
}

/// Reset this thread's benchmark count of successful nonempty convergence
/// checks. Generated Datalog calls check once per SCC publication batch.
pub fn reset_round_count() {
    NONEMPTY_ROUND_COUNT.with(|count| count.set(0));
}

/// Nonempty SCC publication batches observed since reset on this thread.
/// Includes nonrecursive and recursive seeds; excludes terminal empty batches
/// and failed checks. Calling `RoundStatus::any` repeatedly counts repeatedly.
/// The diagnostic counter saturates rather than affecting query semantics.
pub fn round_count() -> usize {
    NONEMPTY_ROUND_COUNT.with(Cell::get)
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

    /// Compute a successor without mutating any published source snapshot:
    /// `delta' = distinct(union(candidates)) \\ full`, `full' = full union delta'`.
    /// Candidate projection duplicates and duplicates across holes are removed
    /// together before admission. Empty candidates replace, not retain, delta.
    pub fn prepare<C>(&self, candidates: &[DeviceColumns<'data, Row, C>]) -> Result<Self> {
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
            let added = execution.checked_rows(delta)?;
            let rows = self
                .rows
                .checked_add(added)
                .ok_or_else(|| failure("CQ_GPU_OUTPUT_ROWS", "full cardinality overflow"))?;
            // Every bound table and ancestor is included in delta's checked
            // status closure. Prune historical chains only after that succeeds.
            // Current metadata owners remain retained by all new outputs.
            for table in &mut execution.tables {
                table.statuses.clear();
            }
            let full = if added == 0 {
                full
            } else {
                execution.union_with_capacity(&[full, delta], rows)?
            };
            let columns: Vec<_> = (0..Row::WIDTH).collect();
            let full: DeviceColumns<'data, Row> = execution.into_columns(full, &columns)?;
            let mut delta: DeviceColumns<'data, Row> = execution.into_columns(delta, &columns)?;
            // The convergence summary must also observe the final full-copy
            // status, including when callers include only member deltas.
            let mut table = delta.storage().clone();
            table.statuses.push(full.storage().metadata.clone());
            delta = unsafe {
                DeviceColumns::from_parts(
                    table,
                    Arc::new(delta.dictionary().clone()),
                    delta.pool().clone(),
                )
            };
            Ok(Self { full, delta, rows })
        })
    }
}

/// Heterogeneous SCC convergence. All owners are retained until `any` completes;
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
    pub fn any(&self) -> Result<bool> {
        with_runtime_for(Some(self.pool.clone()), |runtime| {
            let any = crate::sets::checked_tables(runtime, &self.tables)? != 0;
            if any {
                NONEMPTY_ROUND_COUNT.with(|count| count.set(count.get().saturating_add(1)));
            }
            Ok(any)
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
