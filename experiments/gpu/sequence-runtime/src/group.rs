//! GPU machine instance of segmented grouped monoid folds.
//! Kernel programs are retained per context for the active query scope; each
//! fold binds them to its caller's current stream and allocator.
use crate::{pointer, release, temporary, Auxiliary, Geometry, Result};
use cq_gpu_runtime::{Buffer, Pool};
use gpu_device_columns::OutputCount;
mod keyed;
pub use keyed::{dense_add, DenseFold, DenseFrame};
use std::{cell::RefCell, rc::Rc, sync::Arc};

// CUB borrows the context owned by the last field, after both loaded modules.
struct GroupPrograms {
    module: group_kernels::LoadedModule,
    cub: rccl_cub::cuda::Program<'static>,
    context: Arc<cuda_core::CudaContext>,
}

impl GroupPrograms {
    fn load(pool: &Pool) -> Result<Self> {
        let context = pool.context().clone();
        let module = unsafe { group_kernels::load(&context)? };
        let cub = rccl_cub::cuda::Program::load(&context)?;
        // SAFETY: moving the Arc does not move its context. The CUB program
        // drops before the owning context and never escapes this owner.
        let cub = unsafe {
            std::mem::transmute::<rccl_cub::cuda::Program<'_>, rccl_cub::cuda::Program<'static>>(cub)
        };
        Ok(Self {
            module,
            cub,
            context,
        })
    }
}

thread_local! {
    static GROUP_PROGRAMS: RefCell<Option<Rc<GroupPrograms>>> = const { RefCell::new(None) };
}

fn release_query_programs() {
    GROUP_PROGRAMS.with(|cache| cache.borrow_mut().take());
}

fn programs(pool: &Pool) -> Result<Rc<GroupPrograms>> {
    crate::register_query_cleanup(release_query_programs);
    GROUP_PROGRAMS.with(|cache| {
        if let Some(programs) = cache.borrow().as_ref() {
            if Arc::ptr_eq(&programs.context, pool.context()) {
                return Ok(programs.clone());
            }
        }
        let programs = Rc::new(GroupPrograms::load(pool)?);
        *cache.borrow_mut() = Some(programs.clone());
        Ok(programs)
    })
}

/// A checked aggregate's machine representation. Extrema use a separate validity
/// column, so every i64 bit pattern remains a representable value.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Aggregate {
    pub kind: u32,
    pub min: i64,
    pub max: i64,
}
unsafe impl cuda_core::DeviceCopy for Aggregate {}

/// Recorded scalar representation of an aggregate output. Extrema carry their
/// validity separately; rustc resolves aliases and the optional payload type.
pub trait AggregateRepresentation<const OPTIONAL: bool> {
    const MIN: i128;
    const MAX: i128;
}

impl<T: gpu_datalog_runtime::Scalar> AggregateRepresentation<false> for T {
    const MIN: i128 = T::MIN;
    const MAX: i128 = T::MAX;
}

impl<T: gpu_datalog_runtime::Scalar> AggregateRepresentation<true> for Option<T> {
    const MIN: i128 = T::MIN;
    const MAX: i128 = T::MAX;
}

/// Typed inverse at a nullable stage seam. The machine payload is independent
/// of presence, so Some(i64::MIN) and Some(i64::MAX) need no special cases.
pub trait Nullable: Sized {
    fn from_lanes(value: i64, present: bool) -> Self;
    fn value_lane(&self) -> i64;
    fn present(&self) -> bool;
}
impl<T: gpu_datalog_runtime::Scalar<Logical = T>> Nullable for Option<T> {
    #[inline(always)]
    fn from_lanes(value: i64, present: bool) -> Self {
        if present { Some(T::decode(value as i128)) } else { None }
    }
    #[inline(always)]
    fn value_lane(&self) -> i64 {
        self.as_ref().map_or(0, T::encode)
    }
    #[inline(always)]
    fn present(&self) -> bool {
        self.is_some()
    }
}

#[derive(Clone, Copy)]
pub enum GroupLane {
    Value(usize),
    Presence(usize),
}

pub fn aggregate<T: AggregateRepresentation<OPTIONAL>, const OPTIONAL: bool>(
    kind: u32,
) -> Result<Aggregate> {
    let min = if kind == 0 { 0 } else { T::MIN };
    let max = if kind == 0 {
        T::MAX.min(i64::MAX as i128)
    } else {
        T::MAX
    };
    Ok(Aggregate {
        kind,
        min: i64::try_from(min).map_err(|_| "GPU aggregate representation exceeds i64")?,
        max: i64::try_from(max).map_err(|_| "GPU aggregate representation exceeds i64")?,
    })
}

#[cfg(test)]
mod representation_tests {
    use super::aggregate;

    #[test]
    fn aliases_and_optional_payloads_use_scalar_metadata() {
        type Value = i32;
        type Optional = Option<Value>;
        let sum = aggregate::<Value, false>(1).unwrap();
        let min = aggregate::<Optional, true>(2).unwrap();
        assert_eq!((sum.min, sum.max), (i32::MIN as i64, i32::MAX as i64));
        assert_eq!((min.min, min.max), (sum.min, sum.max));
    }

    #[test]
    fn count_bounds_fit_the_machine_accumulator() {
        let count = aggregate::<usize, false>(0).unwrap();
        assert_eq!(count.min, 0);
        assert_eq!(
            count.max as i128,
            (usize::MAX as i128).min(i64::MAX as i128)
        );
    }

    #[test]
    fn radix_controls_reuse_alternating_stage_sizes() -> crate::Result<()> {
        let pool = crate::Pool::new(0, 0, 0)?;
        let program = rccl_cub::cuda::Program::load(pool.context())?;
        let launch = program.on(pool.stream())?;
        let controls = |rows, wide| -> crate::Result<(u64, u64, u64)> {
            let mut addresses = (0, 0, 0);
            crate::with_radix_workspace(&pool, &launch, rows, wide, |scratch| {
                let (bins, lookback, counters) = scratch.__parts_mut();
                addresses = (bins.cu_deviceptr(), lookback.cu_deviceptr(), counters.cu_deviceptr());
                Ok(())
            })?;
            Ok(addresses)
        };
        controls(10_000, true)?;
        let large = controls(20_000, true)?;
        let range = controls(10_000, false)?;
        assert_ne!(large, range);
        assert_eq!(large, controls(10_000, true)?);
        assert_eq!(large, controls(20_000, true)?);
        assert_eq!(range, controls(10_000, false)?);
        for rows in 30_000..30_016 {
            let grown = controls(rows, true)?;
            assert_eq!(grown, controls(10_000, true)?);
            assert_eq!(range, controls(10_000, false)?);
            assert!(crate::RADIX_WORKSPACES.with(|cache| cache.borrow().len() <= 2));
        }
        crate::release_cached_scratch(&pool);
        assert!(crate::RADIX_WORKSPACES.with(|cache| cache.borrow().is_empty()));
        Ok(())
    }
}

/// Measured grouping frame. Computed values may exceed the u32 domain policy;
/// loaded/domain fields may not. Scratch follows `bits`, not the origin flag.
#[derive(Clone, Copy, Debug)]
pub struct GroupKeyDomain {
    pub minimum: i64,
    pub bound: u128,
    pub bits: u32,
}

pub struct GroupedOutput {
    values: Buffer<i64>,
    valid: Buffer<u8>,
    metadata: Arc<Buffer<OutputCount>>,
    statuses: Vec<Arc<Buffer<OutputCount>>>,
    width: usize,
    key_domains: Vec<GroupKeyDomain>,
}
impl GroupedOutput {
    pub fn key_domains(&self) -> &[GroupKeyDomain] {
        &self.key_domains
    }
    pub fn with_decoder<F>(self, decode: F) -> GroupedRows<F> {
        GroupedRows {
            output: self,
            decode,
        }
    }
    pub fn checked_len(&self) -> Result<usize> {
        for status in &self.statuses {
            if status.to_host()?[0].error != 0 {
                return Err("Sequence grouped input failure".into());
            }
        }
        let metadata = self.metadata.to_host()?[0];
        if metadata.error != 0 {
            return Err("Sequence grouped fold arithmetic failure".into());
        }
        Ok(metadata.rows as usize)
    }
}

pub struct GroupedRows<F> {
    output: GroupedOutput,
    decode: F,
}
impl<F> GroupedRows<F> {
    pub fn checked_len(&self) -> Result<usize> {
        self.output.checked_len()
    }
    /// Publish grouped payload and presence lanes as resident input columns.
    ///
    /// Supplied bounds size the encoder; table bounds are measured logical extrema.
    /// # Safety
    /// `slots` must describe `Row` in physical key/aggregate order, selecting
    /// the presence source for each recorded nullable flag. Group machine
    /// values must already be the row codec's integer representation.
    /// Each supplied bound must enclose that output column's complete domain;
    /// group keys inherit their input bounds, not the aggregate's value domain.
    pub unsafe fn into_columns<Row: gpu_device_columns::RowCodec<'static>>(
        self,
        slots: &[GroupLane],
        bounds: &[gpu_device_columns::Bounds],
    ) -> Result<cq_gpu_runtime::DeviceColumns<'static, Row>> {
        use gpu_device_columns::{
            Codec, ColumnView, DeviceColumns, Dictionary, EncodedColumn, SourceView, Table,
        };
        let output = self.output;
        if Row::WIDTH != slots.len()
            || bounds.len() != slots.len()
            || slots.iter().any(|lane| match *lane {
                GroupLane::Value(i) | GroupLane::Presence(i) => i >= output.width,
            })
        {
            return Err("grouped publication schema width mismatch".into());
        }
        let pool = output.values.pool().clone();
        // fold allocates exactly one row per group; metadata retains the same
        // count plus arithmetic failures. Neither values nor count are read back.
        let rows = output.values.len() / output.width;
        let mut columns = Vec::with_capacity(slots.len());
        for (&lane, &bounds) in slots.iter().zip(bounds) {
            let (data, presence) = match lane {
                GroupLane::Value(i) => (output.values.ptr().wrapping_add(if rows == 0 { 0 } else { i }).cast(), false),
                GroupLane::Presence(i) => (output.valid.ptr().wrapping_add(if rows == 0 { 0 } else { i }).cast(), true),
            };
            let mut source = SourceView::from(ColumnView {
                headers: std::ptr::null(),
                data,
                len: output.values.len() as u64,
                count: std::ptr::null(),
                codec: Codec::Plain as u32,
            });
            source.stride = output.width as u64;
            if presence { source.packed = gpu_device_columns::SOURCE_BYTES; }
            columns.push(unsafe {
                EncodedColumn::from_gather_source_pending::<_, i64>(
                    pool.clone(),
                    source,
                    std::ptr::null(),
                    rows,
                    output.metadata.clone(),
                    0,
                    bounds,
                )?
            });
        }
        let columns = gpu_device_columns::PendingColumn::publish(columns)?;
        let views: Vec<_> = columns
            .iter()
            .map(|column| unsafe { column.view() })
            .collect();
        let measured_bounds = columns.iter().map(|column| column.bounds()).collect();
        let table = Table {
            columns,
            views: Arc::new(Buffer::from_host(pool.clone(), &views)?),
            metadata: output.metadata,
            capacity: rows,
            bounds: measured_bounds,
            statuses: output.statuses,
        };
        Ok(unsafe { DeviceColumns::from_parts(table, Arc::new(Dictionary::default()), pool) })
    }
    // DECODE P: GroupedRows explicitly publishes final logical host aggregate answers.
    pub fn to_host<Row>(&self) -> Result<Vec<Row>>
    where
        F: Fn(&[i64], &[u8]) -> Result<Row>,
    {
        let rows = self.checked_len()?;
        let values = self.output.values.to_host()?;
        let valid = self.output.valid.to_host()?;
        let mut output = Vec::with_capacity(rows);
        for row in 0..rows {
            let range = row * self.output.width..(row + 1) * self.output.width;
            // DECODE P: final grouped answer decoder reconstructs logical keys and nullable aggregates.
            output.push((self.decode)(&values[range.clone()], &valid[range])?);
        }
        Ok(output)
    }
}

/// Formal rule GPU-KeyedGridFold: this recorded provider owns the checked
/// frame/scratch decision. Its printer emits both protocols, not a cost choice.
pub struct SegmentedGroup;

impl SegmentedGroup {
    pub fn dense_frame(rows: usize, bounds: &[gpu_device_columns::Bounds], aggregates: usize, aux: &Auxiliary) -> Result<Option<DenseFrame>> {
        aux.validate()?;
        if rows > u32::MAX as usize { return Err("invalid grouped input extent".into()); }
        if rows == 0 { return Ok(None); }
        Ok(DenseFrame::from_bounds(bounds, aggregates, aux.group_reduce))
    }

    /// Execute the recorded segmented accumulator over row-major arguments.
    ///
    /// # Safety
    /// The input and flags must satisfy [`fold`]'s canonical-valuation,
    /// initialization, device and stream requirements.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn fold(
        pool: Arc<Pool>,
        input: &Buffer<i64>,
        live: Option<&Buffer<u32>>,
        rows: usize,
        domain_keys: &[bool],
        aggregates: &[Aggregate],
        statuses: Vec<Arc<Buffer<OutputCount>>>,
        aux: &Auxiliary,
    ) -> Result<GroupedOutput> {
        unsafe { fold(pool, input, live, rows, domain_keys, aggregates, statuses, aux) }
    }
}

/// Reduce canonical valuation arguments. Each input row contains the key fields
/// followed by one evaluated argument per aggregate (count uses one).
/// Small live domains use per-block histograms; sorted large domains fold
/// row-parallel warp fragments, retaining the stable order of their run heads.
///
/// # Safety
/// The initialized row-major input must be canonical over complete valuations,
/// not merely over aggregate arguments. Optional 0/1 flags and all live rows
/// must be initialized on the same device and ready on `pool`'s stream.
#[allow(clippy::too_many_arguments)]
pub unsafe fn fold(
    pool: Arc<Pool>,
    input: &Buffer<i64>,
    live: Option<&Buffer<u32>>,
    rows: usize,
    domain_keys: &[bool],
    aggregates: &[Aggregate],
    statuses: Vec<Arc<Buffer<OutputCount>>>,
    aux: &Auxiliary,
) -> Result<GroupedOutput> {
    aux.validate()?;
    let keys = domain_keys.len();
    let element = aux.group;
    let width = keys
        .checked_add(aggregates.len())
        .ok_or("group width overflow")?;
    if width == 0
        || rows > u32::MAX as usize
        || input.len() < rows.checked_mul(width).ok_or("group size overflow")?
        || !Arc::ptr_eq(input.pool(), &pool)
        || live.is_some_and(|live| live.len() < rows)
    {
        return Err("invalid grouped input extent or pool".into());
    }
    let stream = pool.stream();
    let programs = programs(&pool)?;
    let module = &programs.module;
    let launch = programs.cub.on(stream)?;
    let mut order = temporary::<u32>(&pool, rows)?;
    let rows = if let Some(live) = live {
        let mut count = temporary::<u64>(&pool, 1)?;
        if rows != 0 {
            let mut workspace = rccl_cub::thrust_integration::compaction_workspace_in(
                pool.resource(), &launch, rows,
            )?;
            rccl_cub::thrust_integration::device_select_flagged(
                &launch,
                rccl_cub::DeviceArithmeticRange::new(0_u32, 1_u32),
                rccl_cub::DeviceSliceRead::new(live.ptr() as *const u32),
                rccl_cub::DeviceSliceWrite::new(order.cu_deviceptr() as *mut u32),
                &mut count, rows, &mut workspace,
            )?;
        }
        let selected = if rows == 0 {
            0
        } else {
            count.to_host_vec()?[0] as usize
        };
        if selected < rows {
            let selected_order = temporary::<u32>(&pool, selected)?;
            if selected != 0 {
                unsafe {
                    cuda_core::simt::memory::memcpy_dtod_async(
                        selected_order.cu_deviceptr(), order.cu_deviceptr(),
                        selected * std::mem::size_of::<u32>(), stream.cu_stream(),
                    )?;
                }
            }
            release(stream, order)?;
            order = selected_order;
        }
        release(stream, count)?;
        selected
    } else {
        rows
    };
    let mut key_domains = if rows == 0 {
        vec![GroupKeyDomain { minimum: 0, bound: 0, bits: 0 }; keys]
    } else {
        Vec::with_capacity(keys)
    };
    if rows != 0 {
        if live.is_none() {
            let shape = module.prepare_identity(element.config(1))?;
            let blocks = element.grid(shape.function(), rows)?;
            module.identity(
                stream,
                &module.prepare_identity(element.config(blocks))?,
                pointer(&order),
                rows as u32,
            )?;
        }
        if keys != 0 {
            let shape = module.prepare_key_bounds(aux.bounds.config(1))?;
            let blocks = aux.bounds.grid(shape.function(), rows)?;
            let partials_per_key = blocks as usize * 16;
            let partials = unsafe { Buffer::<i64>::uninitialized(pool.clone(), partials_per_key * keys)? };
            let grid = blocks.checked_mul(u32::try_from(keys)?).ok_or("group bounds grid overflow")?;
            module.key_bounds(
                stream, &module.prepare_key_bounds(aux.bounds.config(grid))?,
                input.ptr(), pointer(&order), rows as u32, width, blocks, partials.ptr(),
            )?;
            let partials = partials.to_host()?;
            for (key, partials) in partials.chunks_exact(partials_per_key).enumerate() {
                let (min, max) = partials.chunks_exact(2).fold((i64::MAX, i64::MIN), |(min, max), pair| {
                    (min.min(pair[0]), max.max(pair[1]))
                });
                if domain_keys[key] {
                    gpu_device_columns::KeyDomain::from_bounds(
                        key as u32, gpu_device_columns::Bounds::Inclusive { min, max },
                    )?;
                }
                let range = (i128::from(max) - i128::from(min)) as u64;
                key_domains.push(GroupKeyDomain {
                    minimum: min, bound: u128::from(range) + 1,
                    bits: (64 - range.leading_zeros()).max(1),
                });
            }
        }
    }
    if let Some(frame) = DenseFrame::new(&key_domains, aggregates.len(), aux.group_reduce) {
        let shape = module.prepare_dense_rows(aux.group_reduce.config(1))?;
        let blocks = aux.group_reduce.grid(shape.function(), rows)?;
        let dense = DenseFold::new(pool.clone(), frame, blocks, aggregates)?;
        module.dense_rows(
            stream, &module.prepare_dense_rows(aux.group_reduce.config(blocks))?,
            input.ptr(), pointer(&order), rows, width, keys, dense.axes(), dense.specs(),
            dense.bins(), dense.partials(), dense.error(),
        )?;
        release(stream, order)?;
        return unsafe { dense.finish(statuses, aux) };
    }
    if rows != 0 {
        let mut alternate_order = temporary::<u32>(&pool, rows)?;
        let shape = module.prepare_gather_key(element.config(1))?;
        let blocks = element.grid(shape.function(), rows)?;
        let gather_prepared = module.prepare_gather_key(element.config(blocks))?;
        let narrow_rows = if key_domains.iter().any(|domain| domain.bits <= 32) { rows } else { 0 };
        let wide_rows = if key_domains.iter().any(|domain| domain.bits > 32) { rows } else { 0 };
        let mut sort_keys = temporary::<u32>(&pool, narrow_rows)?;
        let mut alternate_keys = temporary::<u32>(&pool, narrow_rows)?;
        let mut wide_keys = temporary::<u64>(&pool, wide_rows)?;
        let mut alternate_wide = temporary::<u64>(&pool, wide_rows)?;
        // Stable least-significant-field passes produce lexicographic group order.
        for key in (0..keys).rev() {
            if key_domains[key].bits > 32 {
                let shape = module.prepare_gather_wide_key(element.config(1))?;
                let blocks = element.grid(shape.function(), rows)?;
                module.gather_wide_key(
                    stream, &module.prepare_gather_wide_key(element.config(blocks))?,
                    input.ptr(), pointer(&order), pointer(&wide_keys), rows as u32,
                    width, key, key_domains[key].minimum,
                )?;
                let mut pairs = rccl_cub::DoubleBuffer::paired(
                    (&mut wide_keys, &mut order), (&mut alternate_wide, &mut alternate_order),
                ).ok_or("group wide radix extents differ")?;
                crate::with_radix_workspace(&pool, &launch, rows, true, |scratch| {
                    rccl_cub::device_radix_sort_pairs_buffer(&launch, &mut pairs, 0..key_domains[key].bits, scratch)?;
                    Ok(())
                })?;
                let swapped = !pairs.current_is_first();
                drop(pairs);
                if swapped {
                    std::mem::swap(&mut order, &mut alternate_order);
                    std::mem::swap(&mut wide_keys, &mut alternate_wide);
                }
                continue;
            }
            module.gather_key(
                stream,
                &gather_prepared,
                input.ptr(),
                pointer(&order),
                pointer(&sort_keys),
                rows as u32,
                width,
                key,
                key_domains[key].minimum,
            )?;
            let mut pairs = rccl_cub::DoubleBuffer::paired(
                (&mut sort_keys, &mut order),
                (&mut alternate_keys, &mut alternate_order),
            )
            .ok_or("group radix extents differ")?;
            crate::with_radix_workspace(&pool, &launch, rows, false, |scratch| {
                rccl_cub::device_radix_sort_pairs_buffer(
                    &launch,
                    &mut pairs,
                    0..key_domains[key].bits,
                    scratch,
                )?;
                Ok(())
            })?;
            let swapped = !pairs.current_is_first();
            drop(pairs);
            if swapped {
                std::mem::swap(&mut order, &mut alternate_order);
                std::mem::swap(&mut sort_keys, &mut alternate_keys);
            }
        }
        for buffer in [sort_keys, alternate_keys] { release(stream, buffer)?; }
        for buffer in [wide_keys, alternate_wide] { release(stream, buffer)?; }
        release(stream, alternate_order)?;
    }
    let result = keyed::segmented(&pool, &programs, input, pointer(&order), rows, key_domains,
        aggregates, statuses, aux)?;
    release(stream, order)?;
    Ok(result)
}

/// Formal rule GPU-GroupGridFold: grid partials of a keyless group. The
/// generated fused kernel writes one typed partial (and extremum presence)
/// per aggregate and block; [`GlobalFold::finish`] combines them on the same
/// stream into the single global row with one combine launch per aggregate,
/// each a single block of the recorded combine geometry, retaining the
/// empty-input identity.
pub struct GlobalFold {
    pool: Arc<Pool>,
    partials: Buffer<i64>,
    present: Buffer<u32>,
    metadata: Arc<Buffer<OutputCount>>,
    blocks: usize,
    aggregates: Vec<Aggregate>,
}

impl GlobalFold {
    pub fn new(pool: Arc<Pool>, blocks: usize, aggregates: &[Aggregate]) -> Result<Self> {
        if blocks == 0 || blocks > u32::MAX as usize || aggregates.is_empty() {
            return Err("invalid global fold grid or aggregate list".into());
        }
        let cells = blocks
            .checked_mul(aggregates.len())
            .ok_or("global fold size overflow")?;
        Ok(Self {
            partials: Buffer::zeroed(pool.clone(), cells)?,
            present: Buffer::zeroed(pool.clone(), cells)?,
            metadata: Arc::new(Buffer::from_host(
                pool.clone(),
                &[OutputCount {
                    rows: 1,
                    error: 0,
                    padding: 0,
                }],
            )?),
            pool,
            blocks,
            aggregates: aggregates.to_vec(),
        })
    }
    pub fn partials(&self) -> *mut i64 {
        self.partials.ptr()
    }
    pub fn present(&self) -> *mut u32 {
        self.present.ptr()
    }
    /// Sticky arithmetic failure word of the published row's metadata.
    pub fn error(&self) -> *mut u32 {
        self.metadata
            .ptr()
            .cast::<u64>()
            .cast::<u32>()
            .wrapping_add(2)
    }
    /// Combine every block's partials into the one global row: one
    /// `global_combine` launch per aggregate, each a single block of the
    /// recorded `combine` geometry.
    ///
    /// # Safety
    /// Exactly `blocks` blocks of the fused partial kernel must have been
    /// launched on this pool's stream, each writing every aggregate's partial.
    pub unsafe fn finish(
        self,
        statuses: Vec<Arc<Buffer<OutputCount>>>,
        combine: Geometry,
    ) -> Result<GroupedOutput> {
        combine.check("group combine", Auxiliary::SUPPORTED_COMBINE)?;
        let width = self.aggregates.len();
        let values = Buffer::<i64>::zeroed(self.pool.clone(), width)?;
        let valid = Buffer::<u8>::zeroed(self.pool.clone(), width)?;
        let programs = programs(&self.pool)?;
        let module = &programs.module;
        let config = combine.config(1);
        for (column, aggregate) in self.aggregates.iter().enumerate() {
            module.global_combine(
                self.pool.stream(),
                &module.prepare_global_combine(config)?,
                self.partials.ptr(),
                self.present.ptr(),
                self.blocks as u32,
                width,
                column,
                aggregate.kind,
                aggregate.min,
                aggregate.max,
                values.ptr(),
                valid.ptr(),
                self.error(),
            )?;
        }
        Ok(GroupedOutput {
            values,
            valid,
            metadata: self.metadata,
            statuses,
            width,
            key_domains: Vec::new(),
        })
    }
}

#[cuda_device::cuda_module]
mod group_kernels {
    use cuda_device::{kernel, launch_bounds, launch_contract};
    /// Only extrema cross to the host; group key rows never widen for sorting.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain=1,block=(256,1,1))]
    pub fn key_bounds(
        input: *const i64, order: *const u32, rows: u32, width: usize,
        blocks_per_key: u32, partials: *mut i64,
    ) {
        let block = cuda_device::thread::blockIdx_x();
        let column = (block / blocks_per_key) as usize;
        let lane = ((block % blocks_per_key) * 256 + cuda_device::thread::threadIdx_x()) as usize;
        let mut row = lane;
        let mut min = i64::MAX;
        let mut max = i64::MIN;
        while row < rows as usize {
            let value = unsafe { *input.add(*order.add(row) as usize * width + column) };
            min = min.min(value);
            max = max.max(value);
            row += blocks_per_key as usize * 256;
        }
        let mut distance = 16;
        while distance != 0 {
            min = min.min(cuda_device::warp::shuffle_xor_u64(min as u64, distance) as i64);
            max = max.max(cuda_device::warp::shuffle_xor_u64(max as u64, distance) as i64);
            distance /= 2;
        }
        if lane % 32 == 0 {
            let slot = column * blocks_per_key as usize * 16 + lane / 32 * 2;
            unsafe {
                *partials.add(slot) = min;
                *partials.add(slot + 1) = max;
            }
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain=1,block=(256,1,1))]
    pub fn identity(order: *mut u32, rows: u32) {
        unsafe {
            let mut i = cuda_device::thread::index_1d().get() as u32;
            while i < rows {
                *order.add(i as usize) = i;
                i += cuda_device::thread::gridDim_x() * 256;
            }
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain=1,block=(256,1,1))]
    pub fn gather_key(
        input: *const i64,
        order: *const u32,
        keys: *mut u32,
        rows: u32,
        width: usize,
        column: usize,
        minimum: i64,
    ) {
        unsafe {
            let mut i = cuda_device::thread::index_1d().get() as u32;
            while i < rows {
                *keys.add(i as usize) = (*input.add(*order.add(i as usize) as usize * width + column))
                    .wrapping_sub(minimum) as u32;
                i += cuda_device::thread::gridDim_x() * 256;
            }
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain=1,block=(256,1,1))]
    pub fn gather_wide_key(
        input: *const i64, order: *const u32, keys: *mut u64,
        rows: u32, width: usize, column: usize, minimum: i64,
    ) {
        unsafe {
            let mut i = cuda_device::thread::index_1d().get() as u32;
            while i < rows {
                *keys.add(i as usize) = (*input.add(*order.add(i as usize) as usize * width + column))
                    .wrapping_sub(minimum) as u64;
                i += cuda_device::thread::gridDim_x() * 256;
            }
        }
    }
    use cuda_device::{thread, warp};
    use crate::group::{Aggregate, dense_add, keyed::{keyed_combine as combine, keyed_identity as neutral}};

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain=1,block=(256,1,1),dynamic_shared_range=(4096,4096))]
    pub fn dense_rows(input: *const i64, order: *const u32, rows: usize, width: usize, keys: usize,
        axes: *const i64, specs: *const Aggregate, bins: usize, partials: *mut i64, error: *mut u32) {
        unsafe {
            let shared = cuda_device::DynamicSharedArray::<i64, 16>::get_raw().cast::<i64>();
            let lane = thread::threadIdx_x() as usize;
            let stride = width - keys + 1;
            let cells = bins * stride;
            let mut i = lane;
            while i < cells {
                let a = i % stride;
                *shared.add(i) = if a == 0 { 0 } else { neutral((*specs.add(a-1)).kind) };
                i += 256;
            }
            thread::sync_threads();
            let mut p = thread::index_1d().get() as usize;
            while p < rows {
                let row = input.add(*order.add(p) as usize * width);
                let mut bin = 0usize;
                let mut k = 0;
                while k < keys {
                    bin = bin * *axes.add(2*k+1) as usize + (*row.add(k)).wrapping_sub(*axes.add(2*k)) as usize;
                    k += 1;
                }
                let base = shared.add(bin * stride);
                dense_add(base, 0, 1, error);
                let mut a = 0;
                while a+1 < stride {
                    dense_add(base.add(a+1), (*specs.add(a)).kind, *row.add(keys+a), error);
                    a += 1;
                }
                p += thread::gridDim_x() as usize * 256;
            }
            thread::sync_threads();
            let mut i = lane;
            while i < cells {
                *partials.add(thread::blockIdx_x() as usize * cells + i) = *shared.add(i);
                i += 256;
            }
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain=1,block=(256,1,1),dynamic_shared_range=(4096,4096))]
    pub fn dense_combine(partials: *const i64, specs: *const Aggregate, blocks: usize,
        bins: usize, stride: usize, sums: *mut i64, occupied: *mut u32, error: *mut u32) {
        unsafe {
            let cell = thread::blockIdx_x() as usize;
            let lane = thread::threadIdx_x() as usize;
            let a = cell % stride;
            let kind = if a == 0 { 0 } else { (*specs.add(a-1)).kind };
            let mut v = neutral(kind);
            let mut b = lane;
            while b < blocks {
                v = combine(kind, v, *partials.add(b * bins * stride + cell), error);
                b += 256;
            }
            let mut delta = 16;
            while delta != 0 {
                let other = warp::shuffle_down_u64(v as u64, delta) as i64;
                if lane % 32 < delta as usize { v = combine(kind, v, other, error); }
                delta /= 2;
            }
            let shared = cuda_device::DynamicSharedArray::<i64,16>::get_raw().cast::<i64>();
            if lane % 32 == 0 { *shared.add(lane/32) = v; }
            thread::sync_threads();
            if lane == 0 {
                let mut total = *shared;
                let mut w = 1;
                while w < 8 { total = combine(kind, total, *shared.add(w), error); w += 1; }
                *sums.add(cell) = total;
                if a == 0 && total != 0 {
                    use cuda_device::atomic::{DeviceAtomicU32, AtomicOrdering};
                    let bin = cell / stride;
                    DeviceAtomicU32::from_ptr(occupied.add(bin/32)).fetch_or(1 << (bin%32), AtomicOrdering::Relaxed);
                }
            }
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain=1,block=(256,1,1))]
    pub fn dense_publish(sums: *const i64, occupied: *const u32, axes: *const i64, bins: usize,
        keys: usize, width: usize, output: *mut i64, valid: *mut u8, metadata: *mut u64,
        groups: u64, specs: *const Aggregate, error: *mut u32) {
        unsafe {
            let bin = thread::index_1d().get() as usize;
            if bin == 0 { *metadata = groups; }
            if bin >= bins { return; }
            let word = bin / 32;
            let bit = 1u32 << (bin % 32);
            let bits = *occupied.add(word);
            if bits & bit == 0 { return; }
            let mut group = (bits & (bit-1)).count_ones();
            let mut w = 0;
            while w < word { group += (*occupied.add(w)).count_ones(); w += 1; }
            let row = group as usize * width;
            let mut id = bin;
            let mut k = keys;
            while k != 0 {
                k -= 1;
                let radix = *axes.add(k*2+1) as usize;
                *output.add(row+k) = *axes.add(k*2) + (id % radix) as i64;
                *valid.add(row+k) = 1;
                id /= radix;
            }
            let stride = width - keys + 1;
            let mut a = 0;
            while a+1 < stride {
                let v = *sums.add(bin*stride+1+a);
                let spec = *specs.add(a);
                if v < spec.min || v > spec.max { crate::device::flag_overflow(error); }
                *output.add(row+keys+a) = v;
                *valid.add(row+keys+a) = 1;
                a += 1;
            }
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain=1,block=(256,1,1))]
    pub fn run_heads(input: *const i64, order: *const u32, rows: usize, width: usize,
        keys: usize, masks: *mut u32, counts: *mut u32) {
        unsafe {
            let lane = warp::lane_id() as usize;
            let mut p = thread::index_1d().get() as usize;
            while p - lane < rows {
                let mut head = p == 0;
                if p != 0 && p < rows {
                    let a = *order.add(p) as usize * width;
                    let b = *order.add(p-1) as usize * width;
                    let mut k = 0;
                    while k < keys && !head { head = *input.add(a+k) != *input.add(b+k); k += 1; }
                }
                let mask = warp::ballot(head);
                if lane == 0 { *masks.add(p/32) = mask; *counts.add(p/32) = mask.count_ones(); }
                p += thread::gridDim_x() as usize * 256;
            }
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain=1,block=(256,1,1))]
    pub fn run_fragments(input: *const i64, order: *const u32, masks: *const u32,
        ends: *const u32, rows: usize, keys: usize, width: usize, specs: *const Aggregate,
        output: *mut i64, valid: *mut u8, carries: *mut i64, error: *mut u32) {
        unsafe {
            let lane = warp::lane_id();
            let mut p = thread::index_1d().get() as usize;
            while p - (lane as usize) < rows {
                let warp = p/32;
                let mask = *masks.add(warp);
                let head = mask & (1 << lane) != 0;
                let following = mask & ((u32::MAX - 1) << lane);
                let group = (*ends.add(warp) - following.count_ones() - 1) as usize;
                let row = if p < rows { *order.add(p) as usize * width } else { 0 };
                if head {
                    let mut k = 0;
                    while k < keys {
                        *output.add(group*width+k) = *input.add(row+k);
                        *valid.add(group*width+k) = 1;
                        k += 1;
                    }
                }
                let local = lane - (31 - (((mask | 1) & !following).leading_zeros()));
                let len = following.trailing_zeros().min((rows - warp*32).min(32) as u32);
                let mut a = 0;
                while a < width-keys {
                    let kind = (*specs.add(a)).kind;
                    let mut v = if p < rows { *input.add(row+keys+a) } else { neutral(kind) };
                    let mut step = 1;
                    while step < 32 {
                        let next = warp::shuffle_down_u64(v as u64, step) as i64;
                        if local & (2*step-1) == 0 && lane+step < len {
                            v = combine(kind, v, next, error);
                        }
                        step *= 2;
                    }
                    if head { *output.add(group*width+keys+a) = v; *valid.add(group*width+keys+a) = 1; }
                    else if lane == 0 { *carries.add(warp*(width-keys)+a) = v; }
                    a += 1;
                }
                p += thread::gridDim_x() as usize * 256;
            }
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain=1,block=(256,1,1))]
    pub fn run_carries(masks: *const u32, ends: *const u32, carries: *const i64, warps: usize,
        keys: usize, width: usize, specs: *const Aggregate, output: *mut i64, error: *mut u32) {
        use cuda_device::atomic::{DeviceAtomicI64, AtomicOrdering};
        unsafe {
            let mut w = thread::index_1d().get() as usize;
            while w < warps {
                if *masks.add(w) & 1 == 0 {
                    let group = (*ends.add(w-1) - 1) as usize;
                    let mut a = 0;
                    while a < width-keys {
                        let v = *carries.add(w*(width-keys)+a);
                        let cell = DeviceAtomicI64::from_ptr(output.add(group*width+keys+a));
                        match (*specs.add(a)).kind {
                            2 => { cell.fetch_min(v, AtomicOrdering::Relaxed); }
                            3 => { cell.fetch_max(v, AtomicOrdering::Relaxed); }
                            _ => { if cell.fetch_add(v, AtomicOrdering::Relaxed).checked_add(v).is_none() {
                                crate::device::flag_overflow(error);
                            } }
                        }
                        a += 1;
                    }
                }
                w += thread::gridDim_x() as usize * 256;
            }
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain=1,block=(256,1,1))]
    pub fn run_check(groups: usize, keys: usize, width: usize, specs: *const Aggregate,
        output: *const i64, error: *mut u32) {
        unsafe {
            let mut g = thread::index_1d().get() as usize;
            while g < groups {
                let mut a = 0;
                while a < width-keys {
                    let spec = *specs.add(a);
                    let v = *output.add(g*width+keys+a);
                    if v < spec.min || v > spec.max { crate::device::flag_overflow(error); }
                    a += 1;
                }
                g += thread::gridDim_x() as usize * 256;
            }
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain=1,block=(256,1,1),dynamic_shared_range=(3072,3072))]
    pub fn global_combine(
        partials: *const i64,
        present: *const u32,
        blocks: u32,
        width: usize,
        column: usize,
        kind: u32,
        min: i64,
        max: i64,
        output: *mut i64,
        valid: *mut u8,
        error: *mut u32,
    ) {
        unsafe {
            let lane = cuda_device::thread::threadIdx_x() as usize;
            let mut value = if kind == 2 {
                i64::MAX
            } else if kind == 3 {
                i64::MIN
            } else {
                0
            };
            let mut seen = false;
            let mut block = lane;
            while block < blocks as usize {
                let next = *partials.add(block * width + column);
                let next_seen = *present.add(block * width + column) != 0;
                if kind == 2 {
                    if next_seen && (!seen || next < value) {
                        value = next;
                    }
                    seen |= next_seen;
                } else if kind == 3 {
                    if next_seen && (!seen || next > value) {
                        value = next;
                    }
                    seen |= next_seen;
                } else {
                    match value.checked_add(next) {
                        Some(sum) => value = sum,
                        None => crate::device::flag_overflow(error),
                    }
                }
                block += 256;
            }
            let shared = cuda_device::DynamicSharedArray::<u8, 16>::get_raw();
            let values = shared.cast::<i64>();
            let flags = shared.add(2048).cast::<u32>();
            *values.add(lane) = value;
            *flags.add(lane) = seen as u32;
            cuda_device::thread::sync_threads();
            let mut stride = 128;
            while stride != 0 {
                if lane < stride {
                    let a = *values.add(lane);
                    let b = *values.add(lane + stride);
                    let a_seen = *flags.add(lane) != 0;
                    let b_seen = *flags.add(lane + stride) != 0;
                    if kind == 2 {
                        if b_seen && (!a_seen || b < a) {
                            *values.add(lane) = b;
                        }
                        *flags.add(lane) = (a_seen || b_seen) as u32;
                    } else if kind == 3 {
                        if b_seen && (!a_seen || b > a) {
                            *values.add(lane) = b;
                        }
                        *flags.add(lane) = (a_seen || b_seen) as u32;
                    } else {
                        match a.checked_add(b) {
                            Some(sum) => *values.add(lane) = sum,
                            None => crate::device::flag_overflow(error),
                        }
                    }
                }
                cuda_device::thread::sync_threads();
                stride /= 2;
            }
            if lane == 0 {
                let value = *values;
                let present = kind < 2 || *flags != 0;
                if present && (value < min || value > max) {
                    crate::device::flag_overflow(error);
                }
                *output.add(column) = if present { value } else { 0 };
                *valid.add(column) = present as u8;
            }
        }
    }
}

#[cfg(test)]
mod publication_tests {
    use super::*;

    #[test]
    fn grouped_program_cache_tracks_context_and_query_lifetime() -> Result<()> {
        let first_pool = Pool::new(0, 0, 0)?;
        let second_pool = Pool::new(0, 0, 0)?;
        assert!(!Arc::ptr_eq(first_pool.context(), second_pool.context()));
        let retained = {
            let _scope = crate::QueryScope::new();
            let first = programs(&first_pool)?;
            assert!(Rc::ptr_eq(&first, &programs(&first_pool)?));
            let second = programs(&second_pool)?;
            assert!(!Rc::ptr_eq(&first, &second));
            assert!(Rc::ptr_eq(&second, &programs(&second_pool)?));
            for (programs, pool) in [(&first, &first_pool), (&second, &second_pool)] {
                programs.cub.on(pool.stream())?;
            }
            // Replacing the cached context must not invalidate an active owner.
            first.cub.on(first_pool.stream())?;
            let replacement = programs(&first_pool)?;
            assert!(!Rc::ptr_eq(&first, &replacement));
            Rc::downgrade(&replacement)
        };
        assert!(retained.upgrade().is_none());
        GROUP_PROGRAMS.with(|cache| assert!(cache.borrow().is_none()));
        {
            let _scope = crate::QueryScope::new();
            let reloaded = programs(&first_pool)?;
            assert!(Rc::ptr_eq(&reloaded, &programs(&first_pool)?));
        }
        GROUP_PROGRAMS.with(|cache| assert!(cache.borrow().is_none()));
        Ok(())
    }

    #[test]
    fn global_fold_combines_typed_partials_and_keeps_the_empty_identity() -> Result<()> {
        let pool = Pool::new(0, 0, 0)?;
        let count = Aggregate {
            kind: 0,
            min: 0,
            max: i64::MAX,
        };
        let sum = Aggregate {
            kind: 1,
            min: i64::MIN,
            max: i64::MAX,
        };
        let min = Aggregate {
            kind: 2,
            min: i64::MIN,
            max: i64::MAX,
        };
        let mut fold = GlobalFold::new(pool.clone(), 3, &[count, sum, min])?;
        // Block-major partials; only the last block saw an extremum.
        fold.partials = Buffer::from_host(pool.clone(), &[2i64, 10, 0, 0, 0, 0, 1, -4, 7])?;
        fold.present = Buffer::from_host(pool.clone(), &[0u32, 0, 0, 0, 0, 0, 0, 0, 1])?;
        let combine = Auxiliary::SUPPORTED_COMBINE;
        let rows = unsafe { fold.finish(Vec::new(), combine)? }.with_decoder(
            |values: &[i64], valid: &[u8]| -> Result<_> { Ok((values.to_vec(), valid.to_vec())) },
        );
        assert_eq!(rows.to_host()?, vec![(vec![3, 6, 7], vec![1, 1, 1])]);
        let empty = GlobalFold::new(pool.clone(), 1, &[count, min])?;
        let rows = unsafe { empty.finish(Vec::new(), combine)? }.with_decoder(
            |values: &[i64], valid: &[u8]| -> Result<_> { Ok((values.to_vec(), valid.to_vec())) },
        );
        assert_eq!(rows.to_host()?, vec![(vec![0, 0], vec![1, 0])]);
        let bounds = [
            gpu_device_columns::Bounds::Inclusive { min: i64::MIN, max: i64::MAX },
            gpu_device_columns::Bounds::Inclusive { min: 0, max: 1 },
        ];
        let slots = [GroupLane::Value(1), GroupLane::Presence(1)];
        let published = unsafe { rows.into_columns::<(i64, bool)>(&slots, &bounds)? };
        assert_eq!(published.to_host()?, [(0, false)]);
        for value in [i64::MIN, i64::MAX] {
            let mut fold = GlobalFold::new(pool.clone(), 1, &[count, min])?;
            fold.partials = Buffer::from_host(pool.clone(), &[1, value])?;
            fold.present = Buffer::from_host(pool.clone(), &[1, 1])?;
            let output = unsafe { fold.finish(Vec::new(), combine)? }.with_decoder(());
            let published = unsafe { output.into_columns::<(i64, bool)>(&slots, &bounds)? };
            assert_eq!(published.to_host()?, [(value, true)]);
        }
        let mut overflow = GlobalFold::new(pool.clone(), 2, &[sum])?;
        overflow.partials = Buffer::from_host(pool.clone(), &[i64::MAX, 1])?;
        let rows = unsafe { overflow.finish(Vec::new(), combine)? }
            .with_decoder(|values: &[i64], _: &[u8]| -> Result<_> { Ok(values[0]) });
        assert!(rows.to_host().is_err());
        // The kernel's static shared layout supports only the recorded table value.
        let narrow = Geometry {
            block_threads: 128,
            ..combine
        };
        let rejected = GlobalFold::new(pool.clone(), 1, &[count])?;
        assert!(unsafe { rejected.finish(Vec::new(), narrow) }.is_err());
        Ok(())
    }

    #[test]
    fn group_keys_record_narrow_ranges_and_refuse_wide_spans() -> Result<()> {
        let pool = Pool::new(0, 0, 0)?;
        let aggregate = Aggregate { kind: 1, min: i64::MIN, max: i64::MAX };
        let input = Buffer::from_host(pool.clone(), &[
            i64::MIN + 1, 1i64 << 40, i64::MIN, 7, i64::MIN + 1, 9,
        ])?;
        let grouped = unsafe {
            fold(pool.clone(), &input, None, 3, &[true], &[aggregate], Vec::new(), &Auxiliary::SUPPORTED)?
        };
        assert_eq!((grouped.key_domains()[0].minimum, grouped.key_domains()[0].bits), (i64::MIN, 1));
        let rows = grouped.with_decoder(|values: &[i64], _: &[u8]| -> Result<_> {
            Ok((values[0], values[1]))
        }).to_host()?;
        assert_eq!(rows, [(i64::MIN, 7), (i64::MIN + 1, (1i64 << 40) + 9)]);
        let computed_narrow = unsafe {
            fold(pool.clone(), &input, None, 3, &[false], &[aggregate], Vec::new(), &Auxiliary::SUPPORTED)?
        };
        assert_eq!(computed_narrow.key_domains()[0].bits, 1);
        let wide = Buffer::from_host(pool.clone(), &[i64::MIN, 1, i64::MAX, 2])?;
        let error = unsafe {
            fold(pool.clone(), &wide, None, 2, &[true], &[aggregate], Vec::new(), &Auxiliary::SUPPORTED)
        }.err().expect("wide key range must be refused");
        assert!(error.to_string().contains("u32 code domain"));
        let computed_wide = unsafe {
            fold(pool, &wide, None, 2, &[false], &[aggregate], Vec::new(), &Auxiliary::SUPPORTED)?
        };
        assert_eq!(computed_wide.key_domains()[0].bits, 64);
        assert_eq!(computed_wide.with_decoder(|values: &[i64], _: &[u8]| -> Result<_> {
            Ok((values[0], values[1]))
        }).to_host()?, [(i64::MIN, 1), (i64::MAX, 2)]);
        Ok(())
    }

    #[test]
    fn resident_group_publication_preserves_values_count_and_failure() -> Result<()> {
        let pool = Pool::new(0, 0, 0)?;
        let input = Buffer::from_host(pool.clone(), &[1i64, 3, 1, 7, 2, 5])?;
        let aggregate = Aggregate {
            kind: 1,
            min: i64::MIN,
            max: i64::MAX,
        };
        for error in [0, 1] {
            let status = Arc::new(Buffer::from_host(
                pool.clone(),
                &[OutputCount {
                    rows: 3,
                    error,
                    padding: 0,
                }],
            )?);
            let grouped = unsafe {
                fold(
                    pool.clone(),
                    &input,
                    None,
                    3,
                    &[true],
                    &[aggregate],
                    vec![status],
                    &Auxiliary::SUPPORTED,
                )?
            };
            let published = unsafe {
                grouped.with_decoder(()).into_columns::<(i64, i64)>(
                    &[GroupLane::Value(1), GroupLane::Value(0)],
                    &[
                        gpu_device_columns::Bounds::Inclusive {
                            min: i64::MIN,
                            max: i64::MAX,
                        },
                        gpu_device_columns::Bounds::Inclusive { min: 1, max: 2 },
                    ],
                )?
            };
            assert_eq!(published.capacity(), 2);
            assert_eq!(
                published.storage().bounds[1],
                gpu_device_columns::Bounds::Inclusive { min: 1, max: 2 }
            );
            if error == 0 {
                // DECODE P: diagnostic resident grouped projection exposes reconstructed host rows.
                assert_eq!(published.to_host()?, [(10, 1), (5, 2)]);
            } else {
                // DECODE P: diagnostic grouped publication checks retained device-error rejection.
                assert!(published.to_host().is_err());
            }
        }
        Ok(())
    }
}
