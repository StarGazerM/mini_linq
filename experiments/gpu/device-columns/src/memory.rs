use crate::Result;
use cuda_core::{CudaContext, CudaStream, DeviceCopy};
use rccl_memory::{
    ContiguousDeviceRange, CudaMemoryResource, DeviceAllocator, DeviceBlock, PoolMemoryResource,
    StatisticsMemoryResource,
};
use std::{alloc::Layout, marker::PhantomData, sync::Arc};

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

/// Shared resource and stream ownership, not a cache of relation data.
pub struct Pool {
    memory: MemoryResource,
    stream: Arc<CudaStream>,
    context: Arc<CudaContext>,
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
        }))
    }
    pub fn resource(&self) -> &MemoryResource {
        &self.memory
    }
    pub fn context(&self) -> &Arc<CudaContext> {
        &self.context
    }
    pub fn stream(&self) -> &CudaStream {
        &self.stream
    }
}

/// An owned device allocation retaining its allocator, context and stream.
/// Deallocation is ordered on that stream; there is no host-runtime lifetime.
pub struct Buffer<T: DeviceCopy> {
    block: Option<DeviceBlock>,
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
            pool,
            len,
            marker: PhantomData,
        })
    }
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
    pub fn ptr(&self) -> *mut T {
        self.cu_deviceptr() as *mut T
    }
    pub fn cu_deviceptr(&self) -> cuda_core::sys::CUdeviceptr {
        self.block.as_ref().unwrap().cu_deviceptr()
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
