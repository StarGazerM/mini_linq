//! GPU-built grouped u32 runs with completed native-compatible spline storage.
//!
//! Builds and merges are synchronous. Slice getters only borrow immutable MLX
//! storage: they perform no FFI, evaluation, copying, or reference-count updates.
//! This module intentionally has no CPU spline reader; consumers use the native
//! RadixRun reader with these arrays and scalar metadata. Clones retain all five
//! arrays and their originating context, including after sources and Gpu drop.

use std::{
    fmt,
    mem::{align_of, offset_of, size_of},
    ptr::NonNull,
    rc::Rc,
    slice,
};

use crate::{
    Error, Gpu, Result, StreamInner, clear_mlx_error, ffi,
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

// Check metadata only, never scan array contents. The native synchronous ABI
// guarantees these shapes describe fully initialized, contiguous owning views.
fn check_array(array: ffi::Array, dtype: i32, shape: &[usize]) -> Result<()> {
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

fn ready_pointer<T>(pointer: *const T, len: usize) -> Result<NonNull<T>> {
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
