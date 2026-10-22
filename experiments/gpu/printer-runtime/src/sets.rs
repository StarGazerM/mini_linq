//! Exact resident row algebra and maintained all-column Datalog admission.
use crate::*;

/// Unique mutable state, moved from a relation into its pending successor.
/// Published tables hold immutable native ranges, never this writer capability.
pub(crate) struct MaintainedSet {
    keys: Buffer<u64>,
    slots: Buffer<u64>,
    width: usize,
    columns: Vec<Arc<Buffer<i64>>>,
    pointers: Option<Buffer<u64>>,
    capacity: usize,
}

impl MaintainedSet {
    pub(crate) fn new(runtime: &Runtime, width: usize, rows: usize) -> Result<Self> {
        let capacity = Self::set_capacity(rows)?;
        let words = capacity
            .checked_mul(width)
            .ok_or_else(|| failure("CQ_GPU_SET_CAPACITY", "persistent key size overflow"))?;
        Ok(Self {
            keys: unsafe { Buffer::uninitialized(runtime.pool.clone(), words)? },
            slots: Buffer::zeroed(runtime.pool.clone(), capacity)?,
            width,
            columns: Vec::new(),
            pointers: None,
            capacity: 0,
        })
    }

    fn set_capacity(rows: usize) -> Result<usize> {
        rows.max(1)
            .checked_mul(2)
            .and_then(usize::checked_next_power_of_two)
            .ok_or_else(|| failure("CQ_GPU_SET_CAPACITY", "persistent set capacity overflow"))
    }

    pub(crate) fn reserve_set(&mut self, runtime: &Runtime, rows: usize) -> Result<()> {
        let capacity = Self::set_capacity(rows)?;
        if capacity <= self.slots.len() {
            return Ok(());
        }
        let _nvtx = nvtx::range(c"admission/rehash");
        let words = capacity
            .checked_mul(self.width)
            .ok_or_else(|| failure("CQ_GPU_SET_CAPACITY", "persistent key size overflow"))?;
        let keys = unsafe { Buffer::uninitialized(runtime.pool.clone(), words)? };
        let slots = Buffer::zeroed(runtime.pool.clone(), capacity)?;
        let launch = runtime
            .kernels
            .prepare_persistent_rehash(config(self.slots.len(), 0))?;
        unsafe {
            runtime.kernels.persistent_rehash(
                runtime.stream(),
                &launch,
                self.keys.ptr(),
                self.slots.ptr(),
                self.slots.len(),
                keys.ptr(),
                slots.ptr(),
                capacity,
                self.width,
            )?;
        }
        self.keys = keys;
        self.slots = slots;
        Ok(())
    }

    pub(crate) fn insert(
        &self,
        runtime: &Runtime,
        input: &Table,
        output: OutputView,
        emit: bool,
    ) -> Result<()> {
        let launch = runtime
            .kernels
            .prepare_persistent_admit(config(input.capacity, 0))?;
        unsafe {
            runtime.kernels.persistent_admit(
                runtime.stream(),
                &launch,
                input.view(),
                self.width,
                self.keys.ptr(),
                self.slots.ptr(),
                self.slots.len(),
                output,
                emit,
            )?;
        }
        Ok(())
    }

    /// Reserve from the checked distinct count, not the candidate upper bound.
    /// Existing prefixes are copied only when doubling or detaching a branch.
    pub(crate) fn append(
        &mut self,
        runtime: &Runtime,
        previous: &Table,
        delta: &Table,
        rows: usize,
        added: usize,
    ) -> Result<()> {
        let _nvtx = nvtx::range(c"admission/append-tail");
        if self.pointers.is_none() || rows > self.capacity {
            let _growth = nvtx::range(c"admission/full-grow");
            let capacity = rows.max(1).checked_next_power_of_two().ok_or_else(|| {
                failure("CQ_GPU_OUTPUT_ROWS", "persistent full capacity overflow")
            })?;
            let output = allocate_output(runtime, self.width, capacity)?;
            let launch = runtime
                .kernels
                .prepare_persistent_copy(config(previous.capacity, 0))?;
            unsafe {
                runtime.kernels.persistent_copy(
                    runtime.stream(),
                    &launch,
                    previous.view(),
                    self.width,
                    output.view(),
                    0,
                )?;
            }
            self.columns = output.columns.into_iter().map(Arc::new).collect();
            self.pointers = Some(output.pointers);
            self.capacity = capacity;
        }
        if added != 0 {
            let launch = runtime.kernels.prepare_persistent_copy(config(added, 0))?;
            let output = OutputView {
                columns: self.pointers.as_ref().unwrap().ptr().cast(),
                capacity: self.capacity as u64,
                count: delta.metadata.ptr().cast(),
                error: unsafe { delta.metadata.ptr().cast::<u8>().add(8).cast() },
            };
            unsafe {
                runtime.kernels.persistent_copy(
                    runtime.stream(),
                    &launch,
                    delta.view(),
                    self.width,
                    output,
                    (rows - added) as u64,
                )?;
            }
        }
        Ok(())
    }

    pub(crate) fn snapshot(
        &self,
        runtime: &Runtime,
        offset: usize,
        rows: usize,
        metadata: Arc<Buffer<OutputCount>>,
        bounds: Vec<Bounds>,
    ) -> Result<Table> {
        let columns = self
            .columns
            .iter()
            .zip(&bounds)
            .map(|(column, &bound)| {
                // The writer only appends beyond all published prefixes. Range
                // metadata is version-local and is never advanced by later appends.
                let range = unsafe { Buffer::shared_slice(column.clone(), offset, rows)? };
                EncodedColumn::from_plain_buffer(range, metadata.clone(), 0, bound).map(Arc::new)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Table {
            views: runtime.column_views(&columns)?,
            columns,
            metadata: metadata.clone(),
            capacity: rows,
            bounds,
            statuses: vec![metadata],
        })
    }
}

impl<'runtime, 'data> Execution<'runtime, 'data> {
    /// Concatenate live rows without decoding them on the host. Inputs must have
    /// the same ordered schema; callers retain responsibility for Rust types.
    pub fn union_all(&mut self, inputs: &[TableId]) -> Result<TableId> {
        let _nvtx = nvtx::range(c"admission/union");
        let capacity = inputs.iter().try_fold(0usize, |sum, &id| {
            sum.checked_add(self.table(id)?.capacity)
                .ok_or_else(|| failure("CQ_GPU_OUTPUT_ROWS", "union capacity overflow"))
        })?;
        self.union_with_capacity(inputs, capacity)
    }

    pub(crate) fn union_with_capacity(
        &mut self,
        inputs: &[TableId],
        capacity: usize,
    ) -> Result<TableId> {
        let first = *inputs
            .first()
            .ok_or_else(|| failure("CQ_GPU_SCHEMA", "union requires a schema-bearing input"))?;
        let width = self.table(first)?.columns.len();
        let mut bounds = vec![Bounds::Empty; width];
        for &input in inputs {
            let table = self.table(input)?;
            if table.columns.len() != width {
                return Err(failure(
                    "CQ_GPU_SCHEMA",
                    "union schemas have different widths",
                ));
            }
            for (bound, &next) in bounds.iter_mut().zip(&table.bounds) {
                *bound = match (*bound, next) {
                    (Bounds::Empty, b) | (b, Bounds::Empty) => b,
                    (
                        Bounds::Inclusive { min: a, max: b },
                        Bounds::Inclusive { min: c, max: d },
                    ) => Bounds::Inclusive {
                        min: a.min(c),
                        max: b.max(d),
                    },
                };
            }
        }
        check_rows(capacity)?;
        let runtime = self.runtime;
        let start = TimingEvent::record(runtime.stream())?;
        let output = allocate_output(runtime, width, capacity)?;
        for &input in inputs {
            let table = self.table(input)?;
            let launch = runtime.kernels.prepare_append(config(table.capacity, 0))?;
            unsafe {
                runtime.kernels.append(
                    runtime.stream(),
                    &launch,
                    table.view(),
                    width,
                    output.view(),
                )?;
            }
        }
        let table = self.complete_output(output, bounds)?;
        self.record_timing(TimingStage::Build, start)?;
        Ok(self.add_table(table))
    }

    /// Exact all-column difference. This preserves candidate multiplicity;
    /// canonical admission separately deduplicates the union of all holes.
    pub fn difference(&mut self, candidates: TableId, full: TableId) -> Result<TableId> {
        let _nvtx = nvtx::range(c"admission/difference");
        let width = self.table(candidates)?.columns.len();
        if self.table(full)?.columns.len() != width {
            return Err(failure(
                "CQ_GPU_SCHEMA",
                "difference schemas have different widths",
            ));
        }
        let columns: Vec<_> = (0..width).collect();
        self.anti_filter(candidates, full, &columns, &columns)
    }

    /// Exact negative membership over arbitrary selected/repeated columns.
    /// With no key columns this is an existence anti join. Constant filters
    /// must already have been applied to `keys` by the checked rule compiler.
    pub fn anti_filter(
        &mut self,
        source: TableId,
        keys: TableId,
        source_columns: &[usize],
        key_columns: &[usize],
    ) -> Result<TableId> {
        if source_columns.len() != key_columns.len() {
            return Err(failure("CQ_GPU_SCHEMA", "anti key widths differ"));
        }
        let projected = self.project_table(source, source_columns)?;
        let membership = self.prepare_exact_membership(keys, key_columns)?;
        let membership = self.exact_membership_view(membership)?;
        let source = self.table(source)?;
        let projected = self.table(projected)?;
        let width = source.columns.len();
        let runtime = self.runtime;
        let start = TimingEvent::record(runtime.stream())?;
        let output = allocate_output(runtime, width, source.capacity)?;
        let exclude = runtime
            .kernels
            .prepare_set_exclude(config(source.capacity, 0))?;
        unsafe {
            runtime.kernels.set_exclude(
                runtime.stream(),
                &exclude,
                source.view(),
                projected.view(),
                membership.keys,
                width,
                membership.width,
                membership.packed,
                membership.width.max(1),
                membership.slots,
                membership.allocation,
                output.view(),
            )?;
        }
        let table = self.complete_output(output, source.bounds.clone())?;
        self.record_timing(TimingStage::Dedup, start)?;
        Ok(self.add_table(table))
    }

    /// Build an exact membership set owned by this execution. A
    /// FrozenDeviceMembership retains that execution across the stratum's holes.
    pub fn prepare_exact_membership(
        &mut self,
        table: TableId,
        columns: &[usize],
    ) -> Result<ExactMembershipId> {
        let _nvtx = nvtx::range(c"membership/exact-build");
        let keys = self.project_table(table, columns)?;
        let source = self.table(keys)?;
        let width = columns.len();
        let words = width.max(1);
        let packed_len = words
            .checked_mul(source.capacity)
            .ok_or_else(|| failure("CQ_GPU_PACKED_KEY_SIZE", "packed keys exceed address space"))?;
        let capacity = source
            .capacity
            .max(1)
            .checked_mul(2)
            .and_then(usize::checked_next_power_of_two)
            .ok_or_else(|| failure("CQ_GPU_SET_CAPACITY", "exact set capacity overflow"))?;
        let runtime = self.runtime;
        let start = TimingEvent::record(runtime.stream())?;
        let packed = unsafe { Buffer::uninitialized(runtime.pool.clone(), packed_len)? };
        let slots = unsafe { Buffer::uninitialized(runtime.pool.clone(), capacity)? };
        let output = allocate_output(runtime, 0, 0)?;
        let clear = runtime.kernels.prepare_clear_set(config(capacity, 0))?;
        let pack = runtime.kernels.prepare_pack(config(source.capacity, 0))?;
        let insert = runtime
            .kernels
            .prepare_set_insert(config(source.capacity, 0))?;
        unsafe {
            runtime.kernels.clear_set(
                runtime.stream(),
                &clear,
                source.view(),
                slots.ptr(),
                capacity,
            )?;
            runtime.kernels.pack(
                runtime.stream(),
                &pack,
                source.view(),
                width,
                packed.ptr(),
                words,
            )?;
            runtime.kernels.set_insert(
                runtime.stream(),
                &insert,
                source.view(),
                width,
                packed.ptr(),
                words,
                slots.ptr(),
                capacity,
                output.view(),
                false,
            )?;
        }
        // Register the build status so every later output retains failures.
        let status = self.complete_output(output, Vec::new())?;
        self.tables[table.0].statuses.push(status.metadata.clone());
        self.add_table(status);
        let id = ExactMembershipId(self.exact_memberships.len());
        self.exact_memberships.push(ExactMembership {
            packed,
            slots,
            keys,
            width,
        });
        self.record_timing(TimingStage::Build, start)?;
        Ok(id)
    }

    pub fn exact_membership_view(&self, id: ExactMembershipId) -> Result<ExactMembershipView> {
        let set = self
            .exact_memberships
            .get(id.0)
            .ok_or_else(|| failure("CQ_GPU_MEMBERSHIP_ID", "unknown exact membership handle"))?;
        Ok(ExactMembershipView {
            packed: set.packed.ptr(),
            slots: set.slots.ptr(),
            keys: self.table(set.keys)?.view(),
            allocation: set.slots.len(),
            width: set.width,
        })
    }

    /// One checked metadata readback, irrespective of retained ancestor count.
    pub(crate) fn checked_rows(&self, id: TableId) -> Result<usize> {
        checked_tables(self.runtime, &[self.table(id)?.clone()])
    }
}

/// Ancestors contribute failures but not cardinality. Owners remain alive until
/// the single compact result has been downloaded and the stream synchronized.
pub(crate) fn checked_tables(runtime: &Runtime, tables: &[Table]) -> Result<usize> {
    checked_table_rows(runtime, tables)?
        .into_iter()
        .try_fold(0usize, |total, rows| {
            total.checked_add(rows).ok_or_else(|| {
                failure(
                    "CQ_GPU_OUTPUT_ROWS",
                    "combined row count exceeds address space",
                )
            })
        })
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct StatusInput {
    pub(crate) status: *const OutputCount,
    pub(crate) capacity: u64,
}
unsafe impl DeviceCopy for StatusInput {}

/// One error slot and one count/status slot per member, downloaded together.
pub(crate) fn checked_table_rows(runtime: &Runtime, tables: &[Table]) -> Result<Vec<usize>> {
    let summary =
        unsafe { Buffer::<OutputCount>::uninitialized(runtime.pool.clone(), tables.len() + 1)? };
    let mut inputs = Vec::with_capacity(tables.len());
    let mut seen = std::collections::HashSet::new();
    for table in tables {
        inputs.push(StatusInput {
            status: table.metadata.ptr(),
            capacity: table.capacity as u64,
        });
        seen.insert(Arc::as_ptr(&table.metadata));
    }
    for table in tables {
        for status in &table.statuses {
            if seen.insert(Arc::as_ptr(status)) {
                inputs.push(StatusInput {
                    status: status.ptr(),
                    capacity: u64::MAX,
                });
            }
        }
    }
    let inputs = runtime.upload_descriptors(inputs.into_iter())?;
    let launch = runtime.kernels.prepare_gather_status(config(1, 0))?;
    unsafe {
        runtime.kernels.gather_status(
            runtime.stream(),
            &launch,
            inputs.ptr(),
            tables.len(),
            inputs.len(),
            summary.ptr(),
        )?;
    }
    record_host_wait(true);
    let statuses = summary.to_host()?;
    runtime.reclaim_descriptors();
    runtime.collect_timings()?;
    statuses
        .into_iter()
        .enumerate()
        .filter_map(|(index, status)| {
            let checked = checked_status(status);
            (index != 0 || checked.is_err()).then_some(checked)
        })
        .collect()
}

fn checked_status(status: OutputCount) -> Result<usize> {
    if status.error != 0 {
        let code = match status.error {
            1 => "CQ_GPU_OUTPUT_OVERFLOW",
            2 => "CQ_GPU_EXPANSION_OVERFLOW",
            3 => "CQ_GPU_SET_FULL",
            4 => "CQ_GPU_OUTPUT_CAPACITY",
            5 => "CQ_GPU_OUTPUT_ROWS",
            6 => "CQ_GPU_MEMBERSHIP_DOMAIN",
            7 => "CQ_GPU_RUN_BUILD",
            _ => "CQ_GPU_KERNEL_ERROR",
        };
        return Err(failure(
            code,
            "checked resident publication/convergence failed",
        ));
    }
    usize::try_from(status.rows)
        .map_err(|_| failure("CQ_GPU_OUTPUT_ROWS", "row count exceeds address space"))
}
