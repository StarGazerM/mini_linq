//! Immutable shared source storage. Ingestion is explicit and separate from index
//! construction; GPU builds project columns without reading source values on CPU.

use std::{fmt, mem::size_of, ptr, rc::Rc, slice};

use crate::{
    clear_mlx_error, ffi, index::MAX_ROWS, mlx_failure, mlx_status, native_status, Error, Gpu,
    Result, StreamInner, U32Array,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Payload {
    /// Original source row position, including when a selection compacts rows.
    RowId,
    Column(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexSpec {
    pub key_column: usize,
    pub payload: Payload,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Dtype {
    U32,
    I64,
}

// This guard also owns unpublished allocations while a loader is running. A
// loader panic drops the allocation without ever publishing uninitialized data.
struct ArrayOwner(ffi::Array);

impl ArrayOwner {
    fn output() -> Self {
        Self(unsafe { ffi::mlx_array_new() })
    }
}

impl Drop for ArrayOwner {
    fn drop(&mut self) {
        unsafe { ffi::mlx_array_free(self.0) };
    }
}

struct Storage {
    array: ArrayOwner,
    // Drop the array before its stream, also after all external Gpu owners drop.
    stream: Rc<StreamInner>,
    rows: usize,
    columns: usize,
    dtype: Dtype,
}

/// Cheaply cloned, thread-affine ownership of a homogeneous row-major relation.
/// Storage is immutable after ingestion; no mutable views or raw handles escape.
#[derive(Clone)]
pub struct Relation {
    inner: Rc<Storage>,
}

impl fmt::Debug for Relation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Relation")
            .field("rows", &self.len())
            .field("columns", &self.columns())
            .field("dtype", &self.inner.dtype)
            .finish_non_exhaustive()
    }
}

impl Relation {
    pub fn len(&self) -> usize {
        self.inner.rows
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn columns(&self) -> usize {
        self.inner.columns
    }
    pub fn belongs_to(&self, gpu: &Gpu) -> bool {
        Rc::ptr_eq(&self.inner.stream, &gpu.stream)
    }
    pub(crate) fn raw(&self) -> ffi::Array {
        self.inner.array.0
    }
    pub(crate) fn dtype(&self) -> Dtype {
        self.inner.dtype
    }

    fn evaluate(&self, dtype: Dtype) -> Result<()> {
        if self.dtype() != dtype {
            return Err(Error("relation view has the wrong dtype".into()));
        }
        clear_mlx_error();
        mlx_status(
            unsafe { ffi::mlx_array_eval(self.raw()) },
            "evaluate relation",
        )
    }

    /// Evaluates lazy dependencies and borrows immutable storage without a copy.
    /// GPU index construction never invokes this CPU read view.
    pub fn u32_values(&self) -> Result<&[u32]> {
        self.evaluate(Dtype::U32)?;
        if self.is_empty() {
            return Ok(&[]);
        }
        clear_mlx_error();
        let data = unsafe { ffi::mlx_array_data_uint32(self.raw()) };
        if data.is_null() {
            return Err(mlx_failure("read u32 relation"));
        }
        Ok(unsafe { slice::from_raw_parts(data, self.len() * self.columns()) })
    }

    /// Full-width signed values are retained; ingestion performs no narrowing.
    pub fn i64_values(&self) -> Result<&[i64]> {
        self.evaluate(Dtype::I64)?;
        if self.is_empty() {
            return Ok(&[]);
        }
        clear_mlx_error();
        let data = unsafe { ffi::mlx_array_data_int64(self.raw()) };
        if data.is_null() {
            return Err(mlx_failure("read i64 relation"));
        }
        Ok(unsafe { slice::from_raw_parts(data, self.len() * self.columns()) })
    }
}

fn shape(rows: usize, columns: usize, bytes: usize) -> Result<[i32; 2]> {
    if rows > MAX_ROWS || columns == 0 || columns > i32::MAX as usize {
        return Err(Error(
            "relation shape exceeds capacity or has zero columns".into(),
        ));
    }
    rows.checked_mul(columns)
        .and_then(|n| n.checked_mul(bytes))
        .filter(|&n| n <= isize::MAX as usize)
        .ok_or_else(|| Error("relation storage exceeds capacity".into()))?;
    Ok([rows as i32, columns as i32])
}

impl Gpu {
    fn publish_relation(
        &self,
        array: ArrayOwner,
        rows: usize,
        columns: usize,
        dtype: Dtype,
    ) -> Relation {
        Relation {
            inner: Rc::new(Storage {
                array,
                stream: Rc::clone(&self.stream),
                rows,
                columns,
                dtype,
            }),
        }
    }

    /// Fills fresh MLX shared storage directly, once. The loader cannot retain a
    /// mutable alias; panic unwinding drops the unpublished allocation safely.
    pub fn relation_u32<const C: usize>(
        &self,
        rows: usize,
        mut initialize: impl FnMut(usize) -> [u32; C],
    ) -> Result<Relation> {
        let shape = shape(rows, C, size_of::<u32>())?;
        let mut array = ArrayOwner::output();
        let mut data = ptr::null_mut();
        native_status(
            unsafe {
                ffi::mccl_allocate_u32(&mut array.0, shape.as_ptr(), 2, &mut data, self.stream.0)
            },
            "allocate u32 relation",
        )?;
        if rows != 0 && data.is_null() {
            return Err(Error("allocate u32 relation: null nonempty mapping".into()));
        }
        for row in 0..rows {
            unsafe { data.cast::<[u32; C]>().add(row).write(initialize(row)) };
        }
        Ok(self.publish_relation(array, rows, C, Dtype::U32))
    }

    /// Fills signed shared storage without narrowing any field to index width.
    pub fn relation_i64<const C: usize>(
        &self,
        rows: usize,
        mut initialize: impl FnMut(usize) -> [i64; C],
    ) -> Result<Relation> {
        let shape = shape(rows, C, size_of::<i64>())?;
        let mut array = ArrayOwner::output();
        let mut data = ptr::null_mut();
        native_status(
            unsafe {
                ffi::mccl_allocate_i64(&mut array.0, shape.as_ptr(), 2, &mut data, self.stream.0)
            },
            "allocate i64 relation",
        )?;
        if rows != 0 && data.is_null() {
            return Err(Error("allocate i64 relation: null nonempty mapping".into()));
        }
        for row in 0..rows {
            unsafe { data.cast::<[i64; C]>().add(row).write(initialize(row)) };
        }
        Ok(self.publish_relation(array, rows, C, Dtype::I64))
    }

    /// Retains and reshapes an existing contiguous array without evaluating or
    /// copying it. Lazy GPU-produced arrays remain dependencies of future builds.
    pub fn relation_from_u32(&self, array: &U32Array, columns: usize) -> Result<Relation> {
        self.check_context(array)?;
        if columns == 0 || !array.len().is_multiple_of(columns) {
            return Err(Error(
                "relation columns must divide the array length".into(),
            ));
        }
        let rows = array.len() / columns;
        shape(rows, columns, size_of::<u32>())?;
        let mut output = ArrayOwner::output();
        native_status(
            unsafe {
                ffi::mccl_relation_view_u32(&mut output.0, array.raw, columns as i32, self.stream.0)
            },
            "reshape u32 relation",
        )?;
        Ok(self.publish_relation(output, rows, columns, Dtype::U32))
    }

    // Metadata-only validation: safe GPU construction must not scan host values.
    pub(crate) fn check_index_inputs(
        &self,
        relation: &Relation,
        spec: IndexSpec,
        selection: Option<&Relation>,
    ) -> Result<i32> {
        if !relation.belongs_to(self) {
            return Err(Error("relation belongs to a different GPU context".into()));
        }
        if spec.key_column >= relation.columns() {
            return Err(Error("index key column is outside the relation".into()));
        }
        let payload = match spec.payload {
            Payload::RowId => -1,
            Payload::Column(column) if column < relation.columns() => column as i32,
            Payload::Column(_) => {
                return Err(Error("index payload column is outside the relation".into()))
            }
        };
        if let Some(mask) = selection {
            if !mask.belongs_to(self) {
                return Err(Error("selection belongs to a different GPU context".into()));
            }
            if mask.dtype() != Dtype::U32 || mask.columns() != 1 || mask.len() != relation.len() {
                return Err(Error(
                    "selection must be a u32 relation with shape [N, 1]".into(),
                ));
            }
        }
        Ok(payload)
    }
}
