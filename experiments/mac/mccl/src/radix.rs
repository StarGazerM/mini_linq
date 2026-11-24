//! GPU-built grouped u32 runs with completed native-compatible spline storage.
//!
//! Builds and merges are synchronous. Slice getters only borrow immutable MLX
//! storage: they perform no FFI, evaluation, copying, or reference-count updates.
//! This module intentionally has no CPU spline reader; consumers use the native
//! RadixRun reader with these arrays and scalar metadata. Clones retain all five
//! arrays and their originating context, including after sources and Gpu drop.

use std::{
    ffi::c_int,
    fmt,
    mem::{align_of, offset_of, size_of},
    ptr::NonNull,
    rc::Rc,
    slice,
};

use crate::{
    Error, Gpu, Result, StreamInner, U32Array, clear_mlx_error, ffi,
    index::MAX_ROWS,
    mlx_failure, native_status,
    relation::{IndexSpec, Relation},
};

const RADIX_ENTRIES: usize = 4098;

// The native ABI publishes uint64 offsets/ranks. Its zero-copy usize view is
// supported only on the 64-bit target used by MLX Metal.
const _: () = {
    assert!(size_of::<usize>() == size_of::<u64>());
    assert!(align_of::<usize>() == align_of::<u64>());
    assert!(size_of::<ffi::Array>() == 8);
    assert!(align_of::<ffi::RadixIndex>() == 8);
    assert!(size_of::<ffi::RadixIndex>() == 88);
    assert!(offset_of!(ffi::RadixIndex, offsets) == 8);
    assert!(offset_of!(ffi::RadixIndex, items) == 16);
    assert!(offset_of!(ffi::RadixIndex, points) == 24);
    assert!(offset_of!(ffi::RadixIndex, radix) == 32);
    assert!(offset_of!(ffi::RadixIndex, rows) == 40);
    assert!(offset_of!(ffi::RadixIndex, key_count) == 48);
    assert!(offset_of!(ffi::RadixIndex, point_count) == 56);
    assert!(offset_of!(ffi::RadixIndex, min_key) == 64);
    assert!(offset_of!(ffi::RadixIndex, max_key) == 72);
    assert!(offset_of!(ffi::RadixIndex, shift) == 80);
    assert!(offset_of!(ffi::RadixIndex, search_error) == 84);
};

struct Arrays(ffi::RadixIndex);

impl Arrays {
    fn output() -> Self {
        // mlx_array_new initializes an empty output handle with a null ctx.
        Self(ffi::RadixIndex {
            keys: unsafe { ffi::mlx_array_new() },
            offsets: unsafe { ffi::mlx_array_new() },
            items: unsafe { ffi::mlx_array_new() },
            points: unsafe { ffi::mlx_array_new() },
            radix: unsafe { ffi::mlx_array_new() },
            rows: 0,
            key_count: 0,
            point_count: 0,
            min_key: 0,
            max_key: 0,
            shift: 0,
            search_error: 32,
        })
    }

    fn handles(&self) -> [ffi::Array; 5] {
        let raw = &self.0;
        [raw.keys, raw.offsets, raw.items, raw.points, raw.radix]
    }
}

impl Drop for Arrays {
    fn drop(&mut self) {
        for array in self.handles() {
            if !array.ctx.is_null() {
                unsafe { ffi::mlx_array_free(array) };
            }
        }
    }
}

struct Completed {
    // Free the arrays before releasing the context they retain.
    arrays: Arrays,
    stream: Rc<StreamInner>,
    keys: NonNull<u32>,
    offsets: NonNull<usize>,
    items: NonNull<u32>,
    points: NonNull<[u64; 2]>,
    radix: NonNull<usize>,
}

/// Cheaply cloned, thread-affine ownership of an immutable grouped GPU run.
///
/// Only initialized prefixes are exposed. Keys are strictly increasing, offsets
/// have one terminal entry, and items preserve source chronology within a key.
/// Points are `[key, distinct_key_rank]`; radix entries index these points.
#[derive(Clone)]
pub struct RadixIndex {
    inner: Rc<Completed>,
}

impl fmt::Debug for RadixIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RadixIndex")
            .field("rows", &self.len())
            .field("keys", &self.inner.arrays.0.key_count)
            .field("points", &self.inner.arrays.0.point_count)
            .finish_non_exhaustive()
    }
}

impl RadixIndex {
    /// Number of payload occurrences, not the number of distinct keys.
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.arrays.0.rows
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn keys(&self) -> &[u32] {
        unsafe { slice::from_raw_parts(self.inner.keys.as_ptr(), self.inner.arrays.0.key_count) }
    }

    #[inline]
    pub fn offsets(&self) -> &[usize] {
        unsafe {
            slice::from_raw_parts(
                self.inner.offsets.as_ptr(),
                self.inner.arrays.0.key_count + 1,
            )
        }
    }

    #[inline]
    pub fn items(&self) -> &[u32] {
        unsafe { slice::from_raw_parts(self.inner.items.as_ptr(), self.len()) }
    }

    #[inline]
    pub fn points(&self) -> &[[u64; 2]] {
        unsafe {
            slice::from_raw_parts(self.inner.points.as_ptr(), self.inner.arrays.0.point_count)
        }
    }

    #[inline]
    pub fn radix(&self) -> &[usize] {
        unsafe { slice::from_raw_parts(self.inner.radix.as_ptr(), RADIX_ENTRIES) }
    }

    #[inline]
    pub fn min_key(&self) -> u64 {
        self.inner.arrays.0.min_key
    }

    #[inline]
    pub fn max_key(&self) -> u64 {
        self.inner.arrays.0.max_key
    }

    #[inline]
    pub fn shift(&self) -> u32 {
        self.inner.arrays.0.shift
    }

    #[inline]
    pub fn search_error(&self) -> usize {
        self.inner.arrays.0.search_error as usize
    }

    /// Checks context identity, not merely physical device identity.
    #[inline]
    pub fn belongs_to(&self, gpu: &Gpu) -> bool {
        Rc::ptr_eq(&self.inner.stream, &gpu.stream)
    }
}

struct PairsArray(ffi::Array);

impl Drop for PairsArray {
    fn drop(&mut self) {
        if !self.0.ctx.is_null() {
            unsafe { ffi::mlx_array_free(self.0) };
        }
    }
}

/// Completed pairs of a batched GPU join against one grouped run.
///
/// Each pair is `[probe ordinal, item]`, ordered by probe ordinal and then by
/// the run's stable occurrence order. The pairs own their storage; they do not
/// pin the probed run. Thread-affine like the context they retain.
pub struct RadixJoin {
    // Free the array before releasing the context it retains.
    _pairs: PairsArray,
    _stream: Rc<StreamInner>,
    data: NonNull<[u32; 2]>,
    len: usize,
}

impl fmt::Debug for RadixJoin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RadixJoin")
            .field("pairs", &self.len)
            .finish_non_exhaustive()
    }
}

impl RadixJoin {
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub fn pairs(&self) -> &[[u32; 2]] {
        unsafe { slice::from_raw_parts(self.data.as_ptr(), self.len) }
    }
}

/// Row IDs entering a GPU semijoin: a join's pairs (their row column) or a
/// previously filtered row set. Both stay in GPU-visible storage.
#[derive(Clone, Copy)]
pub enum SemiJoinRows<'a> {
    Join(&'a RadixJoin),
    Rows(&'a U32Array),
}

/// One semijoin of [`Gpu::join_semijoin_distinct`]: keep rows whose
/// `key_column` occurs in `index` (and, with `item_column`, whose value is in
/// that key's group).
#[derive(Clone, Copy)]
pub struct SemiJoinStep<'a> {
    pub index: &'a RadixIndex,
    pub key_column: usize,
    pub item_column: Option<usize>,
}

fn distinct_columns(relation: &Relation, columns: &[usize]) -> Result<Vec<c_int>> {
    columns
        .iter()
        .map(|&column| {
            c_int::try_from(column)
                .ok()
                .filter(|_| column < relation.columns())
                .ok_or_else(|| Error("distinct rows column is outside the relation".into()))
        })
        .collect()
}

/// Completed distinct int64 tuples from [`Gpu::distinct_rows`], row-major and
/// in ascending lexicographic order. Owns its storage; thread-affine.
pub struct DistinctRows {
    // Free the array before releasing the context it retains.
    _rows: PairsArray,
    _stream: Rc<StreamInner>,
    data: NonNull<i64>,
    len: usize,
    width: usize,
}

impl DistinctRows {
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Projected columns per tuple.
    #[inline]
    pub fn width(&self) -> usize {
        self.width
    }

    /// All tuples, row-major: `len() * width()` values.
    #[inline]
    pub fn values(&self) -> &[i64] {
        unsafe { slice::from_raw_parts(self.data.as_ptr(), self.len * self.width) }
    }
}

// Check metadata only, never scan array contents. The native synchronous ABI
// guarantees these shapes describe fully initialized, contiguous owning views.
pub(crate) fn check_array(array: ffi::Array, dtype: i32, shape: &[usize]) -> Result<()> {
    let elements = shape.iter().product::<usize>();
    if array.ctx.is_null()
        || unsafe { ffi::mlx_array_dtype(array) } != dtype
        || unsafe { ffi::mlx_array_ndim(array) } != shape.len()
        || unsafe { ffi::mlx_array_size(array) } != elements
    {
        return Err(Error("radix output has invalid dtype or dimensions".into()));
    }
    for (axis, &length) in shape.iter().enumerate() {
        if unsafe { ffi::mlx_array_dim(array, axis as i32) } != length as i32 {
            return Err(Error("radix output has invalid shape".into()));
        }
    }
    if elements != 0 {
        let strides = unsafe { ffi::mlx_array_strides(array) };
        if strides.is_null() {
            return Err(mlx_failure("read radix strides"));
        }
        let mut stride = 1;
        for (axis, &length) in shape.iter().enumerate().rev() {
            if length > 1 && unsafe { *strides.add(axis) } != stride {
                return Err(Error("radix output is not contiguous".into()));
            }
            stride *= length;
        }
    }
    Ok(())
}

pub(crate) fn ready_pointer<T>(pointer: *const T, len: usize) -> Result<NonNull<T>> {
    if len == 0 {
        return Ok(NonNull::dangling());
    }
    let pointer = NonNull::new(pointer.cast_mut())
        .ok_or_else(|| mlx_failure("read completed radix storage"))?;
    if !(pointer.as_ptr() as usize).is_multiple_of(align_of::<T>())
        || len > isize::MAX as usize / size_of::<T>()
    {
        return Err(Error("radix output has invalid alignment or size".into()));
    }
    Ok(pointer)
}

impl Gpu {
    /// Projects, selects, stably groups and constructs the spline on GPU.
    /// Selected signed fields outside the u32 domain are rejected, not narrowed.
    pub fn build_radix_index(
        &self,
        relation: &Relation,
        spec: IndexSpec,
        selection: Option<&Relation>,
    ) -> Result<RadixIndex> {
        let payload_column = self.check_index_inputs(relation, spec, selection)?;
        let mut arrays = Arrays::output();
        native_status(
            unsafe {
                ffi::mccl_build_relation_radix_u32(
                    &mut arrays.0,
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
            "build shared relation radix index",
        )?;
        self.publish_radix_index(arrays)
    }

    /// GPU merge preserving older-before-newer chronology for equal keys.
    /// Both immutable inputs must originate from this context or its clones.
    pub fn merge_radix_indexes(
        &self,
        older: &RadixIndex,
        newer: &RadixIndex,
    ) -> Result<RadixIndex> {
        if !older.belongs_to(self) || !newer.belongs_to(self) {
            return Err(Error(
                "radix index belongs to a different GPU context".into(),
            ));
        }
        older
            .len()
            .checked_add(newer.len())
            .filter(|&len| len <= MAX_ROWS)
            .ok_or_else(|| Error("combined radix index row count exceeds capacity".into()))?;
        let mut arrays = Arrays::output();
        native_status(
            unsafe {
                ffi::mccl_merge_radix_u32(
                    &mut arrays.0,
                    &older.inner.arrays.0,
                    &newer.inner.arrays.0,
                    self.stream.0,
                )
            },
            "merge radix indexes",
        )?;
        self.publish_radix_index(arrays)
    }

    /// Exact equi-join of `probes` against `index`, executed on the GPU.
    ///
    /// Probes are ranked by binary search over the distinct keys, exact u64
    /// output offsets are scanned on the device, and expansion uses fixed
    /// output tiles independent of per-key fanout. A pair count above 1<<30 is
    /// rejected rather than truncated. Synchronous: the result is complete.
    pub fn join_radix(&self, index: &RadixIndex, probes: &U32Array) -> Result<RadixJoin> {
        if !index.belongs_to(self) {
            return Err(Error(
                "radix index belongs to a different GPU context".into(),
            ));
        }
        self.check_context(probes)?;
        let mut pairs = PairsArray(unsafe { ffi::mlx_array_new() });
        native_status(
            unsafe {
                ffi::mccl_radix_join_u32(
                    &mut pairs.0,
                    &index.inner.arrays.0,
                    probes.raw,
                    self.stream.0,
                )
            },
            "join radix index",
        )?;
        clear_mlx_error();
        if pairs.0.ctx.is_null() || unsafe { ffi::mlx_array_ndim(pairs.0) } != 2 {
            return Err(Error("radix join output has invalid dimensions".into()));
        }
        let len = usize::try_from(unsafe { ffi::mlx_array_dim(pairs.0, 0) })
            .ok()
            .filter(|&len| len <= MAX_ROWS)
            .ok_or_else(|| Error("radix join output has invalid length".into()))?;
        check_array(pairs.0, ffi::UINT32, &[len, 2])?;
        let data = ready_pointer(
            unsafe { ffi::mlx_array_data_uint32(pairs.0) }.cast::<[u32; 2]>(),
            len,
        )?;
        Ok(RadixJoin {
            _pairs: pairs,
            _stream: Rc::clone(&self.stream),
            data,
            len,
        })
    }

    /// Keep the source row IDs whose `key_column` value occurs in `index`;
    /// with `item_column`, that key's group must also contain the row's
    /// `item_column` value. Values outside u32 are misses. Runs on the GPU and
    /// returns kept IDs in input order; an ID outside `relation` is an error.
    pub fn semijoin_rows(
        &self,
        rows: SemiJoinRows<'_>,
        relation: &Relation,
        key_column: usize,
        item_column: Option<usize>,
        index: &RadixIndex,
    ) -> Result<U32Array> {
        let raw_rows = self.semijoin_input(rows)?;
        if !relation.belongs_to(self) || !index.belongs_to(self) {
            return Err(Error(
                "semijoin input belongs to a different GPU context".into(),
            ));
        }
        let column = |column: usize| {
            c_int::try_from(column)
                .ok()
                .filter(|_| column < relation.columns())
                .ok_or_else(|| Error("semijoin column is outside the relation".into()))
        };
        let key_column = column(key_column)?;
        let item_column = item_column.map(column).transpose()?.unwrap_or(-1);
        let mut out = self.output(0);
        native_status(
            unsafe {
                ffi::mccl_radix_semijoin_u32(
                    &mut out.raw,
                    raw_rows,
                    relation.raw(),
                    key_column,
                    item_column,
                    &index.inner.arrays.0,
                    self.stream.0,
                )
            },
            "semijoin radix index",
        )?;
        clear_mlx_error();
        if out.raw.ctx.is_null() || unsafe { ffi::mlx_array_ndim(out.raw) } != 1 {
            return Err(Error("semijoin output has invalid dimensions".into()));
        }
        out.len = unsafe { ffi::mlx_array_size(out.raw) };
        check_array(out.raw, ffi::UINT32, &[out.len])?;
        Ok(out)
    }

    /// Distinct tuples of the rows' `columns`, sorted lexicographically on the
    /// GPU; only the result is published. Every projected value must lie in
    /// the u32 domain (an error otherwise, never narrowed); 1 to 4 columns.
    pub fn distinct_rows(
        &self,
        rows: SemiJoinRows<'_>,
        relation: &Relation,
        columns: &[usize],
    ) -> Result<DistinctRows> {
        let raw_rows = self.semijoin_input(rows)?;
        if !relation.belongs_to(self) {
            return Err(Error(
                "distinct rows relation belongs to a different GPU context".into(),
            ));
        }
        let columns = distinct_columns(relation, columns)?;
        let mut out = PairsArray(unsafe { ffi::mlx_array_new() });
        native_status(
            unsafe {
                ffi::mccl_distinct_rows_i64(
                    &mut out.0,
                    raw_rows,
                    relation.raw(),
                    columns.as_ptr(),
                    columns.len() as c_int,
                    self.stream.0,
                )
            },
            "distinct rows",
        )?;
        self.publish_distinct(out, columns.len())
    }

    /// Join `probes` against `join_index` (whose items are `relation` row
    /// IDs), keep rows passing every semijoin step in order, and return the
    /// distinct tuples of `columns`, all as one GPU graph. Only the join's pair
    /// count is read back on the host; step and column semantics match
    /// [`Self::semijoin_rows`] and [`Self::distinct_rows`].
    pub fn join_semijoin_distinct(
        &self,
        join_index: &RadixIndex,
        probes: &U32Array,
        relation: &Relation,
        steps: &[SemiJoinStep<'_>],
        columns: &[usize],
    ) -> Result<DistinctRows> {
        self.check_context(probes)?;
        if !join_index.belongs_to(self)
            || !relation.belongs_to(self)
            || steps.iter().any(|step| !step.index.belongs_to(self))
        {
            return Err(Error(
                "join chain input belongs to a different GPU context".into(),
            ));
        }
        let column = |column: usize| {
            c_int::try_from(column)
                .ok()
                .filter(|_| column < relation.columns())
                .ok_or_else(|| Error("semijoin column is outside the relation".into()))
        };
        let steps = steps
            .iter()
            .map(|step| {
                Ok(ffi::SemiJoinStep {
                    index: &step.index.inner.arrays.0,
                    key_column: column(step.key_column)?,
                    item_column: step.item_column.map(column).transpose()?.unwrap_or(-1),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let columns = distinct_columns(relation, columns)?;
        let mut out = PairsArray(unsafe { ffi::mlx_array_new() });
        native_status(
            unsafe {
                ffi::mccl_radix_join_semijoin_distinct_i64(
                    &mut out.0,
                    &join_index.inner.arrays.0,
                    probes.raw,
                    relation.raw(),
                    steps.as_ptr(),
                    steps.len() as c_int,
                    columns.as_ptr(),
                    columns.len() as c_int,
                    self.stream.0,
                )
            },
            "join, semijoin and distinct",
        )?;
        self.publish_distinct(out, columns.len())
    }

    fn publish_distinct(&self, out: PairsArray, width: usize) -> Result<DistinctRows> {
        clear_mlx_error();
        if out.0.ctx.is_null() || unsafe { ffi::mlx_array_ndim(out.0) } != 2 {
            return Err(Error("distinct rows output has invalid dimensions".into()));
        }
        let len = usize::try_from(unsafe { ffi::mlx_array_dim(out.0, 0) })
            .ok()
            .filter(|&len| len <= MAX_ROWS)
            .ok_or_else(|| Error("distinct rows output has invalid length".into()))?;
        check_array(out.0, ffi::INT64, &[len, width])?;
        let data = ready_pointer(unsafe { ffi::mlx_array_data_int64(out.0) }, len * width)?;
        Ok(DistinctRows {
            _rows: out,
            _stream: Rc::clone(&self.stream),
            data,
            len,
            width,
        })
    }

    fn semijoin_input(&self, rows: SemiJoinRows<'_>) -> Result<ffi::Array> {
        match rows {
            SemiJoinRows::Join(join) => {
                if !Rc::ptr_eq(&join._stream, &self.stream) {
                    return Err(Error("join belongs to a different GPU context".into()));
                }
                Ok(join._pairs.0)
            }
            SemiJoinRows::Rows(rows) => {
                self.check_context(rows)?;
                Ok(rows.raw)
            }
        }
    }

    fn publish_radix_index(&self, arrays: Arrays) -> Result<RadixIndex> {
        let raw = &arrays.0;
        let empty = raw.rows == 0;
        if raw.rows > MAX_ROWS
            || raw.key_count > raw.rows
            || raw.point_count > raw.key_count
            || (raw.key_count == 0) != empty
            || (raw.point_count == 0) != empty
            || (!empty && raw.point_count < raw.key_count.min(2))
            || raw.min_key > raw.max_key
            || raw.max_key > u32::MAX as u64
            || raw.shift != (64 - (raw.max_key - raw.min_key).leading_zeros()).saturating_sub(12)
            || raw.search_error != if empty { 32 } else { 33 }
            || (empty && (raw.min_key != 0 || raw.max_key != 0))
        {
            return Err(Error("radix output has invalid scalar metadata".into()));
        }
        clear_mlx_error();
        check_array(raw.keys, ffi::UINT32, &[raw.key_count])?;
        check_array(raw.offsets, ffi::UINT64, &[raw.key_count + 1])?;
        check_array(raw.items, ffi::UINT32, &[raw.rows])?;
        check_array(raw.points, ffi::UINT64, &[raw.point_count, 2])?;
        check_array(raw.radix, ffi::UINT64, &[RADIX_ENTRIES])?;
        // Native completion is the initialization/hazard boundary. These data
        // accessors only map completed storage; no source or output is scanned.
        let keys = ready_pointer(
            unsafe { ffi::mlx_array_data_uint32(raw.keys) },
            raw.key_count,
        )?;
        let offsets = ready_pointer(
            unsafe { ffi::mlx_array_data_uint64(raw.offsets) }.cast::<usize>(),
            raw.key_count + 1,
        )?;
        let items = ready_pointer(unsafe { ffi::mlx_array_data_uint32(raw.items) }, raw.rows)?;
        let points = ready_pointer(
            unsafe { ffi::mlx_array_data_uint64(raw.points) }.cast::<[u64; 2]>(),
            raw.point_count,
        )?;
        let radix = ready_pointer(
            unsafe { ffi::mlx_array_data_uint64(raw.radix) }.cast::<usize>(),
            RADIX_ENTRIES,
        )?;
        Ok(RadixIndex {
            inner: Rc::new(Completed {
                arrays,
                stream: Rc::clone(&self.stream),
                keys,
                offsets,
                items,
                points,
                radix,
            }),
        })
    }
}
