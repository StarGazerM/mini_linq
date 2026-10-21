use crate::Result;
use cuda_core::{CudaContext, CudaEvent, CudaStream, DeviceCopy, PinnedHostBuffer};
use rccl_memory::{
    ContiguousDeviceRange, CudaMemoryResource, DeviceAllocator, DeviceBlock, PoolMemoryResource,
    StatisticsMemoryResource,
};
use std::{
    alloc::Layout,
    marker::PhantomData,
    sync::{Arc, Mutex},
};

pub type MemoryResource = StatisticsMemoryResource<PoolMemoryResource<CudaMemoryResource>>;

fn allocation_error(error: rccl_memory::MemoryError) -> Box<dyn std::error::Error> {
    let exhausted = match &error {
        rccl_memory::MemoryError::AllocationLimitExceeded { .. } => true,
        rccl_memory::MemoryError::Driver(error) => {
            error.0 == cuda_core::sys::cudaError_enum_CUDA_ERROR_OUT_OF_MEMORY
        }
        _ => false,
    };
    if exhausted {
        crate::failure("CQ_GPU_VRAM_EXCEEDED", error.to_string())
    } else {
        Box::new(error)
    }
}

/// Shared resources and stream ownership, not a cache of relation data.
///
/// Asynchronous uploads share a pinned staging arena. Completed chunks are
/// reused without waiting; staging is released only when the pool is dropped.
/// Pool teardown waits for outstanding uploads before freeing their sources.
pub struct Pool {
    memory: MemoryResource,
    stream: Arc<CudaStream>,
    context: Arc<CudaContext>,
    uploads: Mutex<UploadStaging>,
}
impl Pool {
    pub fn new(device: usize, initial: usize, maximum: usize) -> Result<Arc<Self>> {
        let context = CudaContext::new(device)?;
        let stream = context.default_stream();
        let memory = StatisticsMemoryResource::new(
            PoolMemoryResource::new(CudaMemoryResource::new(context.clone()), initial, maximum)
                .map_err(allocation_error)?,
        );
        Ok(Arc::new(Self {
            memory,
            stream,
            context,
            uploads: Mutex::new(UploadStaging::default()),
        }))
    }
    pub fn resource(&self) -> &MemoryResource {
        &self.memory
    }
    pub fn context(&self) -> &Arc<CudaContext> {
        &self.context
    }
    pub fn stream(&self) -> &Arc<CudaStream> {
        &self.stream
    }
}
impl Drop for Pool {
    fn drop(&mut self) {
        let uploads = self
            .uploads
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for chunk in uploads.chunks.drain(..) {
            let completed = if chunk.tracked {
                chunk.ready.synchronize()
            } else {
                // Submission failed before a completion event was recorded.
                // This teardown-only fallback must cover a possibly live DMA.
                self.stream.synchronize()
            };
            if let Err(error) = completed {
                self.context.record_err::<()>(Err(error));
                // Completion is unknown: freeing the source could race DMA.
                // Retain its context too; only a failed CUDA teardown leaks.
                std::mem::forget(chunk);
            }
        }
    }
}

// Like the descriptor staging arena, append into disjoint pinned ranges.
// Re-recording a chunk's event covers all its uploads on this one stream.
// Events, rather than host callbacks, permit reuse without a callback barrier
// and keep CUDA frees off the driver's callback thread.
#[derive(Default)]
struct UploadStaging {
    chunks: Vec<UploadChunk>,
}
struct UploadChunk {
    host: PinnedHostBuffer<u8>,
    ready: CudaEvent,
    used: usize,
    // False means a submission may be live but is not covered by `ready`.
    tracked: bool,
}
impl UploadStaging {
    fn upload<T: DeviceCopy>(
        &mut self,
        pool: &Pool,
        destination: cuda_core::sys::CUdeviceptr,
        values: &[T],
    ) -> Result<()> {
        let bytes = std::mem::size_of_val(values);
        pool.context().bind_to_thread()?;
        for chunk in &mut self.chunks {
            if !chunk.tracked || bytes > chunk.host.len() {
                continue;
            }
            if bytes > chunk.host.len() - chunk.used {
                if !chunk.ready.query()? {
                    continue;
                }
                chunk.used = 0;
            }
            return chunk.upload(pool.stream(), destination, values);
        }
        // Keep the same 64-KiB minimum as descriptor staging. Growth happens
        // only when no existing chunk has enough unused or completed storage.
        self.chunks.push(UploadChunk {
            host: PinnedHostBuffer::zeroed(pool.context(), bytes.max(64 * 1024))?,
            ready: pool.context().new_event(None)?,
            used: 0,
            tracked: true,
        });
        self.chunks
            .last_mut()
            .unwrap()
            .upload(pool.stream(), destination, values)
    }
}
impl UploadChunk {
    fn upload<T: DeviceCopy>(
        &mut self,
        stream: &CudaStream,
        destination: cuda_core::sys::CUdeviceptr,
        values: &[T],
    ) -> Result<()> {
        let bytes = std::mem::size_of_val(values);
        // Byte copies need no T alignment in the staging arena. No slice into
        // the arena is exposed, including padding copied from DeviceCopy types.
        let source = unsafe { self.host.as_mut_ptr().add(self.used) };
        unsafe {
            std::ptr::copy_nonoverlapping(values.as_ptr().cast::<u8>(), source, bytes);
        }
        // Reserve before submission, even on failure. Never overwrite or free
        // an uncertain DMA source based on an event from an earlier upload.
        self.used += bytes;
        self.tracked = false;
        unsafe {
            cuda_core::memory::memcpy_htod_async(destination, source, bytes, stream.cu_stream())?;
        }
        self.ready.record(stream)?;
        self.tracked = true;
        Ok(())
    }
}

/// An owned device allocation retaining its allocator, context and stream.
/// Deallocation is ordered on that stream; there is no host-runtime lifetime.
/// An immutable range may retain an allocation through `owner` instead of
/// owning its affine token. Range construction is unsafe: the unique writer
/// must never overwrite a published range, even when the backing buffer grows.
pub struct Buffer<T: DeviceCopy> {
    block: Option<DeviceBlock>,
    owner: Option<Arc<Buffer<T>>>,
    offset: usize,
    pool: Arc<Pool>,
    len: usize,
    marker: PhantomData<T>,
}
impl<T: DeviceCopy> Buffer<T> {
    pub fn zeroed(pool: Arc<Pool>, len: usize) -> Result<Self> {
        let buffer = unsafe { Self::uninitialized(pool, len)? };
        if buffer.bytes() != 0 {
            unsafe {
                cuda_core::memory::memset_d8_async(
                    buffer.cu_deviceptr(),
                    0,
                    buffer.bytes(),
                    buffer.pool.stream().cu_stream(),
                )?;
            }
        }
        Ok(buffer)
    }
    /// Allocate pooled storage without clearing it.
    ///
    /// # Safety
    /// Every element must be initialized on this pool's stream before it is
    /// read, downloaded, or exposed to an operation that may read it. Unwritten
    /// capacity may only be passed to operations gated by an initialized count.
    pub unsafe fn uninitialized(pool: Arc<Pool>, len: usize) -> Result<Self> {
        let layout = Layout::array::<T>(len)?;
        if std::mem::size_of::<T>() == 0 {
            return Err("zero-sized device elements".into());
        }
        let block = pool
            .memory
            .allocate(layout, pool.stream())
            .map_err(allocation_error)?;
        Ok(Self {
            block: Some(block),
            owner: None,
            offset: 0,
            pool,
            len,
            marker: PhantomData,
        })
    }
    /// Upload host values and wait for this pool's stream to complete.
    ///
    /// Use [`Self::from_host_async`] when only subsequent stream-ordered work
    /// needs the values; that method also accepts short-lived host slices.
    pub fn from_host(pool: Arc<Pool>, values: &[T]) -> Result<Self> {
        let buffer = unsafe { Self::uninitialized(pool, values.len())? };
        if !values.is_empty() {
            unsafe {
                cuda_core::memory::memcpy_htod_async(
                    buffer.cu_deviceptr(),
                    values.as_ptr(),
                    buffer.bytes(),
                    buffer.pool.stream().cu_stream(),
                )?;
            }
            buffer.pool.stream().synchronize()?;
        }
        Ok(buffer)
    }
    /// Allocate and enqueue an upload without synchronizing the stream.
    ///
    /// Values are copied into pool-owned pinned staging before returning, so
    /// the host slice may immediately be changed or dropped. Reads must be
    /// ordered after this upload on the pool's stream. Dropping the buffer
    /// does not wait for or free its staging; the pool retains and reuses it.
    /// First use or growth of the staging arena may allocate pinned memory.
    pub fn from_host_async(pool: Arc<Pool>, values: &[T]) -> Result<Self> {
        let buffer = unsafe { Self::uninitialized(pool, values.len())? };
        // This new allocation has no published aliases, and upload initializes
        // every element before any subsequent operation on the pool's stream.
        unsafe { buffer.write_async(values)? };
        Ok(buffer)
    }
    /// Enqueue a prefix overwrite, retaining no borrow of the host slice.
    ///
    /// The suffix is unchanged. An empty slice is a no-op; a slice longer than
    /// the buffer is rejected before submission. Reusable pool-owned pinned
    /// staging keeps the copied bytes alive and immutable through the transfer,
    /// including when this buffer is dropped immediately afterwards. No stream
    /// synchronization is performed; arena growth may allocate pinned memory.
    ///
    /// # Safety
    /// The caller must own the right to overwrite this prefix: neither this
    /// buffer nor an alias may publish it as immutable while it is overwritten.
    /// All earlier reads/writes and later consumers of the prefix must be
    /// ordered around this upload on the pool's stream. A shared reference or
    /// an `Arc` alone does not establish exclusive access to device contents.
    pub unsafe fn write_async(&self, values: &[T]) -> Result<()> {
        if values.len() > self.len {
            return Err("host upload exceeds buffer capacity".into());
        }
        if values.is_empty() {
            return Ok(());
        }
        self.pool
            .uploads
            .lock()
            .map_err(|_| "host upload staging lock poisoned")?
            .upload(&self.pool, self.cu_deviceptr(), values)
    }
    /// Retain a range without copying its initialized elements.
    ///
    /// # Safety
    /// All reads must be stream ordered after initialization. Neither the
    /// caller nor any alias may mutate this range while it is published.
    /// Writers may append outside the range. This does not permit slicing
    /// encoded payloads: offsets here are native element offsets only.
    pub unsafe fn shared_slice(owner: Arc<Self>, offset: usize, len: usize) -> Result<Self> {
        if offset.checked_add(len).is_none_or(|end| end > owner.len()) {
            return Err("shared device range exceeds allocation".into());
        }
        Ok(Self {
            pool: owner.pool.clone(),
            block: None,
            owner: Some(owner),
            offset,
            len,
            marker: PhantomData,
        })
    }
    pub fn ptr(&self) -> *mut T {
        self.cu_deviceptr() as *mut T
    }
    pub fn cu_deviceptr(&self) -> cuda_core::sys::CUdeviceptr {
        match &self.owner {
            Some(owner) => owner.cu_deviceptr() + (self.offset * std::mem::size_of::<T>()) as u64,
            None => self.block.as_ref().unwrap().cu_deviceptr(),
        }
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn bytes(&self) -> usize {
        self.len * std::mem::size_of::<T>()
    }
    pub fn num_bytes(&self) -> usize {
        self.bytes()
    }
    pub fn pool(&self) -> &Arc<Pool> {
        &self.pool
    }
    pub fn to_host(&self) -> Result<Vec<T>> {
        self.to_host_prefix(self.len)
    }
    pub fn to_host_vec(&self) -> Result<Vec<T>> {
        self.to_host()
    }
    pub fn to_host_prefix(&self, len: usize) -> Result<Vec<T>> {
        if len > self.len {
            return Err("device download exceeds buffer capacity".into());
        }
        let mut values = Vec::<T>::with_capacity(len);
        if len != 0 {
            unsafe {
                cuda_core::memory::memcpy_dtoh_async(
                    values.as_mut_ptr(),
                    self.cu_deviceptr(),
                    len * std::mem::size_of::<T>(),
                    self.pool.stream().cu_stream(),
                )?;
            }
            self.pool.stream().synchronize()?;
            // The completed transfer initialized exactly this prefix.
            unsafe {
                values.set_len(len);
            }
        }
        Ok(values)
    }
}
impl<T: DeviceCopy> Drop for Buffer<T> {
    fn drop(&mut self) {
        if let Some(block) = self.block.take() {
            // This owner is the unique holder of the affine allocation token.
            // All library uses are ordered on the retained stream.
            if let Err(rccl_memory::MemoryError::Driver(error)) =
                unsafe { self.pool.memory.deallocate(block, self.pool.stream()) }
            {
                self.pool.context().record_err::<()>(Err(error));
            }
        }
    }
}
// SAFETY: the owner retains the pool/context/stream and exposes exactly the
// allocated extent; uninitialized allocation requires initialization before reads.
unsafe impl<T: DeviceCopy> ContiguousDeviceRange<T> for Buffer<T> {
    fn len(&self) -> usize {
        self.len
    }
    fn cu_deviceptr(&self) -> cuda_core::sys::CUdeviceptr {
        self.cu_deviceptr()
    }
    fn context(&self) -> &Arc<CudaContext> {
        self.pool.context()
    }
    fn ensure_usable_on(
        &self,
        stream: &CudaStream,
    ) -> std::result::Result<(), rccl_memory::MemoryError> {
        if self.pool.context().as_ref() != stream.context().as_ref() {
            return Err(rccl_memory::MemoryError::ContextMismatch);
        }
        if stream.cu_stream() != self.pool.stream().cu_stream() {
            return Err(rccl_memory::MemoryError::StreamMismatch);
        }
        Ok(())
    }
}
