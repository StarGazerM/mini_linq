//! Runs in CUDA's `DeviceRadixRun` layout (GPU-RadixRun, STORAGE §8.2),
//! built and merged on the GPU over unified memory.
//!
//! A key is `lanes` u64 words, stored least-significant lane first (CUDA
//! `u64`/`u128`) or most-significant first (CUDA `[u64; L]`). A run holds
//! the distinct keys in increasing order, group offsets (`u32` or `u64`), the
//! stable permutation of source rows (equal keys keep source order), item
//! columns gathered through it, and the exact radix-prefix directory.
//! [`WordRunSet`] publishes deltas as runs under CUDA's ratio law.
//! Builds and merges are synchronous; slices borrow completed shared storage
//! directly, so the CPU reads exactly what the GPU wrote, with no copy.

use std::{
    ffi::c_int,
    fmt,
    mem::{offset_of, size_of},
    ptr::NonNull,
    rc::Rc,
    slice,
    sync::atomic::{AtomicU64, Ordering},
};

use crate::{
    Error, Gpu, Result, StreamInner, clear_mlx_error, ffi, mlx_failure, native_status,
    radix::{check_array, ready_pointer},
};

const _: () = {
    assert!(size_of::<ffi::WordRun>() == 72);
    assert!(offset_of!(ffi::WordRun, rows) == 32);
    assert!(offset_of!(ffi::WordRun, key_count) == 40);
    assert!(offset_of!(ffi::WordRun, lanes) == 48);
    assert!(offset_of!(ffi::WordRun, offset_bytes) == 64);
};

mod native {
    use std::ffi::c_int;

    use crate::ffi::{Array, Stream, WordRun};

    // mlx/c/array.h: MLX_INT32 = 7.
    pub const INT32: c_int = 7;
    extern "C" {
        pub fn mlx_array_data_int32(array: Array) -> *const i32;
        pub fn mccl_build_word_run_items(
            out: *mut WordRun,
            items_out: *mut Array,
            keys: Array,
            items: *const Array,
            item_count: u32,
            lanes: u32,
            ms_first: c_int,
            active_bits: u32,
            radix_bits: u32,
            offset_bytes: u32,
            stream: Stream,
        ) -> c_int;
        pub fn mccl_merge_word_runs(
            out: *mut WordRun,
            items_out: *mut Array,
            older: *const WordRun,
            older_items: *const Array,
            newer: *const WordRun,
            newer_items: *const Array,
            item_count: u32,
            stream: Stream,
        ) -> c_int;
    }
}

/// Largest key, in 64-bit lanes, the Metal kernels accept.
pub const MAX_LANES: usize = 8;
/// Largest directory resolution; `2^bits + 1` entries.
pub const MAX_RADIX_BITS: u32 = 30;
/// Item columns per run: CUDA's item tuple arity.
pub const MAX_ITEM_COLUMNS: usize = 12;
/// Rows of one build or merge (the native row-id domain).
pub const MAX_ROWS: usize = 1 << 30;

/// The group-offset carrier `O`; a build whose rows exceed it is rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OffsetWidth {
    U32,
    U64,
}

impl OffsetWidth {
    /// Whether `items` occurrences (the terminal offset) fit the carrier.
    pub fn fits(self, items: usize) -> bool {
        match self {
            Self::U32 => items <= u32::MAX as usize,
            Self::U64 => u64::try_from(items).is_ok(),
        }
    }

    fn bytes(self) -> u32 {
        match self {
            Self::U32 => 4,
            Self::U64 => 8,
        }
    }
}

/// Caller-recorded layout facts; the runtime chooses none of them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WordRunParams {
    pub lanes: usize,
    /// Memory word 0 is the most significant lane (CUDA `[u64; L]`).
    pub ms_first: bool,
    /// Sorted key bits; bits at or above must be zero (unchecked, as CUDA).
    pub active_bits: u32,
    /// Directory resolution; zero means binary search without a directory.
    pub radix_bits: u32,
    pub offsets: OffsetWidth,
}

impl WordRunParams {
    fn validate(&self) -> Result<()> {
        if self.lanes == 0 || self.lanes > MAX_LANES {
            return Err(Error(
                "word run keys must be whole keys of 1..=8 lanes".into(),
            ));
        }
        if self.active_bits as usize > self.lanes * 64 || self.radix_bits > MAX_RADIX_BITS {
            return Err(Error(
                "word run active or directory bits out of range".into(),
            ));
        }
        Ok(())
    }
}

/// Borrowed offsets or directory entries in the run's carrier.
#[derive(Clone, Copy, Debug)]
pub enum Offsets<'a> {
    U32(&'a [u32]),
    U64(&'a [u64]),
}

impl Offsets<'_> {
    pub fn len(&self) -> usize {
        match self {
            Self::U32(values) => values.len(),
            Self::U64(values) => values.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get(&self, index: usize) -> u64 {
        match self {
            Self::U32(values) => u64::from(values[index]),
            Self::U64(values) => values[index],
        }
    }
}

/// Element type of an item column (CUDA `ItemLane` of 4 or 8 bytes).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ItemType {
    U32,
    I32,
    U64,
    I64,
}

impl ItemType {
    fn dtype(self) -> c_int {
        match self {
            Self::U32 => ffi::UINT32,
            Self::I32 => native::INT32,
            Self::U64 => ffi::UINT64,
            Self::I64 => ffi::INT64,
        }
    }

    fn data(self, array: ffi::Array, len: usize) -> Result<NonNull<u8>> {
        Ok(match self {
            Self::U32 => ready_pointer(unsafe { ffi::mlx_array_data_uint32(array) }, len)?.cast(),
            Self::I32 => ready_pointer(unsafe { native::mlx_array_data_int32(array) }, len)?.cast(),
            Self::U64 => ready_pointer(unsafe { ffi::mlx_array_data_uint64(array) }, len)?.cast(),
            Self::I64 => ready_pointer(unsafe { ffi::mlx_array_data_int64(array) }, len)?.cast(),
        })
    }
}

mod sealed {
    pub trait Sealed {}
}

/// A Rust element type of an item column.
pub trait ItemLane: sealed::Sealed + Copy {
    const TYPE: ItemType;
}

macro_rules! item_lane {
    ($($ty:ty => $kind:ident),+) => {$(
        impl sealed::Sealed for $ty {}
        impl ItemLane for $ty {
            const TYPE: ItemType = ItemType::$kind;
        }
    )+};
}
item_lane!(u32 => U32, i32 => I32, u64 => U64, i64 => I64);

/// Frees one owned MLX array handle.
struct Owned(ffi::Array);

impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.ctx.is_null() {
            unsafe { ffi::mlx_array_free(self.0) };
        }
    }
}

/// Resident build keys: owned uint64 `[rows, lanes]` MLX shared storage, so
/// builds read them in place with no host copy. Thread-affine.
pub struct WordKeys {
    // Free the array before releasing the context it retains.
    array: Owned,
    stream: Rc<StreamInner>,
    rows: usize,
    lanes: usize,
}

impl fmt::Debug for WordKeys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WordKeys")
            .field("rows", &self.rows)
            .field("lanes", &self.lanes)
            .finish_non_exhaustive()
    }
}

impl WordKeys {
    pub fn len(&self) -> usize {
        self.rows
    }

    pub fn is_empty(&self) -> bool {
        self.rows == 0
    }

    pub fn lanes(&self) -> usize {
        self.lanes
    }
}

/// A resident item column: owned one-dimensional MLX shared storage.
pub struct ItemColumn {
    // Free the array before releasing the context it retains.
    array: Owned,
    stream: Rc<StreamInner>,
    rows: usize,
    kind: ItemType,
}

impl fmt::Debug for ItemColumn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ItemColumn")
            .field("rows", &self.rows)
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

impl ItemColumn {
    pub fn len(&self) -> usize {
        self.rows
    }

    pub fn is_empty(&self) -> bool {
        self.rows == 0
    }

    pub fn item_type(&self) -> ItemType {
        self.kind
    }
}

struct Arrays {
    run: ffi::WordRun,
    items: Vec<ffi::Array>,
}

impl Arrays {
    fn output(columns: usize) -> Self {
        Self {
            run: ffi::WordRun {
                keys: unsafe { ffi::mlx_array_new() },
                offsets: unsafe { ffi::mlx_array_new() },
                order: unsafe { ffi::mlx_array_new() },
                directory: unsafe { ffi::mlx_array_new() },
                rows: 0,
                key_count: 0,
                lanes: 0,
                ms_first: 0,
                active_bits: 0,
                radix_bits: 0,
                offset_bytes: 0,
            },
            items: (0..columns)
                .map(|_| unsafe { ffi::mlx_array_new() })
                .collect(),
        }
    }
}

impl Drop for Arrays {
    fn drop(&mut self) {
        let raw = &self.run;
        for array in [raw.keys, raw.offsets, raw.order, raw.directory]
            .into_iter()
            .chain(self.items.iter().copied())
        {
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
    params: WordRunParams,
    keys: NonNull<u64>,
    offsets: NonNull<u8>,
    order: NonNull<u32>,
    directory: NonNull<u8>,
    item_types: Vec<ItemType>,
    items: Vec<NonNull<u8>>,
}

/// Cheaply cloned, thread-affine ownership of one immutable GPU-built run.
#[derive(Clone)]
pub struct WordRun {
    inner: Rc<Completed>,
}

impl fmt::Debug for WordRun {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WordRun")
            .field("rows", &self.len())
            .field("keys", &self.key_count())
            .field("params", &self.inner.params)
            .field("items", &self.inner.item_types)
            .finish_non_exhaustive()
    }
}

impl WordRun {
    /// Occurrences (source rows), not distinct keys.
    pub fn len(&self) -> usize {
        self.inner.arrays.run.rows
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn key_count(&self) -> usize {
        self.inner.arrays.run.key_count
    }

    pub fn params(&self) -> WordRunParams {
        self.inner.params
    }

    /// Distinct keys, `lanes` words each, in the input lane layout.
    pub fn keys(&self) -> &[u64] {
        unsafe {
            slice::from_raw_parts(
                self.inner.keys.as_ptr(),
                self.key_count() * self.inner.params.lanes,
            )
        }
    }

    /// Group starts with the terminal row count, `key_count + 1` entries.
    pub fn offsets(&self) -> Offsets<'_> {
        self.carrier(self.inner.offsets, self.key_count() + 1)
    }

    /// Source row of each occurrence, grouped by key in stable order. A
    /// merged run's rows index the concatenation of its deltas, oldest first.
    pub fn order(&self) -> &[u32] {
        unsafe { slice::from_raw_parts(self.inner.order.as_ptr(), self.len()) }
    }

    /// `2^radix_bits + 1` lower bounds, or empty when `radix_bits` is zero.
    pub fn directory(&self) -> Offsets<'_> {
        let bits = self.inner.params.radix_bits;
        let entries = if bits == 0 { 0 } else { (1 << bits) + 1 };
        self.carrier(self.inner.directory, entries)
    }

    /// Item column types, in build order.
    pub fn item_types(&self) -> &[ItemType] {
        &self.inner.item_types
    }

    /// Item column `column` in stable order: the source column permuted by
    /// [`Self::order`], `len()` entries of exactly its built type.
    pub fn items<T: ItemLane>(&self, column: usize) -> Result<&[T]> {
        match self.inner.item_types.get(column) {
            Some(&kind) if kind == T::TYPE => Ok(unsafe {
                slice::from_raw_parts(self.inner.items[column].as_ptr().cast(), self.len())
            }),
            Some(kind) => Err(Error(format!(
                "word run item column {column} is {kind:?}, not {:?}",
                T::TYPE
            ))),
            None => Err(Error(format!(
                "word run has {} item columns, not {}",
                self.inner.item_types.len(),
                column + 1
            ))),
        }
    }

    fn carrier(&self, data: NonNull<u8>, len: usize) -> Offsets<'_> {
        unsafe {
            match self.inner.params.offsets {
                OffsetWidth::U32 => Offsets::U32(slice::from_raw_parts(data.as_ptr().cast(), len)),
                OffsetWidth::U64 => Offsets::U64(slice::from_raw_parts(data.as_ptr().cast(), len)),
            }
        }
    }
}

impl Gpu {
    /// Copies whole keys (`rows * lanes` words) once into resident storage.
    pub fn upload_word_keys(&self, keys: &[u64], lanes: usize) -> Result<WordKeys> {
        if lanes == 0 || lanes > MAX_LANES || !keys.len().is_multiple_of(lanes) {
            return Err(Error(
                "word run keys must be whole keys of 1..=8 lanes".into(),
            ));
        }
        let rows = keys.len() / lanes;
        let shape = [
            i32::try_from(rows).map_err(|_| Error("word run rows exceed INT_MAX".into()))?,
            lanes as i32,
        ];
        clear_mlx_error();
        // The official constructor copies into a new shared allocation.
        let array = unsafe {
            ffi::mlx_array_new_data(keys.as_ptr().cast(), shape.as_ptr(), 2, ffi::UINT64)
        };
        if array.ctx.is_null() {
            return Err(mlx_failure("create word run keys"));
        }
        Ok(WordKeys {
            array: Owned(array),
            stream: Rc::clone(&self.stream),
            rows,
            lanes,
        })
    }

    /// Copies one item column once into resident storage.
    pub fn upload_items<T: ItemLane>(&self, values: &[T]) -> Result<ItemColumn> {
        let shape = i32::try_from(values.len())
            .map_err(|_| Error("word run item rows exceed INT_MAX".into()))?;
        clear_mlx_error();
        let array =
            unsafe { ffi::mlx_array_new_data(values.as_ptr().cast(), &shape, 1, T::TYPE.dtype()) };
        if array.ctx.is_null() {
            return Err(mlx_failure("create word run items"));
        }
        Ok(ItemColumn {
            array: Owned(array),
            stream: Rc::clone(&self.stream),
            rows: values.len(),
            kind: T::TYPE,
        })
    }

    /// Group `keys` (`rows * lanes` words) into one run without item columns.
    /// An empty input publishes nothing and returns `None`, as CUDA's empty
    /// delta.
    pub fn build_word_run(&self, keys: &[u64], params: WordRunParams) -> Result<Option<WordRun>> {
        params.validate()?;
        let keys = self.upload_word_keys(keys, params.lanes)?;
        self.build_word_run_from(&keys, &[], params)
    }

    /// Group resident keys into one run, gathering `items` (each `rows`
    /// long, at most [`MAX_ITEM_COLUMNS`]) through its stable order. Reads
    /// the inputs in place; an empty input publishes nothing.
    pub fn build_word_run_from(
        &self,
        keys: &WordKeys,
        items: &[&ItemColumn],
        params: WordRunParams,
    ) -> Result<Option<WordRun>> {
        params.validate()?;
        if keys.lanes != params.lanes {
            return Err(Error(
                "word run keys have other lanes than the params".into(),
            ));
        }
        if items.len() > MAX_ITEM_COLUMNS {
            return Err(Error("word run item columns exceed 12".into()));
        }
        if !Rc::ptr_eq(&keys.stream, &self.stream)
            || items
                .iter()
                .any(|item| !Rc::ptr_eq(&item.stream, &self.stream))
        {
            return Err(Error(
                "word run inputs belong to another GPU context".into(),
            ));
        }
        if items.iter().any(|item| item.rows != keys.rows) {
            return Err(Error("word run item columns must have the key rows".into()));
        }
        let rows = keys.rows;
        if rows == 0 {
            return Ok(None);
        }
        if !params.offsets.fits(rows) {
            return Err(Error("word run rows exceed the u32 offset carrier".into()));
        }
        if rows > MAX_ROWS {
            return Err(Error("word run rows exceed 1<<30".into()));
        }
        let sources: Vec<_> = items.iter().map(|item| item.array.0).collect();
        let types: Vec<_> = items.iter().map(|item| item.kind).collect();
        let mut arrays = Arrays::output(items.len());
        clear_mlx_error();
        let status = if items.is_empty() {
            unsafe {
                ffi::mccl_build_word_run(
                    &mut arrays.run,
                    keys.array.0,
                    params.lanes as u32,
                    params.ms_first.into(),
                    params.active_bits,
                    params.radix_bits,
                    params.offsets.bytes(),
                    self.stream.0,
                )
            }
        } else {
            unsafe {
                native::mccl_build_word_run_items(
                    &mut arrays.run,
                    arrays.items.as_mut_ptr(),
                    keys.array.0,
                    sources.as_ptr(),
                    items.len() as u32,
                    params.lanes as u32,
                    params.ms_first.into(),
                    params.active_bits,
                    params.radix_bits,
                    params.offsets.bytes(),
                    self.stream.0,
                )
            }
        };
        native_status(status, "build word run")?;
        self.publish_word_run(arrays, params, types).map(Some)
    }

    /// Merge two runs of identical params and item types into the run a
    /// build of `older ++ newer` publishes: stable merge-path over distinct
    /// keys (equal keys: every older occurrence first), new offsets, newer
    /// order rows rebased after the older rows, merged item columns and a
    /// rebuilt directory. One count readback, as CUDA.
    pub fn merge_word_runs(&self, older: &WordRun, newer: &WordRun) -> Result<WordRun> {
        let (a, b) = (&older.inner, &newer.inner);
        if a.params != b.params || a.item_types != b.item_types {
            return Err(Error(
                "merged word runs must have identical params and item types".into(),
            ));
        }
        if !Rc::ptr_eq(&a.stream, &self.stream) || !Rc::ptr_eq(&b.stream, &self.stream) {
            return Err(Error(
                "word run inputs belong to another GPU context".into(),
            ));
        }
        let rows = older
            .len()
            .checked_add(newer.len())
            .ok_or_else(|| Error("merged word run rows overflow".into()))?;
        if !a.params.offsets.fits(rows) {
            return Err(Error(
                "merged word run rows exceed the u32 offset carrier".into(),
            ));
        }
        if rows > MAX_ROWS {
            return Err(Error("merged word run rows exceed 1<<30".into()));
        }
        let mut arrays = Arrays::output(a.item_types.len());
        clear_mlx_error();
        native_status(
            unsafe {
                native::mccl_merge_word_runs(
                    &mut arrays.run,
                    arrays.items.as_mut_ptr(),
                    &a.arrays.run,
                    a.arrays.items.as_ptr(),
                    &b.arrays.run,
                    b.arrays.items.as_ptr(),
                    a.item_types.len() as u32,
                    self.stream.0,
                )
            },
            "merge word runs",
        )?;
        self.publish_word_run(arrays, a.params, a.item_types.clone())
    }

    fn publish_word_run(
        &self,
        arrays: Arrays,
        params: WordRunParams,
        item_types: Vec<ItemType>,
    ) -> Result<WordRun> {
        let raw = &arrays.run;
        let (rows, keys) = (raw.rows, raw.key_count);
        if keys == 0
            || keys > rows
            || raw.lanes as usize != params.lanes
            || (raw.ms_first != 0) != params.ms_first
            || raw.active_bits != params.active_bits
            || raw.radix_bits != params.radix_bits
        {
            return Err(Error("word run output has invalid scalar metadata".into()));
        }
        let dtype = match params.offsets {
            OffsetWidth::U32 => ffi::UINT32,
            OffsetWidth::U64 => ffi::UINT64,
        };
        if raw.offset_bytes != params.offsets.bytes() {
            return Err(Error("word run output has another offset carrier".into()));
        }
        let entries = if params.radix_bits == 0 {
            0
        } else {
            (1usize << params.radix_bits) + 1
        };
        clear_mlx_error();
        check_array(raw.keys, ffi::UINT64, &[keys, params.lanes])?;
        check_array(raw.offsets, dtype, &[keys + 1])?;
        check_array(raw.order, ffi::UINT32, &[rows])?;
        check_array(raw.directory, dtype, &[entries])?;
        let keys_ptr = ready_pointer(
            unsafe { ffi::mlx_array_data_uint64(raw.keys) },
            keys * params.lanes,
        )?;
        let order = ready_pointer(unsafe { ffi::mlx_array_data_uint32(raw.order) }, rows)?;
        let carrier = |array: ffi::Array, len: usize| -> Result<NonNull<u8>> {
            Ok(match params.offsets {
                OffsetWidth::U32 => {
                    ready_pointer(unsafe { ffi::mlx_array_data_uint32(array) }, len)?.cast()
                }
                OffsetWidth::U64 => {
                    ready_pointer(unsafe { ffi::mlx_array_data_uint64(array) }, len)?.cast()
                }
            })
        };
        let offsets = carrier(raw.offsets, keys + 1)?;
        let directory = carrier(raw.directory, entries)?;
        let mut items = Vec::with_capacity(item_types.len());
        for (&array, &kind) in arrays.items.iter().zip(&item_types) {
            check_array(array, kind.dtype(), &[rows])?;
            items.push(kind.data(array, rows)?);
        }
        Ok(WordRun {
            inner: Rc::new(Completed {
                arrays,
                stream: Rc::clone(&self.stream),
                params,
                keys: keys_ptr,
                offsets,
                order,
                directory,
                item_types,
                items,
            }),
        })
    }
}

static NEXT_SNAPSHOT: AtomicU64 = AtomicU64::new(1);

/// An immutable publication of a [`WordRunSet`]: its runs, oldest first.
/// Retains every run after later publications and the owner's drop.
#[derive(Debug)]
pub struct WordRunSnapshot {
    id: u64,
    runs: Vec<WordRun>,
}

impl WordRunSnapshot {
    fn new(runs: Vec<WordRun>) -> Rc<Self> {
        Rc::new(Self {
            id: NEXT_SNAPSHOT.fetch_add(1, Ordering::Relaxed),
            runs,
        })
    }

    /// Process-unique publication identity.
    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn runs(&self) -> &[WordRun] {
        &self.runs
    }

    pub fn run_count(&self) -> usize {
        self.runs.len()
    }

    /// Total occurrences across all runs.
    pub fn item_count(&self) -> usize {
        self.runs.iter().map(WordRun::len).sum()
    }

    /// `(distinct keys, occurrences)` per run, oldest first.
    pub fn run_lengths(&self) -> Vec<(usize, usize)> {
        self.runs
            .iter()
            .map(|run| (run.key_count(), run.len()))
            .collect()
    }
}

/// Owner of a multi-run publication (CUDA `DeviceRadixRun::publish`). Each
/// nonempty delta is grouped into a run, then adjacent newest runs merge
/// while `older.len() / ratio < newer.len()`, so every older run holds at
/// least `ratio` times the next run's occurrences. Thread-affine.
pub struct WordRunSet {
    gpu: Gpu,
    params: WordRunParams,
    ratio: usize,
    item_types: Vec<ItemType>,
    current: Rc<WordRunSnapshot>,
}

impl fmt::Debug for WordRunSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WordRunSet")
            .field("params", &self.params)
            .field("ratio", &self.ratio)
            .field("items", &self.item_types)
            .field("runs", &self.current.run_lengths())
            .finish_non_exhaustive()
    }
}

impl WordRunSet {
    /// An empty set; every delta must carry `item_types` columns.
    pub fn new(
        gpu: &Gpu,
        params: WordRunParams,
        ratio: usize,
        item_types: &[ItemType],
    ) -> Result<Self> {
        params.validate()?;
        if ratio < 2 {
            return Err(Error("word run set ratio must be at least two".into()));
        }
        if item_types.len() > MAX_ITEM_COLUMNS {
            return Err(Error("word run item columns exceed 12".into()));
        }
        Ok(Self {
            gpu: gpu.clone(),
            params,
            ratio,
            item_types: item_types.to_vec(),
            current: WordRunSnapshot::new(Vec::new()),
        })
    }

    pub fn params(&self) -> WordRunParams {
        self.params
    }

    pub fn ratio(&self) -> usize {
        self.ratio
    }

    pub fn item_types(&self) -> &[ItemType] {
        &self.item_types
    }

    pub fn runs(&self) -> &[WordRun] {
        &self.current.runs
    }

    pub fn run_count(&self) -> usize {
        self.current.run_count()
    }

    /// Total occurrences across all runs.
    pub fn item_count(&self) -> usize {
        self.current.item_count()
    }

    /// `(distinct keys, occurrences)` per run, oldest first.
    pub fn run_lengths(&self) -> Vec<(usize, usize)> {
        self.current.run_lengths()
    }

    /// Group the delta, append its run and merge by the ratio law. Empty
    /// deltas are no-ops; a failed build or merge leaves the published runs
    /// (and the current snapshot) unchanged.
    pub fn publish(&mut self, keys: &WordKeys, items: &[&ItemColumn]) -> Result<()> {
        if items.len() != self.item_types.len()
            || items
                .iter()
                .zip(&self.item_types)
                .any(|(item, &kind)| item.kind != kind)
        {
            return Err(Error(
                "word run set delta item columns differ from the set's types".into(),
            ));
        }
        let Some(mut merged) = self.gpu.build_word_run_from(keys, items, self.params)? else {
            return Ok(());
        };
        let runs = &self.current.runs;
        let mut keep = runs.len();
        while keep > 0 && runs[keep - 1].len() / self.ratio < merged.len() {
            merged = self.gpu.merge_word_runs(&runs[keep - 1], &merged)?;
            keep -= 1;
        }
        let mut next = runs[..keep].to_vec();
        next.push(merged);
        self.current = WordRunSnapshot::new(next);
        Ok(())
    }

    /// The current publication; the same snapshot (same id) until the next
    /// nonempty publish.
    pub fn snapshot(&self) -> Rc<WordRunSnapshot> {
        Rc::clone(&self.current)
    }
}
