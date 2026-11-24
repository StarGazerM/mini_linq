//! Runs in CUDA's `DeviceRadixRun` layout (GPU-RadixRun, STORAGE §8.2),
//! built on the GPU over unified memory.
//!
//! A key is `lanes` u64 words, stored least-significant lane first (CUDA
//! `u64`/`u128`) or most-significant first (CUDA `[u64; L]`). A run holds
//! the distinct keys in increasing order, group offsets (`u32` or `u64`), the
//! stable permutation of source rows (equal keys keep source order; item
//! columns are gathered through it) and the exact radix-prefix directory.
//! Builds are synchronous; slices borrow completed shared storage directly,
//! so the CPU reads exactly what the GPU wrote, with no copy.

use std::{
    fmt,
    mem::{offset_of, size_of},
    ptr::NonNull,
    rc::Rc,
    slice,
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

/// Largest key, in 64-bit lanes, the Metal kernels accept.
pub const MAX_LANES: usize = 8;
/// Largest directory resolution; `2^bits + 1` entries.
pub const MAX_RADIX_BITS: u32 = 30;

/// The group-offset carrier `O`; a build whose rows exceed it is rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OffsetWidth {
    U32,
    U64,
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

struct Arrays(ffi::WordRun);

impl Arrays {
    fn output() -> Self {
        Self(ffi::WordRun {
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
        })
    }
}

impl Drop for Arrays {
    fn drop(&mut self) {
        let raw = &self.0;
        for array in [raw.keys, raw.offsets, raw.order, raw.directory] {
            if !array.ctx.is_null() {
                unsafe { ffi::mlx_array_free(array) };
            }
        }
    }
}

struct Completed {
    // Free the arrays before releasing the context they retain.
    arrays: Arrays,
    _stream: Rc<StreamInner>,
    params: WordRunParams,
    keys: NonNull<u64>,
    offsets: NonNull<u8>,
    order: NonNull<u32>,
    directory: NonNull<u8>,
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
            .finish_non_exhaustive()
    }
}

impl WordRun {
    /// Occurrences (source rows), not distinct keys.
    pub fn len(&self) -> usize {
        self.inner.arrays.0.rows
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn key_count(&self) -> usize {
        self.inner.arrays.0.key_count
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

    /// Source row of each occurrence, grouped by key in stable order.
    pub fn order(&self) -> &[u32] {
        unsafe { slice::from_raw_parts(self.inner.order.as_ptr(), self.len()) }
    }

    /// `2^radix_bits + 1` lower bounds, or empty when `radix_bits` is zero.
    pub fn directory(&self) -> Offsets<'_> {
        let bits = self.inner.params.radix_bits;
        let entries = if bits == 0 { 0 } else { (1 << bits) + 1 };
        self.carrier(self.inner.directory, entries)
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
    /// Group `keys` (`rows * lanes` words) into one run. An empty input
    /// publishes nothing and returns `None`, as CUDA's empty delta.
    pub fn build_word_run(&self, keys: &[u64], params: WordRunParams) -> Result<Option<WordRun>> {
        let WordRunParams {
            lanes,
            ms_first,
            active_bits,
            radix_bits,
            offsets,
        } = params;
        if lanes == 0 || lanes > MAX_LANES || !keys.len().is_multiple_of(lanes) {
            return Err(Error(
                "word run keys must be whole keys of 1..=8 lanes".into(),
            ));
        }
        if active_bits as usize > lanes * 64 || radix_bits > MAX_RADIX_BITS {
            return Err(Error(
                "word run active or directory bits out of range".into(),
            ));
        }
        let rows = keys.len() / lanes;
        if rows == 0 {
            return Ok(None);
        }
        if offsets == OffsetWidth::U32 && rows > u32::MAX as usize {
            return Err(Error("word run rows exceed the u32 offset carrier".into()));
        }
        let shape = [
            i32::try_from(rows).map_err(|_| Error("word run rows exceed INT_MAX".into()))?,
            lanes as i32,
        ];
        clear_mlx_error();
        // The official constructor copies into a new shared allocation.
        let source = unsafe {
            ffi::mlx_array_new_data(keys.as_ptr().cast(), shape.as_ptr(), 2, ffi::UINT64)
        };
        if source.ctx.is_null() {
            return Err(mlx_failure("create word run keys"));
        }
        let source = Source(source);
        let offset_bytes = match offsets {
            OffsetWidth::U32 => 4,
            OffsetWidth::U64 => 8,
        };
        let mut arrays = Arrays::output();
        native_status(
            unsafe {
                ffi::mccl_build_word_run(
                    &mut arrays.0,
                    source.0,
                    lanes as u32,
                    ms_first.into(),
                    active_bits,
                    radix_bits,
                    offset_bytes,
                    self.stream.0,
                )
            },
            "build word run",
        )?;
        drop(source);
        self.publish_word_run(arrays, params).map(Some)
    }

    fn publish_word_run(&self, arrays: Arrays, params: WordRunParams) -> Result<WordRun> {
        let raw = &arrays.0;
        let (rows, keys) = (raw.rows, raw.key_count);
        if keys == 0
            || keys > rows
            || raw.lanes as usize != params.lanes
            || (raw.ms_first != 0) != params.ms_first
            || raw.radix_bits != params.radix_bits
        {
            return Err(Error("word run output has invalid scalar metadata".into()));
        }
        let (dtype, width) = match params.offsets {
            OffsetWidth::U32 => (ffi::UINT32, 4),
            OffsetWidth::U64 => (ffi::UINT64, 8),
        };
        if raw.offset_bytes != width {
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
        Ok(WordRun {
            inner: Rc::new(Completed {
                arrays,
                _stream: Rc::clone(&self.stream),
                params,
                keys: keys_ptr,
                offsets,
                order,
                directory,
            }),
        })
    }
}

/// Frees the uploaded key array once the synchronous build no longer needs it.
struct Source(ffi::Array);

impl Drop for Source {
    fn drop(&mut self) {
        unsafe { ffi::mlx_array_free(self.0) };
    }
}
