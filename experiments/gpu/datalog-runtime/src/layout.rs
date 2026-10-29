//! Typed, immutable layouts and the erased packed-run kernel ABI.
use crate::{Buffer, Column, Packing, Pool, Result, Run, Word};
use cuda_core::DeviceCopy;
use std::{any::Any, marker::PhantomData, sync::Arc};

/// One immutable sorted run; lane order is recorded independently of its width.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PackedRunView {
    pub ptr: *const u64,
    pub len: u32,
    pub ms_first: u32,
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
    type Logical;
    /// Preserve the scalar's storage bits in the runtime's signed numeric lane.
    fn encode(value: &Self::Logical) -> i64;
    // DECODE ?: Scalar inverse lacks a caller-level opaque-read/publication requirement.
    fn decode(value: i128) -> Self::Logical;
    const COLUMN: Column;
    const MIN: i128 = if Self::COLUMN.is_signed() { -(1i128 << (Self::COLUMN.bits() - 1)) } else { 0 };
    const MAX: i128 = (1i128 << Self::COLUMN.bits()) - 1 + Self::MIN;
}
/// Encode an identity-domain scalar without naming its Rust type in generated code.
pub fn encode_scalar<T: Scalar<Logical = T>>(value: &T) -> i64 {
    T::encode(value)
}

macro_rules! scalars {
    ($($ty:ty => $column:ident),* $(,)?) => {$(
        impl sealed::Scalar for $ty {}
        impl Scalar for $ty {
            type Logical = Self;
            const COLUMN: Column = Column::$column;
            fn encode(value: &Self) -> i64 { *value as i64 }
            // DECODE ?: numeric Scalar inverse has no local logical-consumer evidence.
            fn decode(value: i128) -> Self { value as Self }
        }
    )*};
}
scalars!(u8 => U8, i8 => I8, u16 => U16, i16 => I16,
    u32 => U32, i32 => I32, u64 => U64, i64 => I64);
#[cfg(target_pointer_width = "32")]
scalars!(usize => U32, isize => I32);
#[cfg(target_pointer_width = "64")]
scalars!(usize => U64, isize => I64);
impl sealed::Scalar for bool {}
impl Scalar for bool {
    type Logical = bool;
    fn encode(value: &bool) -> i64 { *value as i64 }
    // DECODE ?: boolean Scalar inverse needs caller evidence rather than type spelling.
    fn decode(value: i128) -> bool { value != 0 }
    const COLUMN: Column = Column::U8;
    const MAX: i128 = 1;
}
impl<T: 'static, C: crate::SymbolCodec<T> + 'static> sealed::Scalar for crate::Symbol<T, C> {}
impl<T: 'static, C: crate::SymbolCodec<T> + 'static> Scalar for crate::Symbol<T, C> {
    type Logical = T;
    fn encode(value: &T) -> i64 { C::code(value) as i64 }
    // DECODE ?: Symbol Scalar delegates dictionary inversion without a caller publication fact.
    fn decode(value: i128) -> T { C::decode(value as u32) }
    const COLUMN: Column = Column::U32;
}

// These domains retain their authored Rust type and methods in device expressions.
// Encoding is an identity representation: coefficients and epoch days, respectively.
impl<const SCALE: u32> sealed::Scalar for cq_load::Decimal<SCALE> {}
impl<const SCALE: u32> Scalar for cq_load::Decimal<SCALE> {
    type Logical = Self;
    const COLUMN: Column = {
        assert!(SCALE <= 18, "Decimal scale must be at most 18");
        Column::I64
    };
    fn encode(value: &Self) -> i64 { value.scaled() }
    // DECODE ?: Decimal construction needs caller read evidence despite identity storage.
    fn decode(value: i128) -> Self {
        const { assert!(SCALE <= 18, "Decimal scale must be at most 18"); }
        // SAFETY: Decimal is transparent over i64; every coefficient is valid
        // at the supported scale. Avoid the host constructor's String error path.
        unsafe { core::mem::transmute::<i64, Self>(value as i64) }
    }
}
impl sealed::Scalar for cq_load::Date {}
impl Scalar for cq_load::Date {
    type Logical = Self;
    const COLUMN: Column = Column::I32;
    fn encode(value: &Self) -> i64 { value.days_since_epoch() as i64 }
    // DECODE ?: epoch-day Date construction carries no caller read or publication fact.
    fn decode(value: i128) -> Self { Self::from_days(value as i32) }
}

/// A tuple of scalar columns in recorded layout order, including `()`.
/// Each tuple supports up to sixteen columns; key and value schemas concatenate.
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
    allocated_run_bytes: usize,
    // Descriptors borrow these immutable runs, never the caller's vector.
    _owner: Box<dyn Any>,
    download: fn(&dyn Any, &Packing, usize) -> Result<Vec<Vec<i128>>>,
    permute: fn(&LayoutStorage, &[usize]) -> Result<Arc<LayoutStorage>>,
}

impl LayoutStorage {
    fn from_runs<W: Word + 'static>(
        pool: Arc<Pool>, packing: Packing, runs: Vec<Run<W>>,
    ) -> Result<Arc<Self>> {
        if packing.bits() > W::BITS {
            return Err("device layout packing exceeds its run word width".into());
        }
        u32::try_from(runs.len()).map_err(|_| "device layout run count exceeds u32")?;
        let mut rows = 0_usize;
        let mut allocated_run_bytes = 0_usize;
        let mut descriptors = Vec::with_capacity(runs.len());
        for run in &runs {
            if !Arc::ptr_eq(&pool, run.buffer().pool()) {
                return Err("device layout runs belong to a different pool".into());
            }
            let len = u32::try_from(run.len()).map_err(|_| "device layout run length exceeds u32")?;
            rows = rows.checked_add(run.len()).ok_or("device layout row count overflow")?;
            allocated_run_bytes = allocated_run_bytes.checked_add(run.allocated_bytes())
                .ok_or("device layout allocation byte count overflow")?;
            if rows > u32::MAX as usize {
                return Err("device layout row count exceeds u32".into());
            }
            descriptors.push(PackedRunView { ptr: run.view().ptr.cast(), len, ms_first: W::MS_FIRST as u32 });
        }
        let fields = packing.packed_fields().collect::<Vec<_>>();
        let device_descriptors = Buffer::from_host_async(pool.clone(), &descriptors)?;
        if crate::trace_kernels() {
            eprint!("gpu_datalog_layout\tdescriptors={:p}\trows={rows}\trun_lengths=", device_descriptors.ptr());
            for (index, descriptor) in descriptors.iter().enumerate() {
                if index != 0 { eprint!(","); }
                eprint!("{}", descriptor.len);
            }
            eprintln!();
        }
        let fields = Buffer::from_host_async(pool, &fields)?;
        fn download<W: Word + 'static>(
            owner: &dyn Any, packing: &Packing, rows: usize,
        ) -> Result<Vec<Vec<i128>>> {
            let runs = owner.downcast_ref::<Vec<Run<W>>>()
                .expect("layout download matches its retained run word type");
            let mut result = Vec::with_capacity(rows);
            for run in runs {
                for word in run.buffer().to_host()? {
                    // DECODE P: to_host_codes diagnostic download unpacks numeric fields while retaining symbol IDs.
                    result.push(packing.unpack(word)?);
                }
            }
            Ok(result)
        }
        fn permute<W: Word + 'static>(
            storage: &LayoutStorage, order: &[usize],
        ) -> Result<Arc<LayoutStorage>> {
            let packing = storage.packing.permute(order)?;
            let pool = storage.runs.pool().clone();
            let source = storage._owner.downcast_ref::<Vec<Run<W>>>()
                .expect("layout permutation matches its retained run word type");
            let sorter = crate::DeviceRuns::<W>::new(pool.clone(), 4)?;
            let mut runs = Vec::with_capacity(source.len());
            for run in source {
                let values = storage.packing.permute_device(run.buffer(), run.len(), order)?;
                // SAFETY: a column permutation is injective, and each retained
                // immutable Run already contains pairwise distinct rows.
                runs.push(unsafe { sorter.sort_known_unique(&values, run.len())? });
            }
            LayoutStorage::from_runs(pool, packing, runs)
        }
        Ok(Arc::new(Self {
            runs: device_descriptors, fields, packing, rows, word_bits: W::BITS, allocated_run_bytes,
            _owner: Box::new(runs), download: download::<W>, permute: permute::<W>,
        }))
    }

    fn view(&self) -> RunsView {
        RunsView {
            runs: self.runs.ptr(), fields: self.fields.ptr(),
            run_count: self.runs.len() as u32, field_count: self.fields.len() as u32,
            word_bits: self.word_bits, bits: self.packing.bits(),
        }
    }
}

/// A materialized packed lookup layout with an explicitly permuted schema.
/// Unlike `DeviceLayout<K, V>`, this provider makes no tuple-type claim.
#[derive(Clone)]
pub struct PackedProvider {
    storage: Arc<LayoutStorage>,
}
impl PackedProvider {
    pub fn view(&self) -> RunsView { self.storage.view() }
    pub fn owner(&self) -> Arc<dyn Any> { self.storage.clone() }
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
        Ok(Self {
            storage: LayoutStorage::from_runs(pool, packing, runs)?,
            marker: PhantomData,
        })
    }

    pub fn pool(&self) -> &Arc<Pool> { self.storage.runs.pool() }
    pub fn len(&self) -> usize { self.storage.rows }
    pub fn is_empty(&self) -> bool { self.storage.rows == 0 }
    /// Initialized packed-row bytes, excluding descriptors and spare capacity.
    pub fn live_bytes(&self) -> usize {
        self.storage.rows * (self.storage.word_bits / 8) as usize
    }
    /// Sum of retained per-run backing bytes and this layout's descriptor bytes.
    /// Runs may alias accepted state, other layouts, or repeated entries in this
    /// layout. This is a retention footprint, not unique pool consumption.
    pub fn allocated_bytes(&self) -> usize {
        self.storage.allocated_run_bytes + self.descriptor_bytes()
    }
    /// Device descriptor storage owned by this layout, excluding shared runs.
    /// Cloning the layout shares these bytes too.
    pub fn descriptor_bytes(&self) -> usize {
        self.storage.runs.allocated_bytes() + self.storage.fields.allocated_bytes()
    }
    pub fn packing(&self) -> &Packing { &self.storage.packing }
    pub fn owner(&self) -> Arc<dyn Any> { self.storage.clone() }
    /// Download rows in physical run order, retaining encoded symbol codes.
    // DECODE P: explicit diagnostic host publication unpacks numeric fields, not symbol dictionaries.
    pub fn to_host_codes(&self) -> Result<Vec<Vec<i128>>> {
        (self.storage.download)(
            self.storage._owner.as_ref(), &self.storage.packing, self.storage.rows)
    }
    pub fn view(&self) -> RunsView {
        self.storage.view()
    }
    /// Materialize a sorted lookup provider without changing the source runs.
    /// `order[destination]` is the source column at that destination.
    pub fn permuted_provider(&self, order: &[usize]) -> Result<PackedProvider> {
        Ok(PackedProvider { storage: (self.storage.permute)(&self.storage, order)? })
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
    use crate::Word;

    /// Read one logical lane, numbered from the least significant end.
    /// # Safety
    /// The layout, run, row and lane must designate live initialized storage.
    #[inline(always)]
    pub unsafe fn lane(view: RunsView, run: u32, row: u32, lane: u32) -> u64 {
        let descriptor = unsafe { *view.runs.add(run as usize) };
        let lanes = view.word_bits / 64;
        let physical = if descriptor.ms_first != 0 { lanes - 1 - lane } else { lane };
        unsafe { *descriptor.ptr.add(row as usize * lanes as usize + physical as usize) }
    }

    /// Read a packed word without assuming native u128 alignment.
    /// # Safety
    /// The selected row must be live and `W::BITS` must equal `view.word_bits`.
    #[inline(always)]
    pub unsafe fn word<W: Word>(view: RunsView, run: u32, row: u32) -> W {
        let mut word = W::zero();
        let mut index = 0;
        while index < W::BITS / 64 {
            word.set_lane(index, unsafe { lane(view, run, row, index) });
            index += 1;
        }
        word
    }

    /// Decode signed domains numerically; u64 returns its bit pattern as i64.
    /// # Safety
    /// The layout and selected run, row and column must be valid on this device.
    #[inline(always)]
    // DECODE V: packed inverse feeds compare_prefix and generated relational reads; P6 compare encoded fields.
    pub unsafe fn decode(view: RunsView, run: u32, row: u32, column: u32) -> i64 {
        // SAFETY: all descriptor and row accesses are covered by the caller.
        let field = unsafe { *view.fields.add(column as usize) };
        let mask = u64::MAX >> (64 - field.bits);
        let offset = field.shift % 64;
        let mut encoded = unsafe { lane(view, run, row, field.shift / 64) } >> offset;
        if offset != 0 && field.bits > 64 - offset {
            encoded |= unsafe { lane(view, run, row, field.shift / 64 + 1) } << (64 - offset);
        }
        let encoded = encoded & mask;
        if field.signed != 0 {
            // DECODE V: signed ordinal inversion precedes prefix comparisons; P6 translate probes to packed order.
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
    /// This never computes `hi + 1`, including for a full-width key.
    /// # Safety
    /// `run` must designate initialized sorted words in a live device layout,
    /// with `W::BITS == view.word_bits`.
    #[inline(always)]
    pub unsafe fn range<W: Word>(view: RunsView, run: u32, lo: W, hi: W) -> (u32, u32) {
        // SAFETY: the caller supplies a valid run descriptor and sorted storage.
        let len = unsafe { (*view.runs.add(run as usize)).len };
        let mut begin = 0;
        let mut end = len;
        while begin < end {
            let middle = begin + (end - begin) / 2;
            if unsafe { word::<W>(view, run, middle) } < lo { begin = middle + 1; } else { end = middle; }
        }
        let first = begin;
        end = len;
        while begin < end {
            let middle = begin + (end - begin) / 2;
            if unsafe { word::<W>(view, run, middle) } <= hi { begin = middle + 1; } else { end = middle; }
        }
        (first, begin)
    }

    /// Find rows matching a leading decoded tuple, without constructing a key.
    /// Unsigned u64 keys are passed as their i64 bit patterns.
    /// # Safety
    /// The sorted run and `columns` leading fields must be live, and `prefix`
    /// must point to `columns` initialized values readable on this device.
    #[inline(always)]
    // DECODE V: prefix lookup reconstructs packed fields during binary search; P6 encode probes and compare packed prefixes.
    pub unsafe fn prefix_range(
        view: RunsView, run: u32, prefix: *const i64, columns: u32,
    ) -> (u32, u32) {
        let len = unsafe { (*view.runs.add(run as usize)).len };
        let mut begin = 0;
        let mut end = len;
        while begin < end {
            let middle = begin + (end - begin) / 2;
            // DECODE V: lower-bound comparison invokes field inversion before ordering; P6 compare encoded prefixes.
            if unsafe { compare_prefix(view, run, middle, prefix, columns) } < 0 {
                begin = middle + 1;
            } else { end = middle; }
        }
        let first = begin;
        end = len;
        while begin < end {
            let middle = begin + (end - begin) / 2;
            // DECODE V: upper-bound comparison invokes field inversion before ordering; P6 retain packed prefix comparisons.
            if unsafe { compare_prefix(view, run, middle, prefix, columns) } <= 0 {
                begin = middle + 1;
            } else { end = middle; }
        }
        (first, begin)
    }

    #[inline(always)]
    // DECODE V: tuple prefix comparison reconstructs each numeric field; P6 compare encoded field order directly.
    unsafe fn compare_prefix(
        view: RunsView, run: u32, row: u32, prefix: *const i64, columns: u32,
    ) -> i32 {
        let mut column = 0;
        while column < columns {
            // DECODE V: prefix search decodes each compared field; P6 compare encoded prefixes directly.
            let value = unsafe { decode(view, run, row, column) };
            let target = unsafe { *prefix.add(column as usize) };
            if value != target {
                let signed = unsafe { (*view.fields.add(column as usize)).signed } != 0;
                let less = if signed { value < target } else { (value as u64) < target as u64 };
                return if less { -1 } else { 1 };
            }
            column += 1;
        }
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, size_of};

    #[test]
    fn scalar_encoding_preserves_identity_storage_bits() {
        assert_eq!(encode_scalar(&u64::MAX), -1);
        // DECODE P: diagnostic unsigned identity assertion exposes the reconstructed maximum.
        assert_eq!(<u64 as Scalar>::decode(-1), u64::MAX);
        assert_eq!(encode_scalar(&i64::MIN), i64::MIN);
        assert_eq!(encode_scalar(&true), 1);
        assert_eq!(encode_scalar(&false), 0);
    }

    #[test]
    fn decimal_and_date_layouts_preserve_authored_scalar_domains() {
        use cq_load::{Date, Decimal};

        type Amount = Decimal<2>;
        assert_eq!(size_of::<Amount>(), size_of::<i64>());
        assert_eq!(align_of::<Amount>(), align_of::<i64>());
        assert_eq!(size_of::<Date>(), size_of::<i32>());
        assert_eq!(align_of::<Date>(), align_of::<i32>());
        assert_eq!(<(Amount, Date) as ScalarTuple>::COLUMNS, &[Column::I64, Column::I32]);
        for coefficient in [i64::MIN, -12345, 0, 12345, i64::MAX] {
            // DECODE P: diagnostic Decimal fixture constructs the expected coefficient answer.
            let amount = Amount::from_scaled(coefficient).unwrap();
            assert_eq!(encode_scalar(&amount), coefficient);
            // DECODE P: diagnostic Decimal round trip exposes each reconstructed coefficient.
            let decoded = <Amount as Scalar>::decode(coefficient as i128);
            assert_eq!(decoded, amount);
            assert_eq!(decoded.scaled(), coefficient);
        }
        // DECODE P: diagnostic scale-18 fixture constructs the expected fractional answer.
        let fine = Decimal::<18>::from_scaled(-1).unwrap();
        // DECODE P: diagnostic fine-scale round trip exposes a reconstructed Decimal.
        assert_eq!(<Decimal<18> as Scalar>::decode(encode_scalar(&fine) as i128), fine);
        for days in [i32::MIN, -1, 0, 1, i32::MAX] {
            // DECODE P: diagnostic Date fixture constructs the expected epoch-day answer.
            let date = Date::from_days(days);
            assert_eq!(encode_scalar(&date), days as i64);
            // DECODE P: diagnostic Date round trip exposes each reconstructed date.
            assert_eq!(<Date as Scalar>::decode(days as i128), date);
        }
        // DECODE P: diagnostic Decimal ordering assertion exposes both inverse values.
        assert!(<Amount as Scalar>::decode(-1) < <Amount as Scalar>::decode(0));
        // DECODE P: diagnostic Date ordering assertion exposes both inverse dates.
        assert!(<Date as Scalar>::decode(-1) < <Date as Scalar>::decode(0));
    }

    #[test]
    fn erased_abi_and_scalar_tuple_packing() {
        assert_eq!(size_of::<PackedRunView>(), 16);
        assert_eq!(size_of::<PackedField>(), 12);
        assert_eq!(size_of::<RunsView>(), 32);
        assert_eq!(align_of::<RunsView>(), 8);
        assert_eq!(DeviceLayout::<(i32,), (u64, u16)>::derive_packing().unwrap(),
            Packing::with_columns(&[Column::I32, Column::U64, Column::U16]).unwrap());
        assert_eq!(DeviceLayout::<(), ()>::derive_packing().unwrap().bits(), 0);
        assert_eq!(DeviceLayout::<(u64, u64), (u8,)>::derive_packing().unwrap().word_bits(), 192);
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
        let runs = [PackedRunView { ptr: lanes.as_ptr(), len: 2, ms_first: 0 }];
        let view = RunsView { runs: runs.as_ptr(), fields: fields.as_ptr(), run_count: 1,
            field_count: 3, word_bits: 128, bits: packing.bits() };
        // SAFETY: the same pointer-only helpers can read these live host arrays.
        unsafe {
            // DECODE P: diagnostic cross-lane i8 minimum inverse exposes the expected host value.
            assert_eq!(device::decode(view, 0, 0, 0), -128);
            // DECODE P: diagnostic cross-lane u64 maximum inverse exposes its signed lane bits.
            assert_eq!(device::decode(view, 0, 0, 1), -1);
            // DECODE P: diagnostic cross-lane i32 minimum inverse exposes the expected host value.
            assert_eq!(device::decode(view, 0, 0, 2), i32::MIN as i64);
            // DECODE P: diagnostic cross-lane i8 maximum inverse exposes the expected host value.
            assert_eq!(device::decode(view, 0, 1, 0), 127);
            // DECODE P: diagnostic cross-lane unsigned zero inverse exposes the expected host value.
            assert_eq!(device::decode(view, 0, 1, 1), 0);
            // DECODE P: diagnostic cross-lane i32 maximum inverse exposes the expected host value.
            assert_eq!(device::decode(view, 0, 1, 2), i32::MAX as i64);
        }
        let signed = Packing::with_columns(&[Column::I64]).unwrap();
        let fields = signed.packed_fields().collect::<Vec<_>>();
        let words = [signed.pack::<u64>(&[i64::MIN as i128]).unwrap(),
            signed.pack::<u64>(&[i64::MAX as i128]).unwrap()];
        let runs = [PackedRunView { ptr: words.as_ptr(), len: 2, ms_first: 0 }];
        let view = RunsView { runs: runs.as_ptr(), fields: fields.as_ptr(), run_count: 1,
            field_count: 1, word_bits: 64, bits: 64 };
        // SAFETY: both words and the field descriptor remain live for each call.
        unsafe {
            // DECODE P: diagnostic full-width signed minimum inverse exposes the expected host value.
            assert_eq!(device::decode(view, 0, 0, 0), i64::MIN);
            // DECODE P: diagnostic full-width signed maximum inverse exposes the expected host value.
            assert_eq!(device::decode(view, 0, 1, 0), i64::MAX);
        }
    }

    #[test]
    fn locate_empty_runs_and_search_full_width_bounds() {
        let words = [0_u64, 0, 0, 1, u64::MAX, u64::MAX];
        let empty = PackedRunView { ptr: words.as_ptr(), len: 0, ms_first: 0 };
        let runs = [empty, PackedRunView { ptr: words.as_ptr(), len: 3, ms_first: 0 }, empty];
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
            assert_eq!(device::range(view, 1, 3_u128, 2), (1, 1));
        }
    }

    // DECODE P: diagnostic wide-layout inverse helper reconstructs fields for exact host assertions.
    fn wide_decode<W: Word>(columns: &[Column], rows: &[Vec<i128>]) {
        let packing = Packing::with_columns(columns).unwrap();
        let fields = packing.packed_fields().collect::<Vec<_>>();
        let words = rows.iter().map(|row| packing.pack::<W>(row).unwrap()).collect::<Vec<_>>();
        let runs = [PackedRunView { ptr: words.as_ptr().cast(), len: rows.len() as u32,
            ms_first: W::MS_FIRST as u32 }];
        let view = RunsView { runs: runs.as_ptr(), fields: fields.as_ptr(), run_count: 1,
            field_count: fields.len() as u32, word_bits: W::BITS, bits: packing.bits() };
        for (index, row) in rows.iter().enumerate() {
            for (column, &expected) in row.iter().enumerate() {
                // SAFETY: descriptors borrow these initialized local arrays.
                // DECODE P: diagnostic wide-word field inverse exposes each expected logical scalar.
                assert_eq!(unsafe { device::decode(view, 0, index as u32, column as u32) }, expected as i64);
            }
            let prefix = row.iter().map(|&value| value as i64).collect::<Vec<_>>();
            for length in 0..=row.len() {
                let first = rows.partition_point(|candidate| candidate[..length] < row[..length]);
                let end = rows.partition_point(|candidate| candidate[..length] <= row[..length]);
                // SAFETY: the sorted words and leading prefix are initialized.
                assert_eq!(unsafe { device::prefix_range(view, 0, prefix.as_ptr(), length as u32) },
                    (first as u32, end as u32));
                let bounds = packing.prefix_range::<W>(&row[..length]).unwrap();
                assert_eq!(unsafe { device::range(view, 0, *bounds.start(), *bounds.end()) },
                    (first as u32, end as u32));
            }
        }
        let absent = [256_i64];
        assert_eq!(unsafe { device::prefix_range(view, 0, absent.as_ptr(), 1) },
            (rows.len() as u32, rows.len() as u32));
    }

    #[test]
    fn wide_erased_decode_and_prefix_search_match_tuples() {
        let rows = vec![
            vec![0, i64::MIN as i128, 0],
            vec![0, -1, u64::MAX as i128],
            vec![0, i64::MAX as i128, 1],
            vec![255, i64::MAX as i128, u64::MAX as i128],
        ];
        // DECODE P: diagnostic three-lane inverse helper exposes reconstructed tuples for exact assertions.
        wide_decode::<[u64; 3]>(&[Column::U8, Column::I64, Column::U64], &rows);
        let rows = rows.into_iter().map(|mut row| {
            row.push(u64::MAX as i128 - row[0]);
            row
        }).collect::<Vec<_>>();
        // DECODE P: diagnostic four-lane inverse helper exposes reconstructed tuples for exact assertions.
        wide_decode::<[u64; 4]>(&[Column::U8, Column::I64, Column::U64, Column::U64], &rows);
    }
}
