//! Completed immutable indexes of stable full-width u32 key/payload bags.
//!
//! Construction is synchronous; reads borrow already-ready MLX storage without
//! evaluation, FFI, allocation, or retaining another owner. MLX alone manages
//! allocation reuse and submitted-work lifetimes. CPU construction is explicit,
//! never a fallback. Indexes retain their originating context and are thread-affine.

use std::{fmt, iter::FusedIterator, mem::MaybeUninit, ptr::NonNull, rc::Rc, slice};

use crate::{
    clear_mlx_error, ffi, mlx_failure, mlx_status, native_status,
    relation::{Dtype, IndexSpec, Payload, Relation},
    Error, Gpu, Result, StreamInner,
};

mod cpu_radix;

/// Bound for padded GPU grids and u32 offsets; device allocation limits also apply.
pub const MAX_ROWS: usize = 1 << 30;
const DIRECTORY_ENTRIES: usize = 4097;

/// One packed key/payload output row.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub key: u32,
    pub value: u32,
}

// A pair array is deliberately not a U32Array: its native shape is [N, 2].
// These private owners never expose a handle or participate in buffer donation.
struct IndexArray(ffi::Array);

impl IndexArray {
    fn output() -> Self {
        Self(unsafe { ffi::mlx_array_new() })
    }
}

impl Drop for IndexArray {
    fn drop(&mut self) {
        unsafe { ffi::mlx_array_free(self.0) };
    }
}

struct Completed {
    // Free arrays before releasing the stream retained for their entire lifetime.
    _pairs: IndexArray,
    _directory: IndexArray,
    stream: Rc<StreamInner>,
    rows: NonNull<Row>,
    directory: NonNull<u32>,
    len: usize,
}

/// Cheaply cloned ownership of completed, immutable MLX row and directory arrays.
#[derive(Clone)]
pub struct Index {
    inner: Rc<Completed>,
}

impl fmt::Debug for Index {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Index")
            .field("len", &self.len())
            .finish_non_exhaustive()
    }
}

impl Index {
    pub fn len(&self) -> usize {
        self.inner.len
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Borrows contiguous ready rows, including all duplicate occurrences.
    pub fn rows(&self) -> &[Row] {
        // Both publication paths initialize every row before constructing this
        // owner. The retained arrays cannot be donated, mutated, or recycled.
        unsafe { slice::from_raw_parts(self.inner.rows.as_ptr(), self.len()) }
    }

    /// Exact lower bounds for every 12-bit prefix plus the terminal row count.
    pub fn directory(&self) -> &[u32] {
        unsafe { slice::from_raw_parts(self.inner.directory.as_ptr(), DIRECTORY_ENTRIES) }
    }

    /// Context identity, not merely physical device identity. Cloned Gpu values
    /// share identity; independently created contexts do not, even on one device.
    pub fn belongs_to(&self, gpu: &Gpu) -> bool {
        Rc::ptr_eq(&self.inner.stream, &gpu.stream)
    }

    fn bucket(&self, key: u32) -> &[Row] {
        let prefix = (key >> 20) as usize;
        let directory = self.directory();
        &self.rows()[directory[prefix] as usize..directory[prefix + 1] as usize]
    }

    /// Exact, allocation-free bounds within the radix-12 bucket.
    pub fn lookup(&self, key: &u32) -> Cursor<'_> {
        let bucket = self.bucket(*key);
        let begin = bucket.partition_point(|row| row.key < *key);
        let end = begin + bucket[begin..].partition_point(|row| row.key == *key);
        Cursor {
            rows: &bucket[begin..end],
        }
    }

    pub fn contains(&self, key: &u32) -> bool {
        let bucket = self.bucket(*key);
        let begin = bucket.partition_point(|row| row.key < *key);
        bucket.get(begin).is_some_and(|row| row.key == *key)
    }
}

/// Borrowed, copyable exact-size payload cursor preserving occurrence chronology.
#[derive(Clone, Copy, Debug)]
pub struct Cursor<'a> {
    rows: &'a [Row],
}

impl Iterator for Cursor<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<Self::Item> {
        let (row, rest) = self.rows.split_first()?;
        self.rows = rest;
        Some(row.value)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.rows.len(), Some(self.rows.len()))
    }
}

impl ExactSizeIterator for Cursor<'_> {}
impl FusedIterator for Cursor<'_> {}

impl Gpu {
    // Fresh, concrete MLX storage is exclusively owned here. Callers must finish
    // initialization before passing the handle to any lazy operation or owner.
    fn allocate_index_array(&self, shape: &[i32]) -> Result<(IndexArray, *mut u32)> {
        let mut output = IndexArray::output();
        let mut data = std::ptr::null_mut();
        native_status(
            unsafe {
                ffi::mccl_allocate_u32(
                    &mut output.0,
                    shape.as_ptr(),
                    shape.len() as i32,
                    &mut data,
                    self.stream.0,
                )
            },
            "allocate index storage",
        )?;
        if data.is_null() && !shape.contains(&0) {
            return Err(Error(
                "allocate index storage: null nonempty mapping".into(),
            ));
        }
        Ok((output, data))
    }

    /// Stable GPU projection, optional compaction, radix sort and directory build.
    /// Source values are never scanned or packed on the host. Selected signed
    /// fields outside the u32 domain are rejected by GPU-produced error metadata.
    pub fn build_index(
        &self,
        relation: &Relation,
        spec: IndexSpec,
        selection: Option<&Relation>,
    ) -> Result<Index> {
        let payload_column = self.check_index_inputs(relation, spec, selection)?;
        let mut pairs = IndexArray::output();
        let mut directory = IndexArray::output();
        let mut len = 0;
        native_status(
            unsafe {
                ffi::mccl_build_relation_index_u32(
                    &mut pairs.0,
                    &mut directory.0,
                    &mut len,
                    relation.raw(),
                    spec.key_column as i32,
                    payload_column,
                    selection.map_or(
                        ffi::Array {
                            ctx: std::ptr::null_mut(),
                        },
                        Relation::raw,
                    ),
                    self.stream.0,
                )
            },
            "build shared relation index",
        )?;
        // Native construction is synchronous and exposes only initialized rows.
        self.publish_ready_gpu_index(pairs, directory, len)
    }

    /// Explicit stable CPU comparison baseline. Reads source storage on CPU and
    /// initializes its own MLX output; sorting may allocate temporary host scratch.
    pub fn build_index_on_cpu(
        &self,
        relation: &Relation,
        spec: IndexSpec,
        selection: Option<&Relation>,
    ) -> Result<Index> {
        self.build_cpu_index::<false>(relation, spec, selection)
    }

    /// Explicit single-threaded stable CPU byte radix baseline. Reads source
    /// values on CPU; output, directory and unpublished scratch are MLX-owned.
    pub fn build_index_on_cpu_radix(
        &self,
        relation: &Relation,
        spec: IndexSpec,
        selection: Option<&Relation>,
    ) -> Result<Index> {
        self.build_cpu_index::<true>(relation, spec, selection)
    }

    fn build_cpu_index<const RADIX: bool>(
        &self,
        relation: &Relation,
        spec: IndexSpec,
        selection: Option<&Relation>,
    ) -> Result<Index> {
        self.check_index_inputs(relation, spec, selection)?;
        let mask = selection.map(Relation::u32_values).transpose()?;
        let len = mask.map_or(relation.len(), |mask| {
            mask.iter().filter(|&&v| v != 0).count()
        });
        enum Values<'a> {
            U32(&'a [u32]),
            I64(&'a [i64]),
        }
        let values = match relation.dtype() {
            Dtype::U32 => Values::U32(relation.u32_values()?),
            Dtype::I64 => Values::I64(relation.i64_values()?),
        };
        let field = |row: usize, column: usize| -> Result<u32> {
            let offset = row * relation.columns() + column;
            match &values {
                Values::U32(values) => Ok(values[offset]),
                Values::I64(values) => u32::try_from(values[offset]).map_err(|_| {
                    Error(format!(
                        "selected source row {row}, column {column} is outside u32"
                    ))
                }),
            }
        };
        let (pairs, data) = self.allocate_index_array(&[len as i32, 2])?;
        let target = NonNull::new(data.cast::<Row>()).unwrap_or_else(NonNull::dangling);
        let mut count = 0;
        let mut first_key = 0;
        let mut varying = 0;
        for row in 0..relation.len() {
            if mask.is_some_and(|mask| mask[row] == 0) {
                continue;
            }
            let key = field(row, spec.key_column)?;
            let value = match spec.payload {
                Payload::RowId => row as u32,
                Payload::Column(column) => field(row, column)?,
            };
            if count == 0 {
                first_key = key;
            }
            if RADIX {
                varying |= key ^ first_key;
            }
            unsafe { target.as_ptr().add(count).write(Row { key, value }) };
            count += 1;
        }
        // Only the CPU baseline initializes output here. Failure above drops the
        // unpublished partial allocation without exposing a reference to its tail.
        let sorted = unsafe { slice::from_raw_parts_mut(target.as_ptr(), len) };
        if RADIX {
            if varying != 0 {
                let (_scratch, data) = self.allocate_index_array(&[len as i32, 2])?;
                let scratch =
                    unsafe { slice::from_raw_parts_mut(data.cast::<MaybeUninit<Row>>(), len) };
                cpu_radix::sort(sorted, scratch, varying);
            }
        } else {
            sorted.sort_by_key(|row| row.key);
        }
        let (directory, offsets) = self.allocate_index_array(&[DIRECTORY_ENTRIES as i32])?;
        let mut offset = 0usize;
        for prefix in 0..DIRECTORY_ENTRIES {
            while offset < sorted.len() && ((sorted[offset].key >> 20) as usize) < prefix {
                offset += 1;
            }
            unsafe { offsets.add(prefix).write(offset as u32) };
        }
        Ok(self.publish_index(
            pairs,
            directory,
            target,
            NonNull::new(offsets).unwrap(),
            len,
        ))
    }

    /// Stable GPU merge with older equal-key rows before newer ones. Both indexes
    /// must originate from this Gpu (or its clones), not merely the same device.
    /// A context mismatch is rejected before constructing/submitting a graph.
    pub fn merge_indexes(&self, older: &Index, newer: &Index) -> Result<Index> {
        if !older.belongs_to(self) || !newer.belongs_to(self) {
            return Err(Error("index belongs to a different GPU context".into()));
        }
        let len = older
            .len()
            .checked_add(newer.len())
            .filter(|&n| n <= MAX_ROWS)
            .ok_or_else(|| Error("combined index row count exceeds capacity".into()))?;
        let mut pairs = IndexArray::output();
        native_status(
            unsafe {
                ffi::mccl_merge_sorted_pairs_u32(
                    &mut pairs.0,
                    older.inner._pairs.0,
                    newer.inner._pairs.0,
                    self.stream.0,
                )
            },
            "merge index pairs",
        )?;
        self.complete_gpu_index(pairs, len)
    }

    fn complete_gpu_index(&self, pairs: IndexArray, len: usize) -> Result<Index> {
        let mut directory = IndexArray::output();
        native_status(
            unsafe { ffi::mccl_radix_directory_u32(&mut directory.0, pairs.0, self.stream.0) },
            "build index directory",
        )?;
        clear_mlx_error();
        // Directory depends on sorted pairs: this waits for the complete graph,
        // not just encoding/submission. Do not publish before successful return.
        mlx_status(
            unsafe { ffi::mlx_array_eval(directory.0) },
            "evaluate index",
        )?;
        self.publish_ready_gpu_index(pairs, directory, len)
    }

    fn publish_ready_gpu_index(
        &self,
        pairs: IndexArray,
        directory: IndexArray,
        len: usize,
    ) -> Result<Index> {
        let rows = if len == 0 {
            NonNull::dangling()
        } else {
            clear_mlx_error();
            NonNull::new(
                unsafe { ffi::mlx_array_data_uint32(pairs.0) }
                    .cast_mut()
                    .cast::<Row>(),
            )
            .ok_or_else(|| mlx_failure("read index rows"))?
        };
        clear_mlx_error();
        let offsets = NonNull::new(unsafe { ffi::mlx_array_data_uint32(directory.0) }.cast_mut())
            .ok_or_else(|| mlx_failure("read index directory"))?;
        Ok(self.publish_index(pairs, directory, rows, offsets, len))
    }

    fn publish_index(
        &self,
        pairs: IndexArray,
        directory: IndexArray,
        rows: NonNull<Row>,
        offsets: NonNull<u32>,
        len: usize,
    ) -> Index {
        Index {
            inner: Rc::new(Completed {
                _pairs: pairs,
                _directory: directory,
                stream: Rc::clone(&self.stream),
                rows,
                directory: offsets,
                len,
            }),
        }
    }
}
