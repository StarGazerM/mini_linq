//! Device-resident FastLanes columns. The CPU loader uploads each raw column
//! once; headers, unified transposition, delta and bit-packing run on the GPU.
//! Alternatively the loader FOR-packs on the host into the same layout
//! (`host_pack`) and uploads only packed words.
//! Query-output encoding publishes exact logical extrema from transient
//! encode-time summaries. One transfer downloads the final extrema pairs for
//! every field of a publication; allocations retain their pool and CUDA stream.
mod device;
mod domain;
mod encoding;
mod host;
mod host_pack;
mod kernels;
mod memory;
pub mod rank;
pub mod native;
pub mod unique;
pub mod scan;
pub mod tile;
use cuda_core::{CudaContext, DeviceCopy, LaunchConfig1D};
pub use device::*;
pub use domain::{Domain, DomainView, KeyDomain, RANK_SPARSITY_THRESHOLD};
pub use encoding::{decode_value, encode_value, DecodeValue, Dictionary, EncodeValue};
pub use host::*;
pub use host_pack::{check_packed_layout, pack_for};
pub use memory::{Buffer, MemoryResource, Pool, PoolMemoryStats};

/// A host-owned handle with an explicitly declared plain device carrier.
/// Device views do not implement this trait and cannot stand in for an owner.
pub trait HostDeviceHandle {
    type Device: cuda_core::DeviceCopy + Copy;
    fn device(&self) -> Self::Device;
}
impl<T: HostDeviceHandle + ?Sized> HostDeviceHandle for &T {
    type Device = T::Device;
    fn device(&self) -> Self::Device { T::device(*self) }
}
impl HostDeviceHandle for Domain {
    type Device = DomainView;
    fn device(&self) -> Self::Device { self.view() }
}
use std::{cell::RefCell, rc::Rc, sync::Arc};

pub type Result<T, E = Box<dyn std::error::Error>> = std::result::Result<T, E>;
fn failure(code: &'static str, detail: impl Into<String>) -> Box<dyn std::error::Error> {
    format!("{code}: {}", detail.into()).into()
}
thread_local! {
    static MODULE: RefCell<Option<(Arc<CudaContext>, Rc<kernels::codecs::LoadedModule>)>> = const { RefCell::new(None) };
}
fn module(pool: &Pool) -> Result<Rc<kernels::codecs::LoadedModule>> {
    MODULE.with(|cache| {
        if let Some((context, module)) = cache.borrow().as_ref() {
            if Arc::ptr_eq(context, pool.context()) {
                return Ok(module.clone());
            }
        }
        let module = Rc::new(unsafe { kernels::codecs::load(pool.context())? });
        *cache.borrow_mut() = Some((pool.context().clone(), module.clone()));
        Ok(module)
    })
}
pub fn prepare_kernels(pool: &Pool) -> Result<()> {
    module(pool).map(|_| ())
}

/// Release the current thread's codec module after a query has finished.
pub fn release_cached_kernels() {
    MODULE.with(|cache| cache.borrow_mut().take());
    scan::release_cached_kernels();
    native::release_cached_kernels();
    rank::release_cached_kernels();
    unique::release_cached_kernels();
}
fn config(items: usize) -> LaunchConfig1D {
    LaunchConfig1D::new(items.div_ceil(256).clamp(1, 568) as u32, 256, 0)
}

enum ColumnData {
    Packed(Buffer<u64>),
    Plain(Buffer<i64>),
}
impl ColumnData {
    fn ptr(&self) -> *const u64 {
        match self {
            Self::Packed(data) => data.ptr(),
            Self::Plain(data) => data.ptr().cast(),
        }
    }
    fn pool(&self) -> &Arc<Pool> {
        match self {
            Self::Packed(data) => data.pool(),
            Self::Plain(data) => data.pool(),
        }
    }
    fn bytes(&self) -> usize {
        match self {
            Self::Packed(data) => data.bytes(),
            Self::Plain(data) => data.bytes(),
        }
    }
}

/// Conservative value domain. Empty has no extrema; a nonempty domain may
/// still describe a filtered result whose live device count is zero.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Bounds {
    Empty,
    Inclusive { min: i64, max: i64 },
}
impl Bounds {
    pub const fn singleton(value: i64) -> Self {
        Self::Inclusive {
            min: value,
            max: value,
        }
    }
    pub fn from_values(values: &[i64]) -> Self {
        let Some((&first, rest)) = values.split_first() else {
            return Self::Empty;
        };
        let (min, max) = rest.iter().fold((first, first), |(lo, hi), &value| {
            (lo.min(value), hi.max(value))
        });
        Self::Inclusive { min, max }
    }
    pub fn cardinality(self) -> u128 {
        match self {
            Self::Empty => 0,
            Self::Inclusive { min, max } => (max as i128 - min as i128 + 1).max(0) as u128,
        }
    }
}

fn count_pointer<T: DeviceCopy>(
    pool: &Arc<Pool>,
    count: &Buffer<T>,
    offset: usize,
) -> Result<*const u64> {
    if !Arc::ptr_eq(pool, count.pool()) {
        return Err("column and count must share a pool".into());
    }
    if offset.checked_add(8).is_none_or(|end| end > count.bytes())
        || (count.cu_deviceptr() as usize).wrapping_add(offset) % 8 != 0
    {
        return Err("column count must be an aligned in-bounds u64".into());
    }
    Ok((count.cu_deviceptr() + offset as u64) as *const u64)
}

/// Element types the packing kernels read. Each widens exactly to i64.
pub trait Lane: DeviceCopy + Copy + 'static {
    const ELEMENT: u32;
    fn widen(self) -> i64;
    /// Exact i64 view of a lane slice; borrows when the lane already is i64.
    fn widen_all(values: &[Self]) -> std::borrow::Cow<'_, [i64]> {
        values.iter().map(|value| value.widen()).collect()
    }
}
impl Lane for i64 {
    const ELEMENT: u32 = ELEMENT_I64;
    fn widen(self) -> i64 {
        self
    }
    fn widen_all(values: &[Self]) -> std::borrow::Cow<'_, [i64]> {
        std::borrow::Cow::Borrowed(values)
    }
}
impl Lane for u32 {
    const ELEMENT: u32 = ELEMENT_U32;
    fn widen(self) -> i64 {
        self as i64
    }
}
impl Lane for i32 {
    const ELEMENT: u32 = ELEMENT_I32;
    fn widen(self) -> i64 {
        self as i64
    }
}

/// Resident representation of a loader-built base column. Device readers
/// dispatch on each column's recorded codec, so every layout serves the same
/// generated code; only bytes moved, encode work and resident size differ.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BaseLayout {
    /// Raw lane upload, then device FOR headers and bit-packing.
    For,
    /// Raw lane upload, then device 64-bit headered transpose (no packing).
    Plain64,
    /// Native i64 upload: no headers, transposition or packing.
    Native,
    /// Host FOR packing into the device layout; only packed words cross.
    HostFor,
    /// FastLanes DELTA (unified transposed 64-row lanes) per column where
    /// [`detect_integer_codec`] finds it smaller than FOR, FOR otherwise;
    /// raw lane upload, device encode.
    Delta,
}

/// Immutable encoded data plus per-vector codec facts. Bounds enclose logical
/// values; they are not a relational ordering or uniqueness guarantee.
pub struct EncodedColumn {
    headers: Option<Buffer<VectorHeader>>,
    data: ColumnData,
    extent: Option<Buffer<u64>>,
    _count_owner: Option<Arc<dyn std::any::Any>>,
    capacity: usize,
    count: *const u64,
    codec: Codec,
    bounds: Bounds,
}

/// Encoded storage whose transient extrema have not yet been published.
pub struct PendingColumn {
    column: EncodedColumn,
    summaries: Buffer<i64>,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PublicationSummary {
    summaries: *const i64,
    capacity: u64,
    count: *const u64,
}
unsafe impl DeviceCopy for PublicationSummary {}

impl PendingColumn {
    fn descriptor(&self) -> PublicationSummary {
        PublicationSummary {
            summaries: self.summaries.ptr(),
            capacity: self.column.capacity as u64,
            count: self.column.count,
        }
    }

    fn measure(pool: &Arc<Pool>, fields: &[PublicationSummary]) -> Result<Vec<i64>> {
        let records = Buffer::from_host_async(pool.clone(), fields)?;
        let extrema = unsafe { Buffer::<i64>::uninitialized(pool.clone(), fields.len() * 2)? };
        let module = module(pool)?;
        let launch = module.prepare_publication_bounds(LaunchConfig1D::new(
            fields.len().try_into().map_err(|_| "too many publication fields")?, 256, 4096,
        ))?;
        unsafe {
            module.publication_bounds(pool.stream(), &launch, records.ptr(), extrema.ptr())?;
        }
        let values = extrema.to_host()?;
        #[cfg(any(test, feature = "publication-counters"))]
        PUBLICATION_READBACKS.with(|count| count.set(count.get() + 1));
        Ok(values)
    }

    fn finish(mut self) -> Result<EncodedColumn> {
        let values = Self::measure(self.summaries.pool(), &[self.descriptor()])?;
        self.column.bounds = if values[0] > values[1] {
            Bounds::Empty
        } else {
            Bounds::Inclusive { min: values[0], max: values[1] }
        };
        Ok(self.column)
    }

    /// Finalize all fields with one reduction launch and one metadata transfer.
    pub fn publish(pending: Vec<Self>) -> Result<Vec<Arc<EncodedColumn>>> {
        if pending.is_empty() {
            return Ok(Vec::new());
        }
        let pool = pending[0].summaries.pool();
        if pending.iter().any(|field| !Arc::ptr_eq(pool, field.summaries.pool())) {
            return Err("publication fields must share one pool".into());
        }
        let records = pending.iter().map(Self::descriptor).collect::<Vec<_>>();
        let values = Self::measure(pool, &records)?;
        Ok(pending.into_iter().enumerate().map(|(index, mut field)| {
            let (min, max) = (values[index * 2], values[index * 2 + 1]);
            field.column.bounds = if min > max { Bounds::Empty } else { Bounds::Inclusive { min, max } };
            Arc::new(field.column)
        }).collect())
    }
}

#[cfg(any(test, feature = "publication-counters"))]
thread_local! {
    static PUBLICATION_READBACKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Current-thread exact-publication readbacks; optional counter-test instrumentation.
#[cfg(feature = "publication-counters")]
pub fn publication_bounds_readbacks() -> usize {
    PUBLICATION_READBACKS.with(|count| count.get())
}
impl EncodedColumn {
    /// Load native device lanes, optionally selecting `positions` in output
    /// order. Bounds, Delta selection, widening and packing run on the device;
    /// only exact bounds, codec and compact extent are downloaded.
    /// `HostFor` is deliberately unsupported; use the host packing constructor.
    ///
    /// # Safety
    /// `source` must describe `source_rows` initialized, correctly aligned
    /// I32/U32/I64 elements accessible on `pool.stream()`. Every position must
    /// be less than `source_rows`. Source and positions must remain immutable
    /// and alive until the submitted work completes on that stream.
    pub unsafe fn from_native(
        pool: Arc<Pool>,
        source: native::NativeView,
        source_rows: usize,
        positions: Option<&Buffer<u32>>,
        layout: BaseLayout,
    ) -> Result<Self> {
        unsafe { native::load(pool, source, source_rows, positions, layout) }
    }

    /// Gather encoded rows directly into a new resident column, in `positions`
    /// order (including repeats). No full decoded intermediate is allocated.
    /// Bounds describe exactly the selected values; Delta uses the existing
    /// device detection policy on the selected order.
    ///
    /// Source, positions and output must share this exact pool and stream.
    /// Positions outside the source's logical length are rejected on device.
    /// Source allocations remain live through submission, and their pool's
    /// stream-ordered frees follow all consuming launches.
    ///
    /// `HostFor` selects device `For` here: the original source may already
    /// have been host-packed, but gathering never downloads values to repack.
    // DECODE I: selected-column construction gathers reconstructed rows into a new resident representation.
    pub fn from_selected(
        pool: Arc<Pool>,
        source: &Self,
        positions: &Buffer<u32>,
        layout: BaseLayout,
    ) -> Result<Self> {
        native::selected(pool, source, positions, layout)
    }

    /// Loader-only path: raw values cross once, then the GPU encodes. Reading
    /// the compact extent is allowed here, outside the query compute path.
    pub fn from_host<L: Lane>(pool: Arc<Pool>, values: &[L], codec: Codec) -> Result<Self> {
        let input = Buffer::from_host(pool.clone(), values)?;
        let bounds =
            values
                .iter()
                .map(|v| v.widen())
                .fold(Bounds::Empty, |bounds, v| match bounds {
                    Bounds::Empty => Bounds::singleton(v),
                    Bounds::Inclusive { min, max } => Bounds::Inclusive {
                        min: min.min(v),
                        max: max.max(v),
                    },
                });
        let (headers, extent) = Self::make_headers(
            pool.clone(),
            input.ptr() as *const u8,
            L::ELEMENT,
            values.len(),
            std::ptr::null(),
            codec,
            std::ptr::null_mut(),
        )?;
        let words = usize::try_from(extent.to_host()?[0])?;
        let data = unsafe { Buffer::uninitialized(pool, words)? };
        Self::pack(
            &headers,
            &data,
            input.ptr() as *const u8,
            L::ELEMENT,
            values.len(),
            std::ptr::null(),
            std::ptr::null_mut(),
        )?;
        Ok(Self {
            headers: Some(headers),
            data: ColumnData::Packed(data),
            extent: Some(extent),
            _count_owner: None,
            capacity: values.len(),
            count: std::ptr::null(),
            codec,
            bounds,
        })
    }

    /// Loader-only path: build a base column in the selected resident layout.
    /// `For` is exactly [`Self::from_host`] with [`Codec::For`].
    pub fn from_host_layout<L: Lane>(
        pool: Arc<Pool>,
        values: &[L],
        layout: BaseLayout,
    ) -> Result<Self> {
        match layout {
            BaseLayout::For => Self::from_host(pool, values, Codec::For),
            BaseLayout::Plain64 => Self::from_host(pool, values, Codec::Plain),
            BaseLayout::Native => Self::from_host_native(pool, &L::widen_all(values)),
            BaseLayout::HostFor => Self::from_host_packed(pool, &L::widen_all(values)),
            BaseLayout::Delta => {
                let codec = detect_integer_codec(&L::widen_all(values));
                Self::from_host(pool, values, codec)
            }
        }
    }

    /// Loader-only path: upload native i64 values once, with no headers,
    /// transposition or packing. Device readers take their native Plain branch.
    pub fn from_host_native(pool: Arc<Pool>, values: &[i64]) -> Result<Self> {
        Ok(Self {
            headers: None,
            data: ColumnData::Plain(Buffer::from_host(pool, values)?),
            extent: None,
            _count_owner: None,
            capacity: values.len(),
            count: std::ptr::null(),
            codec: Codec::Plain,
            bounds: Bounds::from_values(values),
        })
    }

    /// Loader-only path: FOR-pack on the host into the device encoder's exact
    /// layout ([`pack_for`]), then upload only headers and packed words.
    pub fn from_host_packed(pool: Arc<Pool>, values: &[i64]) -> Result<Self> {
        let (headers, payload) = pack_for(values);
        Self::from_packed_parts(
            pool,
            values.len(),
            &headers,
            &payload,
            Bounds::from_values(values),
        )
    }

    /// Upload FOR headers and payload in the [`pack_for`] layout. The layout
    /// is checked; `bounds` must enclose the logical values.
    pub fn from_packed_parts(
        pool: Arc<Pool>,
        rows: usize,
        headers: &[VectorHeader],
        payload: &[u64],
        bounds: Bounds,
    ) -> Result<Self> {
        check_packed_layout(rows, headers, payload.len())?;
        Ok(Self {
            headers: Some(Buffer::from_host(pool.clone(), headers)?),
            data: ColumnData::Packed(Buffer::from_host(pool.clone(), payload)?),
            extent: Some(Buffer::from_host(pool, &[payload.len() as u64])?),
            _count_owner: None,
            capacity: rows,
            count: std::ptr::null(),
            codec: Codec::For,
            bounds,
        })
    }

    /// Adopt an owned native-i64 result without copying, transposing, clearing,
    /// or launching kernels. The count field is a device u64 at `count_offset`
    /// bytes within `count`; its owner is retained for the column's lifetime.
    /// Bounds must enclose logical values. Count is clamped to buffer capacity.
    pub fn from_plain_buffer<T: DeviceCopy + 'static>(
        input: Buffer<i64>,
        count: Arc<Buffer<T>>,
        count_offset: usize,
        bounds: Bounds,
    ) -> Result<Self> {
        let count_ptr = count_pointer(input.pool(), &count, count_offset)?;
        Ok(Self {
            headers: None,
            capacity: input.len(),
            data: ColumnData::Plain(input),
            extent: None,
            _count_owner: Some(count),
            count: count_ptr,
            codec: Codec::Plain,
            bounds,
        })
    }
    /// Borrow native storage for a device-count-gated operation.
    ///
    /// # Safety
    /// Consumers must read only the initialized logical prefix, not capacity,
    /// and must not mutate this immutable column.
    pub unsafe fn plain_buffer(&self) -> Option<&Buffer<i64>> {
        match &self.data {
            ColumnData::Plain(buffer) => Some(buffer),
            ColumnData::Packed(_) => None,
        }
    }

    /// Encode a device result, retaining logical count on the device. Only the
    /// live prefix is encoded; one exact logical-extrema pair is downloaded.
    /// Supplied bounds size storage; [`Self::bounds`] reports measured values.
    ///
    /// # Safety
    /// Input and count must be ready on pool.stream. The live input prefix must
    /// be initialized; unused capacity need not be. Count must remain unchanged.
    /// Its allocation owner is retained independently of any parent relation.
    /// Bounds must enclose all logical values; no sampled bound is sufficient.
    pub unsafe fn from_device<T: DeviceCopy + 'static, L: Lane>(
        pool: Arc<Pool>,
        input: *const L,
        capacity: usize,
        count_owner: Arc<Buffer<T>>,
        count_offset: usize,
        codec: Codec,
        bounds: Bounds,
    ) -> Result<Self> {
        let pending = unsafe {
            Self::from_device_pending(pool, input, capacity, count_owner, count_offset, codec, bounds)?
        };
        pending.finish()
    }

    /// Encode one field; the publication must finalize all fields together.
    /// # Safety
    /// The requirements of `from_device` apply.
    pub unsafe fn from_device_pending<T: DeviceCopy + 'static, L: Lane>(
        pool: Arc<Pool>,
        input: *const L,
        capacity: usize,
        count_owner: Arc<Buffer<T>>,
        count_offset: usize,
        codec: Codec,
        bounds: Bounds,
    ) -> Result<PendingColumn> {
        let summaries = unsafe {
            Buffer::<i64>::uninitialized(pool.clone(), capacity.div_ceil(1024) * 2)?
        };
        let column = unsafe {
            Self::encode_device(pool, input, capacity, count_owner, count_offset, codec, bounds, summaries.ptr())?
        };
        Ok(PendingColumn { column, summaries })
    }

    /// Encode values with a proven enclosing bound, without remeasuring it.
    /// A permutation retains its source bounds; a subset may retain them
    /// conservatively. Arithmetic must supply a separately proven bound.
    ///
    /// # Safety
    /// The input/count lifetime requirements of `from_device` apply, and
    /// `bounds` must enclose every initialized logical value.
    pub unsafe fn from_device_with_bounds<T: DeviceCopy + 'static, L: Lane>(
        pool: Arc<Pool>,
        input: *const L,
        capacity: usize,
        count_owner: Arc<Buffer<T>>,
        count_offset: usize,
        codec: Codec,
        bounds: Bounds,
    ) -> Result<Self> {
        unsafe { Self::encode_device(pool, input, capacity, count_owner, count_offset, codec, bounds, std::ptr::null_mut()) }
    }

    unsafe fn encode_device<T: DeviceCopy + 'static, L: Lane>(
        pool: Arc<Pool>,
        input: *const L,
        capacity: usize,
        count_owner: Arc<Buffer<T>>,
        count_offset: usize,
        codec: Codec,
        bounds: Bounds,
        summaries: *mut i64,
    ) -> Result<Self> {
        let count = count_pointer(&pool, &count_owner, count_offset)?;
        let input = input as *const u8;
        let (headers, extent) = Self::make_headers(
            pool.clone(), input, L::ELEMENT, capacity, count, codec, summaries,
        )?;
        let width = if codec == Codec::Plain || codec == Codec::Delta {
            64
        } else {
            match bounds {
                Bounds::Empty => 0,
                Bounds::Inclusive { min, max } => {
                    64 - (max as u64).wrapping_sub(min as u64).leading_zeros() as usize
                }
            }
        };
        let per_vector = width * 16 + if codec == Codec::Delta { 16 } else { 0 };
        let words = capacity
            .div_ceil(1024)
            .checked_mul(per_vector)
            .ok_or("device encoded capacity overflow")?;
        let data = unsafe { Buffer::uninitialized(pool, words)? };
        Self::pack(
            &headers, &data, input, L::ELEMENT, capacity, count,
            if codec == Codec::Plain { summaries } else { std::ptr::null_mut() },
        )?;
        Ok(Self {
            headers: Some(headers),
            data: ColumnData::Packed(data),
            extent: Some(extent),
            _count_owner: Some(count_owner),
            capacity,
            count,
            codec,
            bounds,
        })
    }

    /// Gather `source[order[i]]` over the count-gated prefix of `rows` and
    /// FOR-pack in one device pass without materializing the gathered column.
    ///
    /// Each 1024-row vector keeps its own base and width but occupies a fixed
    /// slot sized by `bounds`, so no offset scan or extent readback is needed.
    /// Exact logical extrema are reduced from that same pass and published.
    /// Supports native `u32`, `i32`, and `i64` lanes without narrowing.
    ///
    /// # Safety
    /// `order` must hold initialized indices into `source`'s live rows for the
    /// count-gated prefix, ready on pool.stream. Count must remain unchanged,
    /// and `bounds` must enclose every gathered value.
    // DECODE I: gather wrapper reconstructs then FOR-reencodes a permutation; avoidable reorganization, P4 keep index runs plain.
    pub unsafe fn from_gather<T: DeviceCopy + 'static, L: Lane>(
        pool: Arc<Pool>,
        source: ColumnView,
        order: *const u32,
        rows: usize,
        count_owner: Arc<Buffer<T>>,
        count_offset: usize,
        bounds: Bounds,
    ) -> Result<Self> {
        unsafe {
            // DECODE I: gather delegates source inversion and immediate FOR re-encoding; this reorganization is avoidable.
            Self::from_gather_source::<T, L>(
                pool,
                source.into(),
                order,
                rows,
                count_owner,
                count_offset,
                bounds,
            )
        }
    }

    /// Gather a possibly strided or packed source directly into FOR storage.
    /// A null `order` preserves the source's row order without a permutation.
    ///
    /// # Safety
    /// The source storage and nonnull `order` must remain live on the pool stream.
    /// Every selected index (or every row in the count-gated identity prefix)
    /// must address an initialized source row, and `bounds` must enclose every
    /// gathered value, which must fit the declared lane type. Count is immutable.
    // DECODE I: source gather recomputes frames after value reconstruction; avoidable reorganization, P4 prefer plain indexes.
    pub unsafe fn from_gather_source<T: DeviceCopy + 'static, L: Lane>(
        pool: Arc<Pool>,
        source: SourceView,
        order: *const u32,
        rows: usize,
        count_owner: Arc<Buffer<T>>,
        count_offset: usize,
        bounds: Bounds,
    ) -> Result<Self> {
        let pending = unsafe {
            Self::from_gather_source_pending::<T, L>(pool, source, order, rows, count_owner, count_offset, bounds)?
        };
        pending.finish()
    }

    /// Gather one field without a readback; finalize the whole publication together.
    /// # Safety
    /// The requirements of `from_gather_source` apply.
    pub unsafe fn from_gather_source_pending<T: DeviceCopy + 'static, L: Lane>(
        pool: Arc<Pool>,
        source: SourceView,
        order: *const u32,
        rows: usize,
        count_owner: Arc<Buffer<T>>,
        count_offset: usize,
        bounds: Bounds,
    ) -> Result<PendingColumn> {
        let summaries = unsafe { Buffer::<i64>::uninitialized(pool.clone(), rows.div_ceil(1024) * 2)? };
        let column = unsafe {
            Self::encode_gather::<T, L>(pool, source, order, rows, count_owner, count_offset, bounds, summaries.ptr())?
        };
        Ok(PendingColumn { column, summaries })
    }

    /// Gather/permutate/subset values while retaining proven source bounds.
    /// The retained bounds may enclose more than the selected values; this
    /// method does not claim that a subset preserves exact extrema.
    ///
    /// # Safety
    /// The requirements of `from_gather_source` apply. Bounds must enclose
    /// every selected logical value; they must not be inherited through arithmetic.
    pub unsafe fn from_gather_source_with_bounds<T: DeviceCopy + 'static, L: Lane>(
        pool: Arc<Pool>,
        source: SourceView,
        order: *const u32,
        rows: usize,
        count_owner: Arc<Buffer<T>>,
        count_offset: usize,
        bounds: Bounds,
    ) -> Result<Self> {
        unsafe { Self::encode_gather::<T, L>(pool, source, order, rows, count_owner, count_offset, bounds, std::ptr::null_mut()) }
    }

    unsafe fn encode_gather<T: DeviceCopy + 'static, L: Lane>(
        pool: Arc<Pool>,
        source: SourceView,
        order: *const u32,
        rows: usize,
        count_owner: Arc<Buffer<T>>,
        count_offset: usize,
        bounds: Bounds,
        summaries: *mut i64,
    ) -> Result<Self> {
        let count = count_pointer(&pool, &count_owner, count_offset)?;
        let slot_width = match bounds {
            Bounds::Empty => 0,
            Bounds::Inclusive { min, max } => {
                64 - (max as u64).wrapping_sub(min as u64).leading_zeros()
            }
        };
        let vectors = rows.div_ceil(1024);
        let words = vectors
            .checked_mul(slot_width as usize * 16)
            .ok_or("gathered encoded capacity overflow")?;
        let headers =
            unsafe { Buffer::<VectorHeader>::uninitialized(pool.clone(), vectors.max(1))? };
        let data = unsafe { Buffer::<u64>::uninitialized(pool.clone(), words.max(1))? };
        if vectors > 0 {
            let module = module(&pool)?;
            let launch = module.prepare_gather_encode(LaunchConfig1D::new(
                vectors.div_ceil(8).clamp(1, 568 * 4) as u32,
                256,
                if L::ELEMENT == ELEMENT_I64 {
                    65536
                } else {
                    32768
                },
            ))?;
            unsafe {
                // DECODE I: fused gather launch decodes and immediately re-encodes values; this reorganization is avoidable.
                module.gather_encode(
                    pool.stream(),
                    &launch,
                    source,
                    order,
                    rows as u64,
                    count,
                    slot_width,
                    L::ELEMENT,
                    headers.ptr(),
                    data.ptr(),
                    summaries,
                )?;
            }
        }
        Ok(Self {
            headers: Some(headers),
            data: ColumnData::Packed(data),
            extent: None,
            _count_owner: Some(count_owner),
            capacity: rows,
            count,
            codec: Codec::For,
            bounds,
        })
    }


    fn make_headers(
        pool: Arc<Pool>,
        input: *const u8,
        element: u32,
        capacity: usize,
        count: *const u64,
        codec: Codec,
        summaries: *mut i64,
    ) -> Result<(Buffer<VectorHeader>, Buffer<u64>)> {
        let vectors = capacity.div_ceil(1024);
        // Only the live prefix is written and all kernel readers are count-gated.
        let headers = unsafe { Buffer::uninitialized(pool.clone(), vectors)? };
        let module = module(&pool)?;
        let header_launch = module.prepare_headers(config(vectors * 32))?;
        unsafe {
            module.headers(
                pool.stream(),
                &header_launch,
                input,
                element,
                capacity as u64,
                count,
                codec as u32,
                headers.ptr(),
                summaries,
            )?;
        }
        let extent = Self::make_offsets(&headers, capacity, count)?;
        Ok((headers, extent))
    }

    fn make_offsets(
        headers: &Buffer<VectorHeader>,
        capacity: usize,
        count: *const u64,
    ) -> Result<Buffer<u64>> {
        let pool = headers.pool();
        let vectors = headers.len();
        let totals = unsafe { Buffer::uninitialized(pool.clone(), vectors.div_ceil(256).max(1))? };
        let module = module(pool)?;
        let offset_launch =
            module.prepare_offsets(LaunchConfig1D::new(totals.len().min(568) as u32, 256, 2048))?;
        unsafe {
            module.offsets(
                pool.stream(),
                &offset_launch,
                headers.ptr(),
                capacity as u64,
                count,
                totals.ptr(),
            )?;
        }
        let mut levels = vec![totals];
        while levels.last().unwrap().len() > 1 {
            let current = levels.last().unwrap();
            let next = unsafe { Buffer::uninitialized(pool.clone(), current.len().div_ceil(256))? };
            let launch = module.prepare_scan_offsets(LaunchConfig1D::new(
                next.len().min(568) as u32,
                256,
                2048,
            ))?;
            unsafe {
                module.scan_offsets(
                    pool.stream(),
                    &launch,
                    current.ptr(),
                    current.len(),
                    next.ptr(),
                )?;
            }
            levels.push(next);
        }
        // The final level is the total, not an exclusive prefix.
        for level in (0..levels.len().saturating_sub(2)).rev() {
            let current = &levels[level];
            let launch = module.prepare_add_offsets(config(current.len()))?;
            unsafe {
                module.add_offsets(
                    pool.stream(),
                    &launch,
                    current.ptr(),
                    current.len(),
                    levels[level + 1].ptr(),
                )?;
            }
        }
        if levels.len() > 1 {
            let launch = module.prepare_finish_offsets(config(vectors))?;
            unsafe {
                module.finish_offsets(
                    pool.stream(),
                    &launch,
                    headers.ptr(),
                    capacity as u64,
                    count,
                    levels[0].ptr(),
                )?;
            }
        }
        Ok(levels.pop().unwrap())
    }
    fn pack(
        headers: &Buffer<VectorHeader>,
        data: &Buffer<u64>,
        input: *const u8,
        element: u32,
        capacity: usize,
        count: *const u64,
        summaries: *mut i64,
    ) -> Result<()> {
        if headers.is_empty() {
            return Ok(());
        }
        let pool = data.pool();
        let module = module(pool)?;
        let lanes = if summaries.is_null() { 16 } else { 32 };
        let launch = module.prepare_encode(config(headers.len() * lanes))?;
        unsafe {
            module.encode(
                pool.stream(),
                &launch,
                input,
                element,
                headers.ptr(),
                capacity as u64,
                count,
                data.ptr(),
                summaries,
            )?;
        }
        Ok(())
    }
    /// Borrow a raw device ABI descriptor.
    ///
    /// # Safety
    /// Do not use the descriptor after this column is dropped. All submitted
    /// accesses must use the retained pool's stream, where frees are ordered.
    pub unsafe fn view(&self) -> ColumnView {
        ColumnView {
            headers: self.headers.as_ref().map_or(std::ptr::null(), |h| h.ptr()),
            data: self.data.ptr(),
            len: self.capacity as u64,
            count: self.count,
            codec: self.codec as u32,
        }
    }
    pub fn capacity(&self) -> usize {
        self.capacity
    }
    pub fn bytes(&self) -> usize {
        self.headers.as_ref().map_or(0, Buffer::bytes)
            + self.data.bytes()
            + self.extent.as_ref().map_or(0, Buffer::bytes)
    }
    pub fn bounds(&self) -> Bounds {
        self.bounds
    }
    pub fn codec(&self) -> Codec {
        self.codec
    }
    pub fn pool(&self) -> &Arc<Pool> {
        self.data.pool()
    }
    /// Device-only decoded scratch for builds which consume native integers.
    // DECODE ?: whole-column scratch inverse serves both final answers and device-side build consumers.
    pub fn decoded(&self) -> Result<Buffer<i64>> {
        let output = Buffer::zeroed(self.pool().clone(), self.capacity)?;
        unsafe {
            // DECODE ?: scratch reconstruction inherits decoded caller evidence, not the allocation location.
            self.decode_into(output.ptr())?;
        }
        Ok(output)
    }
    /// # Safety
    /// Output must have capacity writable i64 slots on this pool's stream.
    /// Only the live prefix is initialized; inactive slots remain untouched.
    // DECODE ?: decode_into accepts arbitrary device consumers; no opaque-read or publication fact is recorded.
    pub unsafe fn decode_into(&self, output: *mut i64) -> Result<()> {
        let module = module(self.pool())?;
        let launch = module.prepare_fastlanes_decode_column(LaunchConfig1D::new(
            self.capacity.div_ceil(1024).clamp(1, 568) as u32,
            256,
            0,
        ))?;
        unsafe {
            // DECODE ?: whole-column kernel launch inherits the caller boundary and may precede relational work.
            module.fastlanes_decode_column(self.pool().stream(), &launch, self.view(), output)?;
        }
        Ok(())
    }
    fn logical_rows(&self) -> Result<usize> {
        Ok(if self.count.is_null() {
            self.capacity
        } else {
            let mut len = 0u64;
            unsafe {
                cuda_core::simt::memory::memcpy_dtoh_async(
                    &mut len,
                    self.count as u64,
                    8,
                    self.pool().stream().cu_stream(),
                )?;
            }
            self.pool().stream().synchronize()?;
            usize::try_from(len)?.min(self.capacity)
        })
    }
    // DECODE P: explicit logical column host publication reconstructs stored integer values.
    pub fn to_host(&self) -> Result<Vec<i64>> {
        let len = self.logical_rows()?;
        if let ColumnData::Plain(data) = &self.data {
            return data.to_host_prefix(len);
        }
        // DECODE P: compressed host answer download invokes the column inverse before transfer.
        self.decoded()?.to_host_prefix(len)
    }
    /// Explicit diagnostic download of live headers and packed words; never
    /// called by the compute path. Native Plain returns no headers and raw words.
    pub fn inspect(&self) -> Result<(Vec<VectorHeader>, Vec<u64>)> {
        match (&self.headers, &self.extent, &self.data) {
            (Some(headers), extent, ColumnData::Packed(data)) => {
                // Fixed-slot (gathered) columns have no compact extent.
                let words = match extent {
                    Some(extent) => usize::try_from(extent.to_host()?[0])?,
                    None => data.len(),
                };
                let rows = self.logical_rows()?;
                Ok((
                    headers.to_host_prefix(rows.div_ceil(1024))?,
                    data.to_host_prefix(words)?,
                ))
            }
            // Native Plain deliberately has no FastLanes headers or payload.
            (None, None, ColumnData::Plain(_)) => Ok((
                Vec::new(),
                // DECODE P: diagnostic Plain inspection uses logical publication; its identity branch performs no packed inversion.
                self.to_host()?.into_iter().map(|v| v as u64).collect(),
            )),
            _ => unreachable!(),
        }
    }
}

/// Loader heuristic only: choose delta for sorted or near-sorted integer data
/// when its exact per-vector payload estimate beats FOR, including lane bases.
/// Dictionary/plain choices are supplied by the typed field codec instead.
pub fn detect_integer_codec(values: &[i64]) -> Codec {
    let mut descending = 0usize;
    let mut for_bytes = 0usize;
    let mut delta_bytes = 0usize;
    for vector in values.chunks(1024) {
        let mut low = i64::MAX;
        let mut high = i64::MIN;
        let mut dlow = i64::MAX;
        let mut dhigh = i64::MIN;
        for (i, &value) in vector.iter().enumerate() {
            low = low.min(value);
            high = high.max(value);
            if i != 0 && value < vector[i - 1] {
                descending += 1;
            }
            if i % 64 != 0 {
                let delta = value.wrapping_sub(vector[i - 1]);
                dlow = dlow.min(delta);
                dhigh = dhigh.max(delta);
            }
        }
        let width = 64 - (high as u64).wrapping_sub(low as u64).leading_zeros();
        let delta_width = if dlow > dhigh {
            0
        } else {
            64 - (dhigh as u64).wrapping_sub(dlow as u64).leading_zeros()
        };
        for_bytes += width as usize * 128;
        delta_bytes += delta_width as usize * 128 + 128;
    }
    if descending <= values.len() / 16 && delta_bytes < for_bytes {
        Codec::Delta
    } else {
        Codec::For
    }
}

#[cfg(test)]
mod publication_tests {
    use super::*;

    const FULL: Bounds = Bounds::Inclusive { min: i64::MIN, max: i64::MAX };

    #[test]
    fn bound_preserving_encoding_and_subsets_do_not_remeasure() {
        let pool = Pool::new(0, 1 << 20, 256 << 20).unwrap();
        let values = [-100i64, 3, 7, 200];
        let bounds = Bounds::from_values(&values);
        let input = Buffer::from_host(pool.clone(), &values).unwrap();
        let count = Arc::new(Buffer::from_host(pool.clone(), &[values.len() as u64]).unwrap());
        let before = PUBLICATION_READBACKS.with(|count| count.get());
        for codec in [Codec::For, Codec::Plain, Codec::Delta] {
            let column = unsafe {
                EncodedColumn::from_device_with_bounds(
                    pool.clone(), input.ptr(), values.len(), count.clone(), 0, codec, bounds,
                ).unwrap()
            };
            assert_eq!(column.bounds(), bounds);
            assert_eq!(column.to_host().unwrap(), values);
            let order = Buffer::from_host(pool.clone(), &[2u32, 1]).unwrap();
            let selected_count = Arc::new(Buffer::from_host(pool.clone(), &[2u64]).unwrap());
            let selected = unsafe {
                EncodedColumn::from_gather_source_with_bounds::<_, i64>(
                    pool.clone(), column.view().into(), order.ptr(), 2, selected_count, 0, bounds,
                ).unwrap()
            };
            assert_eq!(selected.to_host().unwrap(), [7, 3]);
            // Filtering removes extrema, but the source's enclosing bound remains valid.
            assert_eq!(selected.bounds(), bounds);
        }
        assert_eq!(PUBLICATION_READBACKS.with(|count| count.get()), before);
    }

    #[test]
    fn multi_column_publication_has_one_bounds_readback() {
        let pool = Pool::new(0, 1 << 20, 256 << 20).unwrap();
        let count = Arc::new(Buffer::from_host(pool.clone(), &[2051u64]).unwrap());
        let mut pending = Vec::new();
        let before = PUBLICATION_READBACKS.with(|count| count.get());
        for field in 0..4i64 {
            let values = (0..2051).map(|row| row as i64 - field * 10_000).collect::<Vec<_>>();
            let input = Buffer::from_host(pool.clone(), &values).unwrap();
            pending.push(unsafe {
                if field % 2 == 0 {
                    EncodedColumn::from_device_pending(
                        pool.clone(), input.ptr(), values.len(), count.clone(), 0,
                        Codec::For, FULL,
                    ).unwrap()
                } else {
                    let source = SourceView::from(ColumnView {
                        headers: std::ptr::null(),
                        data: input.ptr().cast(),
                        len: values.len() as u64,
                        count: std::ptr::null(),
                        codec: Codec::Plain as u32,
                    });
                    EncodedColumn::from_gather_source_pending::<_, i64>(
                        pool.clone(), source, std::ptr::null(), values.len(),
                        count.clone(), 0, FULL,
                    ).unwrap()
                }
            });
        }
        assert_eq!(PUBLICATION_READBACKS.with(|count| count.get()), before);
        let columns = PendingColumn::publish(pending).unwrap();
        assert_eq!(PUBLICATION_READBACKS.with(|count| count.get()), before + 1);
        for (field, column) in columns.iter().enumerate() {
            assert_eq!(column.bounds(), Bounds::Inclusive {
                min: -(field as i64) * 10_000, max: 2050 - field as i64 * 10_000,
            });
        }
    }

    fn values() -> Vec<Vec<i64>> {
        vec![
            vec![],
            vec![i64::MIN],
            vec![i64::MAX],
            vec![-97, -91, -104, -98],
            vec![i64::MIN, 0, i64::MAX, -1],
            (0..2051).map(|i| -10_000 + i * 3).collect(),
            // Delta lane bases participate, independently of residual extrema.
            (0..1025).map(|i| if i % 64 == 0 { -900 } else { -899 }).collect(),
        ]
    }

    #[test]
    fn device_publication_measures_logical_bounds_for_every_codec() {
        let pool = Pool::new(0, 1 << 20, 256 << 20).unwrap();
        for values in values() {
            for codec in [Codec::For, Codec::Plain, Codec::Delta, Codec::Dictionary] {
                for capacity in [values.len(), values.len() + 2050] {
                    let input = unsafe { Buffer::<i64>::uninitialized(pool.clone(), capacity) }.unwrap();
                    unsafe { input.write_async(&values) }.unwrap();
                    let count = Arc::new(Buffer::from_host(pool.clone(), &[
                        OutputCount { rows: values.len() as u64, error: 73, padding: 0 },
                    ]).unwrap());
                    let owner = Arc::downgrade(&count);
                    let column = unsafe {
                        EncodedColumn::from_device(
                            pool.clone(), input.ptr(), capacity, count.clone(), 0, codec, FULL,
                        )
                    }.unwrap();
                    assert_eq!(count.to_host().unwrap()[0].error, 73);
                    drop(count);
                    drop(input);
                    assert!(owner.upgrade().is_some());
                    assert_eq!(column.bounds(), Bounds::from_values(&values));
                    assert_eq!(column.to_host().unwrap(), values);
                    drop(column);
                    assert!(owner.upgrade().is_none());
                }
            }
        }
        release_cached_kernels();
    }

    #[test]
    fn gathered_publication_ignores_uninitialized_count_gated_suffixes() {
        let pool = Pool::new(0, 1 << 20, 256 << 20).unwrap();
        for values in values() {
            let capacity = values.len() + 2050;
            let input = unsafe { Buffer::<i64>::uninitialized(pool.clone(), capacity) }.unwrap();
            unsafe { input.write_async(&values) }.unwrap();
            let order = unsafe { Buffer::<u32>::uninitialized(pool.clone(), capacity) }.unwrap();
            let positions: Vec<u32> = (0..values.len() as u32).rev().collect();
            unsafe { order.write_async(&positions) }.unwrap();
            let count = Arc::new(Buffer::from_host(pool.clone(), &[values.len() as u64]).unwrap());
            let source = ColumnView {
                headers: std::ptr::null(),
                data: input.ptr().cast(),
                len: capacity as u64,
                count: count.ptr(),
                codec: Codec::Plain as u32,
            };
            let column = unsafe {
                EncodedColumn::from_gather::<_, i64>(
                    pool.clone(), source, order.ptr(), capacity, count, 0, FULL,
                )
            }.unwrap();
            drop(order);
            drop(input);
            assert_eq!(column.bounds(), Bounds::from_values(&values));
            assert_eq!(column.to_host().unwrap(), values.into_iter().rev().collect::<Vec<_>>());
        }
        release_cached_kernels();
    }

    #[test]
    fn publication_clamps_counts_and_rejects_bad_count_offsets() {
        let pool = Pool::new(0, 1 << 20, 256 << 20).unwrap();
        let input = Buffer::from_host(pool.clone(), &[-8i64, -4, -6]).unwrap();
        let count = Arc::new(Buffer::from_host(pool.clone(), &[17u64, u64::MAX]).unwrap());
        let column = unsafe {
            EncodedColumn::from_device(
                pool.clone(), input.ptr(), input.len(), count.clone(), 8, Codec::For, FULL,
            )
        }.unwrap();
        assert_eq!(column.bounds(), Bounds::Inclusive { min: -8, max: -4 });
        assert_eq!(column.to_host().unwrap(), [-8, -4, -6]);
        assert!(unsafe {
            EncodedColumn::from_device(
                pool, input.ptr(), input.len(), count.clone(), 1, Codec::For, FULL,
            )
        }.is_err());
        assert_eq!(count.to_host().unwrap(), [17, u64::MAX]);
        release_cached_kernels();
    }
}
