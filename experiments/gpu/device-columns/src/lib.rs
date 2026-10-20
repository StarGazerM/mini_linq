//! Device-resident FastLanes columns. The CPU loader uploads each raw column
//! once; headers, unified transposition, delta and bit-packing run on the GPU.
//! Query-output encoding never reads metadata back. All allocations retain the
//! rccl-memory pool and CUDA stream that own them.
mod device;
mod encoding;
mod host;
mod kernels;
mod memory;
use cuda_core::{CudaContext, DeviceCopy, LaunchConfig1D};
pub use device::*;
pub use encoding::{decode_value, encode_value, DecodeValue, Dictionary, EncodeValue};
pub use host::*;
pub use memory::{Buffer, MemoryResource, Pool};
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
        Self::Inclusive { min: value, max: value }
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

fn count_pointer<T: DeviceCopy>(pool: &Arc<Pool>, count: &Buffer<T>, offset: usize) -> Result<*const u64> {
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
}
impl Lane for i64 {
    const ELEMENT: u32 = ELEMENT_I64;
    fn widen(self) -> i64 {
        self
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
impl EncodedColumn {
    /// Loader-only path: raw values cross once, then the GPU encodes. Reading
    /// the compact extent is allowed here, outside the query compute path.
    pub fn from_host<L: Lane>(pool: Arc<Pool>, values: &[L], codec: Codec) -> Result<Self> {
        let input = Buffer::from_host(pool.clone(), values)?;
        let bounds = values.iter().map(|v| v.widen()).fold(Bounds::Empty, |bounds, v| match bounds {
            Bounds::Empty => Bounds::singleton(v),
            Bounds::Inclusive { min, max } => Bounds::Inclusive { min: min.min(v), max: max.max(v) },
        });
        let (headers, extent) = Self::make_headers(
            pool.clone(),
            input.ptr() as *const u8,
            L::ELEMENT,
            values.len(),
            std::ptr::null(),
            codec,
        )?;
        let words = usize::try_from(extent.to_host()?[0])?;
        let data = unsafe { Buffer::uninitialized(pool, words)? };
        Self::pack(&headers, &data, input.ptr() as *const u8, L::ELEMENT, values.len(), std::ptr::null())?;
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

    /// Encode a device result, retaining logical count on the device. No data
    /// or metadata is downloaded. Only the live row prefix is encoded.
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
        let count = count_pointer(&pool, &count_owner, count_offset)?;
        let input = input as *const u8;
        let (headers, extent) = Self::make_headers(pool.clone(), input, L::ELEMENT, capacity, count, codec)?;
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
        Self::pack(&headers, &data, input, L::ELEMENT, capacity, count)?;
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

    fn make_headers(
        pool: Arc<Pool>,
        input: *const u8,
        element: u32,
        capacity: usize,
        count: *const u64,
        codec: Codec,
    ) -> Result<(Buffer<VectorHeader>, Buffer<u64>)> {
        let vectors = capacity.div_ceil(1024);
        // Only the live prefix is written and all kernel readers are count-gated.
        let headers = unsafe { Buffer::uninitialized(pool.clone(), vectors)? };
        let totals = unsafe { Buffer::uninitialized(pool.clone(), vectors.div_ceil(256).max(1))? };
        let module = module(&pool)?;
        let header_launch = module.prepare_headers(config(vectors * 32))?;
        let offset_launch = module.prepare_offsets(LaunchConfig1D::new(
            totals.len().min(568) as u32, 256, 2048,
        ))?;
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
            )?;
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
                next.len().min(568) as u32, 256, 2048,
            ))?;
            unsafe {
                module.scan_offsets(pool.stream(), &launch, current.ptr(), current.len(), next.ptr())?;
            }
            levels.push(next);
        }
        // The final level is the total, not an exclusive prefix.
        for level in (0..levels.len().saturating_sub(2)).rev() {
            let current = &levels[level];
            let launch = module.prepare_add_offsets(config(current.len()))?;
            unsafe {
                module.add_offsets(
                    pool.stream(), &launch, current.ptr(), current.len(), levels[level + 1].ptr(),
                )?;
            }
        }
        if levels.len() > 1 {
            let launch = module.prepare_finish_offsets(config(vectors))?;
            unsafe {
                module.finish_offsets(
                    pool.stream(), &launch, headers.ptr(), capacity as u64, count, levels[0].ptr(),
                )?;
            }
        }
        let extent = levels.pop().unwrap();
        Ok((headers, extent))
    }
    fn pack(
        headers: &Buffer<VectorHeader>,
        data: &Buffer<u64>,
        input: *const u8,
        element: u32,
        capacity: usize,
        count: *const u64,
    ) -> Result<()> {
        if headers.is_empty() {
            return Ok(());
        }
        let pool = data.pool();
        let module = module(pool)?;
        let launch = module.prepare_encode(config(headers.len() * 16))?;
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
    pub fn decoded(&self) -> Result<Buffer<i64>> {
        let output = Buffer::zeroed(self.pool().clone(), self.capacity)?;
        unsafe {
            self.decode_into(output.ptr())?;
        }
        Ok(output)
    }
    /// # Safety
    /// Output must have capacity writable i64 slots on this pool's stream.
    /// Only the live prefix is initialized; inactive slots remain untouched.
    pub unsafe fn decode_into(&self, output: *mut i64) -> Result<()> {
        let module = module(self.pool())?;
        let launch = module.prepare_fastlanes_decode_column(LaunchConfig1D::new(
            self.capacity.div_ceil(1024).clamp(1, 568) as u32,
            256,
            0,
        ))?;
        unsafe {
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
                cuda_core::memory::memcpy_dtoh_async(
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
    pub fn to_host(&self) -> Result<Vec<i64>> {
        let len = self.logical_rows()?;
        if let ColumnData::Plain(data) = &self.data {
            return data.to_host_prefix(len);
        }
        self.decoded()?.to_host_prefix(len)
    }
    /// Explicit diagnostic download of live headers and packed words; never
    /// called by the compute path. Native Plain returns no headers and raw words.
    pub fn inspect(&self) -> Result<(Vec<VectorHeader>, Vec<u64>)> {
        match (&self.headers, &self.extent, &self.data) {
            (Some(headers), Some(extent), ColumnData::Packed(data)) => {
                let words = usize::try_from(extent.to_host()?[0])?;
                let rows = self.logical_rows()?;
                Ok((
                    headers.to_host_prefix(rows.div_ceil(1024))?,
                    data.to_host_prefix(words)?,
                ))
            }
            // Native Plain deliberately has no FastLanes headers or payload.
            (None, None, ColumnData::Plain(_)) => Ok((
                Vec::new(),
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
