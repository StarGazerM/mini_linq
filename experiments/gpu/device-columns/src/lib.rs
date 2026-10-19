//! Device-resident FastLanes columns. The CPU loader uploads each raw column
//! once; headers, unified transposition, delta and bit-packing run on the GPU.
//! Query-output encoding never reads metadata back. All allocations retain the
//! rccl-memory pool and CUDA stream that own them.
mod device;
mod encoding;
mod host;
mod kernels;
mod memory;
use cuda_core::{CudaContext, LaunchConfig1D};
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

/// Immutable encoded data plus per-vector codec facts. Bounds enclose logical
/// values; they are not a relational ordering or uniqueness guarantee.
pub struct EncodedColumn {
    headers: Buffer<VectorHeader>,
    data: Buffer<u64>,
    extent: Buffer<u64>,
    capacity: usize,
    count: *const u64,
    codec: Codec,
    bounds: (i64, i64),
}
impl EncodedColumn {
    /// Loader-only path: raw values cross once, then the GPU encodes. Reading
    /// the compact extent is allowed here, outside the query compute path.
    pub fn from_host(pool: Arc<Pool>, values: &[i64], codec: Codec) -> Result<Self> {
        let input = Buffer::from_host(pool.clone(), values)?;
        let bounds = values
            .iter()
            .fold((i64::MAX, i64::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
        let bounds = if values.is_empty() { (0, -1) } else { bounds };
        let (headers, extent) = Self::make_headers(
            pool.clone(),
            input.ptr(),
            values.len(),
            std::ptr::null(),
            codec,
        )?;
        let words = usize::try_from(extent.to_host()?[0])?;
        let data = Buffer::zeroed(pool, words)?;
        Self::pack(&headers, &data, input.ptr())?;
        Ok(Self {
            headers,
            data,
            extent,
            capacity: values.len(),
            count: std::ptr::null(),
            codec,
            bounds,
        })
    }

    /// Encode a device result, retaining logical count on the device. No data
    /// or metadata is downloaded. Input and count must be ready on pool.stream;
    /// count must outlive this column (the typed relation owns that metadata).
    /// Bounds must enclose all logical values; no sampled bound is sufficient.
    pub unsafe fn from_device(
        pool: Arc<Pool>,
        input: *const i64,
        capacity: usize,
        count: *const u64,
        codec: Codec,
        bounds: (i64, i64),
    ) -> Result<Self> {
        let (headers, extent) = Self::make_headers(pool.clone(), input, capacity, count, codec)?;
        let width = if codec == Codec::Plain || codec == Codec::Delta {
            64
        } else if bounds.0 > bounds.1 {
            0
        } else {
            64 - (bounds.1 as u64)
                .wrapping_sub(bounds.0 as u64)
                .leading_zeros() as usize
        };
        let per_vector = width * 16 + if codec == Codec::Delta { 16 } else { 0 };
        let words = capacity
            .div_ceil(1024)
            .checked_mul(per_vector)
            .ok_or("device encoded capacity overflow")?;
        let data = Buffer::zeroed(pool, words)?;
        Self::pack(&headers, &data, input)?;
        Ok(Self {
            headers,
            data,
            extent,
            capacity,
            count,
            codec,
            bounds,
        })
    }

    fn make_headers(
        pool: Arc<Pool>,
        input: *const i64,
        capacity: usize,
        count: *const u64,
        codec: Codec,
    ) -> Result<(Buffer<VectorHeader>, Buffer<u64>)> {
        let vectors = capacity.div_ceil(1024);
        let headers = Buffer::zeroed(pool.clone(), vectors)?;
        let extent = Buffer::zeroed(pool.clone(), 1)?;
        let module = module(&pool)?;
        let header_launch = module.prepare_headers(config(vectors))?;
        let offset_launch = module.prepare_offsets(config(1))?;
        unsafe {
            module.headers(
                pool.stream(),
                &header_launch,
                input,
                capacity as u64,
                count,
                codec as u32,
                headers.ptr(),
            )?;
            module.offsets(
                pool.stream(),
                &offset_launch,
                headers.ptr(),
                vectors,
                extent.ptr(),
            )?;
        }
        Ok((headers, extent))
    }
    fn pack(headers: &Buffer<VectorHeader>, data: &Buffer<u64>, input: *const i64) -> Result<()> {
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
                headers.ptr(),
                headers.len(),
                data.ptr(),
            )?;
        }
        Ok(())
    }
    pub fn view(&self) -> ColumnView {
        ColumnView {
            headers: self.headers.ptr(),
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
        self.headers.bytes() + self.data.bytes() + self.extent.bytes()
    }
    pub fn bounds(&self) -> (i64, i64) {
        self.bounds
    }
    pub fn codec(&self) -> Codec {
        self.codec
    }
    pub fn pool(&self) -> &Arc<Pool> {
        self.data.pool()
    }
    pub fn extent_ptr(&self) -> *const u64 {
        self.extent.ptr()
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
    /// Output must have capacity initialized i64 slots on this pool's stream.
    pub unsafe fn decode_into(&self, output: *mut i64) -> Result<()> {
        let module = module(self.pool())?;
        let launch = module.prepare_fastlanes_decode_column(config(self.capacity))?;
        unsafe {
            module.fastlanes_decode_column(self.pool().stream(), &launch, self.view(), output)?;
        }
        Ok(())
    }
    pub fn to_host(&self) -> Result<Vec<i64>> {
        let len = if self.count.is_null() {
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
        };
        self.decoded()?.to_host_prefix(len)
    }
    /// Explicit diagnostic download; never called by the compute path.
    pub fn inspect(&self) -> Result<(Vec<VectorHeader>, Vec<u64>)> {
        let words = usize::try_from(self.extent.to_host()?[0])?;
        Ok((self.headers.to_host()?, self.data.to_host_prefix(words)?))
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
