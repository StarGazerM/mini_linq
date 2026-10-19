//! Resident, exact Boolean Datalog publication.
//!
//! Each IDB retains an exact all-column device set and geometrically reserved
//! native full columns. Published full/delta handles are immutable ranges with
//! version-local counts; later appends touch only the unpublished tail.
//! All holes read old snapshots, never the mutable set, so set insertion may
//! happen during preparation. Prepare and finish every SCC member before
//! replacing any member, including empty deltas. One checked batch downloads
//! all delta counts and sticky ancestor errors. Join indexes are not maintained.
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
    maintained: RefCell<Option<crate::sets::MaintainedSet>>,
}
impl<Row> Clone for Relation<'_, Row> {
    fn clone(&self) -> Self {
        Self {
            full: self.full.clone(),
            delta: self.delta.clone(),
            rows: self.rows,
            // A branch is a read-only snapshot until preparation detaches its
            // own columns and set. Never duplicate the append writer.
            maintained: RefCell::new(None),
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
                maintained: RefCell::new(None),
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
                maintained: RefCell::new(None),
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

    /// Insert every hole's candidates into the maintained exact set, emitting
    /// newly inserted tuples once. No host wait or full scan occurs on the
    /// normal path. The writer moves into the pending successor; preparing this
    /// old handle again creates a detached branch, never a second tail writer.
    pub fn prepare<C>(
        &self,
        candidates: &[DeviceColumns<'data, Row, C>],
        round: &mut RoundStatus,
    ) -> Result<PreparedRelation<'data, Row>> {
        with_runtime_for(Some(self.full.pool().clone()), |runtime| {
            let mut execution = runtime.execution();
            let full = execution.bind_columns(&self.full)?;
            let candidates = candidates.iter()
                .map(|input| execution.bind_columns(input))
                .collect::<Result<Vec<_>>>()?;
            let capacity = candidates.iter().try_fold(0usize, |sum, &id| {
                sum.checked_add(execution.table(id)?.capacity)
                    .ok_or_else(|| failure("CQ_GPU_OUTPUT_ROWS", "candidate capacity overflow"))
            })?;
            check_rows(capacity)?;
            let upper = self.rows.checked_add(capacity)
                .ok_or_else(|| failure("CQ_GPU_OUTPUT_ROWS", "admission capacity overflow"))?;
            let _nvtx = nvtx::range(c"admission/insert-new");
            let mut maintained = match self.maintained.borrow_mut().take() {
                Some(state) => state,
                None => {
                    let state = crate::sets::MaintainedSet::new(runtime, Row::WIDTH, upper)?;
                    let status = allocate_output(runtime, 0, 0)?;
                    state.insert(runtime, execution.table(full)?, status.view(), false)?;
                    let status = execution.complete_output(status, Vec::new())?;
                    execution.add_table(status);
                    state
                }
            };
            maintained.reserve_set(runtime, upper)?;
            let output = allocate_output(runtime, Row::WIDTH, capacity)?;
            let mut bounds = vec![Bounds::Empty; Row::WIDTH];
            for &candidate in &candidates {
                let table = execution.table(candidate)?;
                for (bound, &next) in bounds.iter_mut().zip(&table.bounds) {
                    *bound = merge_bound(*bound, next);
                }
                maintained.insert(runtime, table, output.view(), true)?;
            }
            let delta = execution.complete_output(output, bounds)?;
            let delta = execution.add_table(delta);
            let delta = execution.into_columns(delta, &(0..Row::WIDTH).collect::<Vec<_>>())?;
            let slot = round.tables.len();
            round.include(&delta)?;
            Ok(PreparedRelation {
                previous: self.clone(),
                delta,
                slot,
                maintained,
            })
        })
    }
}

/// A resident delta awaiting the round's shared checked cardinality readback.
pub struct PreparedRelation<'data, Row> {
    previous: Relation<'data, Row>,
    delta: DeviceColumns<'data, Row>,
    slot: usize,
    maintained: crate::sets::MaintainedSet,
}
impl<'data, Row: RowCodec<'data>> PreparedRelation<'data, Row> {
    /// Prepare the immutable full successor. Finish every member before
    /// replacing any source handle. Only the matching checked batch is accepted.
    pub fn finish(mut self, checked: &CheckedRound) -> Result<Relation<'data, Row>> {
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
        check_rows(rows)?;
        with_runtime_for(Some(self.delta.pool().clone()), |runtime| {
            self.maintained.append(
                runtime, self.previous.full.storage(), self.delta.storage(), rows, *added,
            )?;
            let bounds = self.previous.full.storage().bounds.iter()
                .zip(&self.delta.storage().bounds)
                .map(|(&old, &new)| merge_bound(old, new))
                .collect();
            let full = self.maintained.snapshot(runtime, 0, rows, runtime.metadata(rows)?, bounds)?;
            let delta = self.maintained.snapshot(
                runtime, self.previous.rows, *added, self.delta.storage().metadata.clone(),
                self.delta.storage().bounds.clone(),
            )?;
            let dictionary = Arc::new(self.delta.dictionary().clone());
            // The shared check validated every ancestor before any successor
            // is published. Deterministic tail copies cannot overflow: rows was
            // checked and storage reserved above. New metadata remains checked
            // by subsequent rounds, including terminal empty publication.
            let full = unsafe {
                DeviceColumns::from_parts(full, dictionary.clone(), self.delta.pool().clone())
            };
            let delta = unsafe {
                DeviceColumns::from_parts(delta, dictionary, self.delta.pool().clone())
            };
            Ok(Relation {
                full, delta, rows,
                maintained: RefCell::new(Some(self.maintained)),
            })
        })
    }
}

fn merge_bound(left: Bounds, right: Bounds) -> Bounds {
    match (left, right) {
        (Bounds::Empty, bound) | (bound, Bounds::Empty) => bound,
        (Bounds::Inclusive { min: a, max: b }, Bounds::Inclusive { min: c, max: d }) =>
            Bounds::Inclusive { min: a.min(c), max: b.max(d) },
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
