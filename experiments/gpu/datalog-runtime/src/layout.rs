//! Typed, immutable layouts and the erased packed-run kernel ABI.
use crate::{Buffer, Column, Packing, Pool, Result, Run, Word};
use cuda_core::DeviceCopy;
use std::{any::Any, marker::PhantomData, sync::Arc};

/// One immutable sorted run, stored as low-u64 then high-u64 lanes.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PackedRunView {
    pub ptr: *const u64,
    pub len: u32,
    pub _padding: u32,
}
// SAFETY: the descriptor contains only a device pointer and initialized integers.
unsafe impl DeviceCopy for PackedRunView {}

/// A most-significant-first packed column. Signed values use sign-bit bias.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PackedField {
    pub bits: u32,
    pub shift: u32,
    pub signed: u32,
}
// SAFETY: all fields are plain integers with no host-side ownership.
unsafe impl DeviceCopy for PackedField {}

/// Borrowed kernel ABI; retain the layout owner through all stream-ordered reads.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RunsView {
    pub runs: *const PackedRunView,
    pub fields: *const PackedField,
    pub run_count: u32,
    pub field_count: u32,
    pub word_bits: u32,
    pub bits: u32,
}
// SAFETY: this C-layout value contains only device pointers and integers.
unsafe impl DeviceCopy for RunsView {}

mod sealed {
    pub trait Scalar {}
    pub trait Tuple {}
}

/// Native scalar domains accepted by a device layout.
pub trait Scalar: sealed::Scalar + 'static {
    const COLUMN: Column;
}
macro_rules! scalars {
    ($($ty:ty => $column:ident),* $(,)?) => {$(
        impl sealed::Scalar for $ty {}
        impl Scalar for $ty { const COLUMN: Column = Column::$column; }
    )*};
}
scalars!(u8 => U8, i8 => I8, u16 => U16, i16 => I16,
    u32 => U32, i32 => I32, u64 => U64, i64 => I64);

/// A tuple of scalar columns in recorded layout order, including `()`.
/// Packing remains bounded to 128 bits, so at most sixteen u8 columns fit.
pub trait ScalarTuple: sealed::Tuple + 'static {
    const COLUMNS: &'static [Column];
}
impl sealed::Tuple for () {}
impl ScalarTuple for () { const COLUMNS: &'static [Column] = &[]; }
macro_rules! tuples {
    ($first:ident $(, $rest:ident)*) => {
        impl<$first: Scalar, $($rest: Scalar),*> sealed::Tuple for ($first, $($rest,)*) {}
        impl<$first: Scalar, $($rest: Scalar),*> ScalarTuple for ($first, $($rest,)*) {
            const COLUMNS: &'static [Column] = &[$first::COLUMN, $($rest::COLUMN),*];
        }
        tuples!($($rest),*);
    };
    () => {};
}
tuples!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P);

struct LayoutStorage {
    runs: Buffer<PackedRunView>,
    fields: Buffer<PackedField>,
    packing: Packing,
    rows: usize,
    word_bits: u32,
    // Descriptors borrow these immutable runs, never the caller's vector.
    _owner: Box<dyn Any>,
}

/// An immutable device layout whose key tuple precedes its value tuple.
///
/// Construction uploads descriptors only, preserving run order and empty runs.
/// Clones and [`Self::owner`] retain both descriptors and immutable run storage.
/// All reads must be submitted on [`Self::pool`]'s stream.
pub struct DeviceLayout<K: ScalarTuple, V: ScalarTuple> {
    storage: Arc<LayoutStorage>,
    marker: PhantomData<fn() -> (K, V)>,
}
impl<K: ScalarTuple, V: ScalarTuple> Clone for DeviceLayout<K, V> {
    fn clone(&self) -> Self {
        Self { storage: self.storage.clone(), marker: PhantomData }
    }
}
impl<K: ScalarTuple, V: ScalarTuple> DeviceLayout<K, V> {
    pub fn derive_packing() -> Result<Packing> {
        Packing::build(K::COLUMNS.iter().chain(V::COLUMNS)
            .map(|column| (column.bits(), column.is_signed())))
    }

    /// Retain already sorted, immutable runs with exactly this tuple schema.
    /// All runs must belong to `pool`; an empty vector still records its pool.
    pub fn from_runs<W: Word + 'static>(
        pool: Arc<Pool>,
        packing: Packing,
        runs: Vec<Run<W>>,
    ) -> Result<Self> {
        let columns = K::COLUMNS.iter().chain(V::COLUMNS);
        if packing.len() != K::COLUMNS.len() + V::COLUMNS.len()
            || !packing.packed_fields().zip(columns).all(|(field, column)| {
                field.bits == column.bits() && (field.signed != 0) == column.is_signed()
            })
        {
            return Err("device layout packing differs from its scalar tuple schema".into());
        }
        if packing.bits() > W::BITS {
            return Err("device layout packing exceeds its run word width".into());
        }
        u32::try_from(runs.len()).map_err(|_| "device layout run count exceeds u32")?;
        let mut rows = 0_usize;
        let mut descriptors = Vec::with_capacity(runs.len());
        for run in &runs {
            if !Arc::ptr_eq(&pool, run.buffer().pool()) {
                return Err("device layout runs belong to a different pool".into());
            }
            let len = u32::try_from(run.len()).map_err(|_| "device layout run length exceeds u32")?;
            rows = rows.checked_add(run.len()).ok_or("device layout row count overflow")?;
            if rows > u32::MAX as usize {
                return Err("device layout row count exceeds u32".into());
            }
            descriptors.push(PackedRunView { ptr: run.view().ptr.cast(), len, _padding: 0 });
        }
        let fields = packing.packed_fields().collect::<Vec<_>>();
        let descriptors = Buffer::from_host_async(pool.clone(), &descriptors)?;
        let fields = Buffer::from_host_async(pool, &fields)?;
        Ok(Self {
            storage: Arc::new(LayoutStorage {
                runs: descriptors, fields, packing, rows, word_bits: W::BITS,
                _owner: Box::new(runs),
            }),
            marker: PhantomData,
        })
    }

    pub fn pool(&self) -> &Arc<Pool> { self.storage.runs.pool() }
    pub fn len(&self) -> usize { self.storage.rows }
    pub fn is_empty(&self) -> bool { self.storage.rows == 0 }
    pub fn packing(&self) -> &Packing { &self.storage.packing }
    pub fn owner(&self) -> Arc<dyn Any> { self.storage.clone() }
    pub fn view(&self) -> RunsView {
        RunsView {
            runs: self.storage.runs.ptr(), fields: self.storage.fields.ptr(),
            run_count: self.storage.runs.len() as u32,
            field_count: self.storage.fields.len() as u32,
            word_bits: self.storage.word_bits, bits: self.storage.packing.bits(),
        }
    }
    /// Check the emitter's fixed physical column order without device readback.
    pub fn validate(&self, columns: &[Column]) -> Result<()> {
        if columns.iter().copied().eq(K::COLUMNS.iter().chain(V::COLUMNS).copied()) {
            Ok(())
        } else {
            Err("device layout columns differ from the recorded input schema".into())
        }
    }
}

/// Pointer-only helpers usable inside generated device kernels.
pub mod device {
    use super::RunsView;

    /// Read a packed word without assuming u128 alignment of its storage.
    /// # Safety
    /// `view`, `run`, and `row` must refer to live initialized device storage.
    #[inline(always)]
    pub unsafe fn word(view: RunsView, run: u32, row: u32) -> u128 {
        // SAFETY: the caller supplies valid descriptors and row indices.
        let descriptor = unsafe { *view.runs.add(run as usize) };
        let offset = row as usize * (view.word_bits / 64) as usize;
        let low = unsafe { *descriptor.ptr.add(offset) } as u128;
        if view.word_bits == 128 {
            low | ((unsafe { *descriptor.ptr.add(offset + 1) } as u128) << 64)
        } else {
            low
        }
    }

    /// Decode signed domains numerically; u64 returns its bit pattern as i64.
    /// # Safety
    /// The layout and selected run, row and column must be valid on this device.
    #[inline(always)]
    pub unsafe fn decode(view: RunsView, run: u32, row: u32, column: u32) -> i64 {
        // SAFETY: all descriptor and row accesses are covered by the caller.
        let field = unsafe { *view.fields.add(column as usize) };
        let mask = u64::MAX >> (64 - field.bits);
        let encoded = ((unsafe { word(view, run, row) } >> field.shift) as u64) & mask;
        if field.signed != 0 {
            encoded.wrapping_sub(1_u64 << (field.bits - 1)) as i64
        } else {
            encoded as i64
        }
    }

    /// Locate a concatenated row, skipping empty runs. Out-of-bounds rows return
    /// `(view.run_count, 0)`, which must not be passed to `decode` or `range`.
    /// # Safety
    /// All run descriptors must be live and readable on this device.
    #[inline(always)]
    pub unsafe fn locate(view: RunsView, mut global_row: u32) -> (u32, u32) {
        let mut run = 0;
        while run < view.run_count {
            // SAFETY: run is bounded by the caller-provided descriptor count.
            let len = unsafe { (*view.runs.add(run as usize)).len };
            if global_row < len { return (run, global_row); }
            global_row -= len;
            run += 1;
        }
        (view.run_count, 0)
    }

    /// Return a half-open row interval for inclusive packed bounds `[lo, hi]`.
    /// This never computes `hi + 1`, including for a full-width u128 key.
    /// # Safety
    /// `run` must designate initialized sorted words in a live device layout.
    #[inline(always)]
    pub unsafe fn range(view: RunsView, run: u32, lo: u128, hi: u128) -> (u32, u32) {
        // SAFETY: the caller supplies a valid run descriptor and sorted storage.
        let len = unsafe { (*view.runs.add(run as usize)).len };
        let mut begin = 0;
        let mut end = len;
        while begin < end {
            let middle = begin + (end - begin) / 2;
            if unsafe { word(view, run, middle) } < lo { begin = middle + 1; } else { end = middle; }
        }
        let first = begin;
        end = len;
        while begin < end {
            let middle = begin + (end - begin) / 2;
            if unsafe { word(view, run, middle) } <= hi { begin = middle + 1; } else { end = middle; }
        }
        (first, begin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, size_of};

    #[test]
    fn erased_abi_and_scalar_tuple_packing() {
        assert_eq!(size_of::<PackedRunView>(), 16);
        assert_eq!(size_of::<PackedField>(), 12);
        assert_eq!(size_of::<RunsView>(), 32);
        assert_eq!(align_of::<RunsView>(), 8);
        assert_eq!(DeviceLayout::<(i32,), (u64, u16)>::derive_packing().unwrap(),
            Packing::with_columns(&[Column::I32, Column::U64, Column::U16]).unwrap());
        assert_eq!(DeviceLayout::<(), ()>::derive_packing().unwrap().bits(), 0);
        assert!(DeviceLayout::<(u64, u64), (u8,)>::derive_packing().is_err());
    }

    #[test]
    fn decode_signed_extremes_and_cross_lane_fields() {
        let columns = [Column::I8, Column::U64, Column::I32];
        let packing = Packing::with_columns(&columns).unwrap();
        let fields = packing.packed_fields().collect::<Vec<_>>();
        let rows = [[-128, u64::MAX as i128, i32::MIN as i128],
            [127, 0, i32::MAX as i128]];
        let mut lanes = Vec::new();
        for row in rows {
            let word = packing.pack::<u128>(&row).unwrap();
            lanes.extend([word as u64, (word >> 64) as u64]);
        }
        let runs = [PackedRunView { ptr: lanes.as_ptr(), len: 2, _padding: 0 }];
        let view = RunsView { runs: runs.as_ptr(), fields: fields.as_ptr(), run_count: 1,
            field_count: 3, word_bits: 128, bits: packing.bits() };
        // SAFETY: the same pointer-only helpers can read these live host arrays.
        unsafe {
            assert_eq!(device::decode(view, 0, 0, 0), -128);
            assert_eq!(device::decode(view, 0, 0, 1), -1);
            assert_eq!(device::decode(view, 0, 0, 2), i32::MIN as i64);
            assert_eq!(device::decode(view, 0, 1, 0), 127);
            assert_eq!(device::decode(view, 0, 1, 1), 0);
            assert_eq!(device::decode(view, 0, 1, 2), i32::MAX as i64);
        }
        let signed = Packing::with_columns(&[Column::I64]).unwrap();
        let fields = signed.packed_fields().collect::<Vec<_>>();
        let words = [signed.pack::<u64>(&[i64::MIN as i128]).unwrap(),
            signed.pack::<u64>(&[i64::MAX as i128]).unwrap()];
        let runs = [PackedRunView { ptr: words.as_ptr(), len: 2, _padding: 0 }];
        let view = RunsView { runs: runs.as_ptr(), fields: fields.as_ptr(), run_count: 1,
            field_count: 1, word_bits: 64, bits: 64 };
        // SAFETY: both words and the field descriptor remain live for each call.
        unsafe {
            assert_eq!(device::decode(view, 0, 0, 0), i64::MIN);
            assert_eq!(device::decode(view, 0, 1, 0), i64::MAX);
        }
    }

    #[test]
    fn locate_empty_runs_and_search_full_width_bounds() {
        let words = [0_u64, 0, 0, 1, u64::MAX, u64::MAX];
        let empty = PackedRunView { ptr: words.as_ptr(), len: 0, _padding: 0 };
        let runs = [empty, PackedRunView { ptr: words.as_ptr(), len: 3, _padding: 0 }, empty];
        let view = RunsView { runs: runs.as_ptr(), fields: std::ptr::null(), run_count: 3,
            field_count: 0, word_bits: 128, bits: 128 };
        // SAFETY: the local arrays contain three sorted u128 values in u64 lanes.
        unsafe {
            assert_eq!(device::locate(view, 0), (1, 0));
            assert_eq!(device::locate(view, 2), (1, 2));
            assert_eq!(device::locate(view, 3), (3, 0));
            assert_eq!(device::range(view, 0, 0, u128::MAX), (0, 0));
            assert_eq!(device::range(view, 1, 0, u128::MAX), (0, 3));
            assert_eq!(device::range(view, 1, 1_u128 << 64, 1_u128 << 64), (1, 2));
            assert_eq!(device::range(view, 1, u128::MAX, u128::MAX), (2, 3));
            assert_eq!(device::range(view, 1, 3, 2), (1, 1));
        }
    }
}
