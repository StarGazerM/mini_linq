//! Shared resident columns and measurements for the GPU Sequence target.
//!
//! The runtime exposes the pooled storage and codec APIs consumed by Sequence.

pub mod expansions;
pub mod nvtx;

pub use gpu_device_columns::{
    decode_stripe, decode_tile_pair, decode_value, detect_integer_codec, encode_value, Bounds,
    Buffer, Codec, ColumnView, DecodeValue, Delta, DeviceColumns, Dictionary, DictionaryCodec,
    EncodeValue, EncodedColumn, For, OwnedRowCodec, Plain, Pool, Representation, RowCodec,
};

pub type Error = Box<dyn std::error::Error>;
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Formal rule GPU-LAUNCH-OCCUPANCY:
/// Size a grid-stride launch from the concrete function's CUDA occupancy query,
/// including its register and static shared-memory requirements. `shared` is
/// dynamic shared memory only. Prepare a one-block launch to obtain the function,
/// then prepare the final launch with this grid. No kernel is launched by either
/// preparation. Empty-safe kernels receive one block for a zero-tile domain.
pub fn launch_grid_for_kernel(
    function: &cuda_core::CudaFunction,
    parts: usize,
    threads: u32,
    shared: u32,
) -> Result<u32> {
    let blocks = function.max_active_blocks_per_multiprocessor(threads, shared)?;
    let resident = u64::from(function.context().multiprocessor_count()?) * u64::from(blocks);
    if resident == 0 {
        return Err("launch shape has no resident CUDA blocks".into());
    }
    Ok((parts.max(1) as u64).min(resident).min(u64::from(u32::MAX)) as u32)
}

/// GPU-ProgramLaunch: a validated one-dimensional launch envelope, owned by
/// one compiled CUDA entry point and context. Its argument ABI remains the
/// generated wrapper's obligation, not a runtime type-inspection decision.
#[derive(Debug)]
pub struct ProgramLaunch {
    function: cuda_core::CudaFunction,
    config: cuda_core::simt::LaunchConfig,
}

impl ProgramLaunch {
    /// Retain a typed preparation without retaining its Rust callable type.
    /// Only ordinary 1-D kernels permit the downward grid refinement below.
    pub fn from_prepared<C: cuda_core::KernelLaunchContract>(
        prepared: &cuda_core::PreparedLaunch<C>,
    ) -> Result<Self> {
        let config = prepared.__raw_config();
        if C::SPEC.cooperative() || C::SPEC.cluster().is_some()
            || config.grid_dim.1 != 1 || config.grid_dim.2 != 1
            || config.block_dim.1 != 1 || config.block_dim.2 != 1
        {
            return Err("program launch requires an ordinary one-dimensional kernel".into());
        }
        Ok(Self { function: prepared.function().clone(), config })
    }

    pub fn resident_blocks(&self) -> u32 {
        self.config.grid_dim.0
    }

    /// Launch within the already validated residency envelope; no CUDA query,
    /// host allocation, synchronization or handle lookup occurs here.
    ///
    /// # Safety
    /// `arguments` must be the exact scalar ABI of this compiled specialization.
    /// Its generated grid-stride kernel must allow every positive grid up to
    /// `resident_blocks()`, and its pointer owners must survive stream use.
    pub unsafe fn launch(
        &self,
        stream: &cuda_core::CudaStream,
        blocks: u32,
        arguments: &mut [*mut std::ffi::c_void],
    ) -> Result<()> {
        if blocks == 0 || blocks > self.resident_blocks() {
            return Err("program launch grid exceeds its validated envelope".into());
        }
        if self.function.context().cu_ctx() != stream.context().cu_ctx() {
            return Err("program launch belongs to another CUDA context".into());
        }
        unsafe {
            cuda_core::launch_kernel_on_stream(
                &self.function, (blocks, 1, 1), self.config.block_dim,
                self.config.shared_mem_bytes, stream, arguments,
            )?;
        }
        Ok(())
    }
}

/// A program-owned slot keyed only by the CUDA module's opaque compiled entry
/// name. Different callable specializations never share a preparation.
#[derive(Debug, Default)]
pub struct ProgramLaunchCache(Vec<(&'static str, ProgramLaunch)>);

impl ProgramLaunchCache {
    pub fn get_or_prepare(
        &mut self,
        entry: &'static str,
        prepare: impl FnOnce() -> Result<ProgramLaunch>,
    ) -> Result<&ProgramLaunch> {
        if let Some(index) = self.0.iter().position(|(name, _)| *name == entry) {
            return Ok(&self.0[index].1);
        }
        let prepared = prepare()?;
        self.0.push((entry, prepared));
        Ok(&self.0.last().expect("inserted program launch").1)
    }
}

/// GPU-RootSpread realization: the root tile of a recorded `Spread` partition,
/// `clamp(ceil(domain / resident), 1, max_rows)` rows, so a small driver fills
/// the `resident` blocks returned by [`launch_grid_for_kernel`] instead of one
/// block per declared tile. A large driver keeps the declared tile.
pub fn spread_tile(domain: u32, resident: u32, max_rows: usize) -> u32 {
    let rows = (domain as usize).div_ceil(resident.max(1) as usize);
    rows.clamp(1, max_rows.clamp(1, u32::MAX as usize)) as u32
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Timings {
    pub upload_ms: f64,
    pub build_ms: f64,
    pub kernel_ms: f64,
    pub dedup_ms: f64,
    pub download_ms: f64,
    pub total_ms: f64,
    pub wall_ms: f64,
}

thread_local! {
    static LAST_TIMINGS: std::cell::Cell<Option<Timings>> = const { std::cell::Cell::new(None) };
    static ARENA_TIMING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn last_run_timings() -> Option<Timings> {
    LAST_TIMINGS.with(std::cell::Cell::get)
}

pub fn take_last_timings() -> Option<Timings> {
    LAST_TIMINGS.with(std::cell::Cell::take)
}

/// Enable synchronizing phase measurements for arena invocations on this thread.
/// Ordinary Sequence outputs retain their existing timing behavior.
pub fn enable_arena_timing(enabled: bool) {
    ARENA_TIMING.with(|timing| timing.set(enabled));
    LAST_TIMINGS.with(|last| last.set(None));
}

pub fn arena_timing_enabled() -> bool {
    ARENA_TIMING.with(std::cell::Cell::get)
}

/// Publish phase measurements from a checked external target interpreter.
pub fn publish_timings(timings: Timings) {
    LAST_TIMINGS.with(|last| last.set(Some(timings)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cuda_device::cuda_module]
    mod kernels {
        use cuda_device::{kernel, launch_bounds, launch_contract};

        #[kernel]
        #[launch_bounds(32)]
        #[launch_contract(domain = 1, block = (32, 1, 1), dynamic_shared_range = (0, 0))]
        pub fn map<F: Copy + Fn(u32) -> u32>(function: F, output: *mut u32, len: u32) {
            let mut index = cuda_device::thread::blockIdx_x() * 32
                + cuda_device::thread::threadIdx_x();
            let stride = cuda_device::thread::gridDim_x() * 32;
            while index < len {
                unsafe { *output.add(index as usize) = function(index); }
                index += stride;
            }
        }
    }

    fn execute<F: Copy + Fn(u32) -> u32>(
        pool: &std::sync::Arc<Pool>,
        module: &kernels::LoadedModule,
        cache: &mut ProgramLaunchCache,
        preparations: &mut usize,
        function: F,
        blocks: u32,
    ) -> Result<Vec<u32>> {
        let entry = kernels::map_ptx_name::<F>();
        let prepared = cache.get_or_prepare(entry, || {
            *preparations += 1;
            ProgramLaunch::from_prepared(
                &module.prepare_map_for(&function, cuda_core::LaunchConfig1D::new(4,32,0))?,
            )
        })?;
        let output = Buffer::<u32>::zeroed(pool.clone(), 257)?;
        let mut function = function;
        let mut pointer = output.ptr();
        let mut len = 257u32;
        let mut arguments = cuda_host::KernelArgs::<3>::new();
        cuda_host::push_kernel_scalar(&mut arguments, &mut function);
        cuda_host::push_kernel_scalar(&mut arguments, &mut pointer);
        cuda_host::push_kernel_scalar(&mut arguments, &mut len);
        // Same compiled entry, exact scalar ABI, and retained output owner.
        unsafe { prepared.launch(pool.stream(), blocks, arguments.as_mut_slice())?; }
        output.to_host()
    }

    #[test]
    fn program_launch_reuses_specializations_and_checks_the_grid_envelope() -> Result<()> {
        let pool = Pool::new(0,0,0)?;
        let module = unsafe { kernels::load(pool.context())? };
        let mut cache = ProgramLaunchCache::default();
        let mut preparations = 0;
        let first = |x: u32| x + 7;
        let second = |x: u32| x * 3;
        for blocks in [1,4,2] {
            assert_eq!(execute(&pool,&module,&mut cache,&mut preparations,first,blocks)?,
                (0..257).map(first).collect::<Vec<_>>());
        }
        assert_eq!(preparations,1);
        assert_eq!(execute(&pool,&module,&mut cache,&mut preparations,second,3)?,
            (0..257).map(second).collect::<Vec<_>>());
        assert_eq!(preparations,2);
        assert_eq!(execute(&pool,&module,&mut cache,&mut preparations,first,1)?,
            (0..257).map(first).collect::<Vec<_>>());
        assert_eq!(preparations,2);
        assert!(execute(&pool,&module,&mut cache,&mut preparations,first,0).is_err());
        assert!(execute(&pool,&module,&mut cache,&mut preparations,first,5).is_err());
        assert_eq!(preparations,2);
        Ok(())
    }
}
