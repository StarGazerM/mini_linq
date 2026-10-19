//! Exact resident row algebra. Sets are rebuilt per operation/publication.
use crate::*;

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
        let start = runtime.stream().record_event(timing_flags())?;
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
        let start = runtime.stream().record_event(timing_flags())?;
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

    /// Build a frozen exact membership set usable inside fused rule kernels.
    /// The index is execution-local, not a persistent/publication-spanning cache.
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
        let start = runtime.stream().record_event(timing_flags())?;
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

/// One error slot and one count/status slot per member, downloaded together.
pub(crate) fn checked_table_rows(runtime: &Runtime, tables: &[Table]) -> Result<Vec<usize>> {
    let summary = Buffer::<OutputCount>::zeroed(runtime.pool.clone(), tables.len() + 1)?;
    let launch = runtime.kernels.prepare_fold_status(config(1, 0))?;
    let mut seen = std::collections::HashSet::new();
    for (index, table) in tables.iter().enumerate() {
        for status in &table.statuses {
            if seen.insert(Arc::as_ptr(status)) {
                unsafe {
                    runtime.kernels.fold_status(
                        runtime.stream(),
                        &launch,
                        status.ptr(),
                        u64::MAX,
                        false,
                        summary.ptr(),
                    )?;
                }
            }
        }
        unsafe {
            runtime.kernels.fold_status(
                runtime.stream(),
                &launch,
                table.metadata.ptr(),
                table.capacity as u64,
                true,
                summary.ptr().add(index + 1),
            )?;
        }
    }
    record_host_wait(true);
    let statuses = summary.to_host()?;
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
