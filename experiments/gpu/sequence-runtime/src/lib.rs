//! Sequence execution primitives over resident columns. Callers supply the
//! recorded key order, selection and payload; this crate makes no plan choices.
pub mod canon;
pub mod device;
mod kernels;
pub mod selected;
pub use selected::{select, SelectedRows};

pub use cq_gpu_runtime::{Bounds, Buffer, Codec, ColumnView, DeviceColumns, EncodedColumn, Pool};
use cuda_core::{CudaContext, CudaStream, DeviceBuffer, DeviceCopy, LaunchConfig1D};
use gpu_compress::{Catalog, Domain, Kind};
pub use gpu_device_columns::MemoryResource;
use rccl_cub::DoubleBuffer;
use rccl_thrust::prelude::*;
use rccl_thrust::{
    arithmetic_range, gather, lower_bound, select_flagged, sort_columns_lexicographic,
};
use std::{cell::RefCell, sync::Arc};

pub type Result<T, E = Box<dyn std::error::Error>> = std::result::Result<T, E>;

/// Host codec for the catalog's shared, order-preserving dense ID domain.
/// Missing constants stay absent rather than aliasing any stored ID.
#[derive(Clone, Copy)]
pub struct DenseDictionary<'a> {
    domain: &'a Domain,
}
impl<'a> DenseDictionary<'a> {
    pub fn new(domain: &'a Domain) -> Self {
        Self { domain }
    }
    pub fn from_catalog(catalog: &'a Catalog, name: &str) -> Result<Self> {
        Ok(Self::new(catalog.domain(name)?))
    }
    pub fn domain(&self) -> &'a Domain {
        self.domain
    }
    pub fn encode(&self, value: i64) -> Option<u32> {
        self.domain.id(value)
    }
    pub fn decode(&self, id: u32) -> Option<i64> {
        ((id as usize) < self.len()).then(|| self.domain.value(id))
    }
    pub fn len(&self) -> usize {
        self.domain.len()
    }
    pub fn is_empty(&self) -> bool {
        self.domain.is_empty()
    }
    pub fn bits(&self) -> u32 {
        self.domain.bits()
    }
    pub fn bounds(&self) -> Bounds {
        self.domain.bounds()
    }
}

/// An owning source column and its known live row prefix.
#[derive(Clone)]
pub struct ColumnDescriptor {
    column: Arc<EncodedColumn>,
    rows: usize,
}
impl ColumnDescriptor {
    pub fn from_catalog(catalog: &Catalog, table: &str, column: &str) -> Result<Self> {
        Ok(Self {
            column: catalog.column(table, column)?.column.clone(),
            rows: catalog.rows(table)?,
        })
    }
    /// Describe an explicit resident column without reading its device count.
    ///
    /// # Safety
    /// `rows` must be the initialized live prefix, ready on the column's pool
    /// stream. Its count must not change during the build.
    pub unsafe fn new(column: Arc<EncodedColumn>, rows: usize) -> Result<Self> {
        if rows > column.capacity() {
            return Err("source rows exceed column capacity".into());
        }
        Ok(Self { column, rows })
    }
    pub fn column(&self) -> &Arc<EncodedColumn> {
        &self.column
    }
    pub fn rows(&self) -> usize {
        self.rows
    }
    fn view(&self) -> ColumnView {
        unsafe { self.column.view() }
    }
}

/// One dense u32 key level, in lexicographic order. The first level owns the
/// domain+1 lower-bound directory; subsequent levels order rows within ranges.
#[derive(Clone)]
pub struct KeyDescriptor {
    pub column: ColumnDescriptor,
    pub domain_size: usize,
}
#[derive(Clone)]
pub struct RangeSelection {
    pub column: ColumnDescriptor,
    pub start: i64,
    pub end: i64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PayloadKind {
    U32,
    I32,
}
#[derive(Clone)]
pub struct PayloadDescriptor {
    pub column: ColumnDescriptor,
    pub kind: PayloadKind,
}
#[derive(Clone)]
pub struct RangeDescriptor {
    pub key: Vec<KeyDescriptor>,
    /// A half-open predicate applied before sorting, not a post-build filter.
    pub selection: Option<RangeSelection>,
    pub payload: Vec<PayloadDescriptor>,
}

/// Catalog names are relative to `table`; dictionary sizes remain explicit.
pub struct CatalogKeyDescriptor {
    pub column: String,
    pub domain_size: usize,
}
pub struct CatalogRangeSelection {
    pub column: String,
    pub start: i64,
    pub end: i64,
}
pub struct CatalogRangeDescriptor {
    pub table: String,
    pub key: Vec<CatalogKeyDescriptor>,
    pub selection: Option<CatalogRangeSelection>,
    pub payload: Vec<String>,
}
impl RangeDescriptor {
    pub fn from_catalog(catalog: &Catalog, descriptor: &CatalogRangeDescriptor) -> Result<Self> {
        let column = |name: &str| ColumnDescriptor::from_catalog(catalog, &descriptor.table, name);
        let key = descriptor
            .key
            .iter()
            .map(|key| {
                if !matches!(
                    &catalog.column(&descriptor.table, &key.column)?.kind,
                    Kind::Id(_)
                ) {
                    return Err("range keys must be dense domain ID columns".into());
                }
                Ok(KeyDescriptor {
                    column: column(&key.column)?,
                    domain_size: key.domain_size,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let selection = descriptor
            .selection
            .as_ref()
            .map(|selection| -> Result<RangeSelection> {
                Ok(RangeSelection {
                    column: column(&selection.column)?,
                    start: selection.start,
                    end: selection.end,
                })
            })
            .transpose()?;
        let payload = descriptor
            .payload
            .iter()
            .map(|name| {
                let kind = match &catalog.column(&descriptor.table, name)?.kind {
                    Kind::Id(_) => PayloadKind::U32,
                    Kind::I32 => PayloadKind::I32,
                };
                Ok(PayloadDescriptor {
                    column: column(name)?,
                    kind,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            key,
            selection,
            payload,
        })
    }
}

/// Nonowning generated-kernel ABI. Keep the provider alive and use its stream.
/// Offsets have `domain + 1` entries; payload columns have `len` entries.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct RangeView {
    pub offsets: ColumnView,
    pub payload: *const ColumnView,
    pub payload_columns: u32,
    pub domain: u32,
    pub len: u32,
}

/// An immutable, duplicate-preserving sorted range index. No uniqueness of
/// keys is assumed: lookup is always [offsets[key], offsets[key+1]).
pub struct RangeProvider {
    offsets: EncodedColumn,
    payload: Vec<EncodedColumn>,
    payload_views: Arc<Buffer<ColumnView>>,
    domain: u32,
    len: u32,
}
impl RangeProvider {
    pub fn build(pool: Arc<Pool>, descriptor: &RangeDescriptor) -> Result<Self> {
        RangeBuilder::new(&pool)?.build(descriptor)
    }
    pub fn from_catalog(
        pool: Arc<Pool>,
        catalog: &Catalog,
        descriptor: &CatalogRangeDescriptor,
    ) -> Result<Self> {
        Self::build(pool, &RangeDescriptor::from_catalog(catalog, descriptor)?)
    }
    pub fn len(&self) -> usize {
        self.len as usize
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn domain_size(&self) -> usize {
        self.domain as usize
    }
    pub fn pool(&self) -> &Arc<Pool> {
        self.offsets.pool()
    }
    pub fn offsets(&self) -> &EncodedColumn {
        &self.offsets
    }
    pub fn payload(&self) -> &[EncodedColumn] {
        &self.payload
    }
    pub fn bytes(&self) -> usize {
        self.offsets.bytes()
            + self.payload_views.bytes()
            + self.payload.iter().map(EncodedColumn::bytes).sum::<usize>()
    }
    /// # Safety
    /// The returned views may only be read on `self.pool().stream()` while
    /// this provider is alive, within the advertised domain/row/column bounds.
    pub unsafe fn view(&self) -> RangeView {
        RangeView {
            offsets: unsafe { self.offsets.view() },
            payload: self.payload_views.ptr(),
            payload_columns: self.payload.len() as u32,
            domain: self.domain,
            len: self.len,
        }
    }
}

thread_local! {
    static BUILD_MODULE: RefCell<Option<(Arc<CudaContext>, kernels::build::LoadedModule)>> =
        const { RefCell::new(None) };
    static RANGE_RADIX_WORKSPACE: RefCell<Option<RangeRadixWorkspace>> =
        const { RefCell::new(None) };
}

/// Keep only the most recently used context's named build module on this thread.
pub(crate) fn build_module(pool: &Pool) -> Result<kernels::build::LoadedModule> {
    BUILD_MODULE.with(|cache| {
        if let Some((context, module)) = cache.borrow().as_ref() {
            if Arc::ptr_eq(context, pool.context()) {
                return Ok(module.clone());
            }
        }
        let module = unsafe { kernels::build::load(pool.context())? };
        *cache.borrow_mut() = Some((pool.context().clone(), module.clone()));
        Ok(module)
    })
}

struct RangeRadixWorkspace {
    by_rows: Vec<(usize, rccl_cub::DeviceRadixSortBufferWorkspace)>,
    pool: Arc<Pool>,
}

fn with_range_radix_workspace(
    pool: &Arc<Pool>,
    launch: &rccl_cub::cuda::Launch<'_, '_>,
    rows: usize,
    sort: impl FnOnce(&mut rccl_cub::DeviceRadixSortBufferWorkspace) -> Result<()>,
) -> Result<()> {
    RANGE_RADIX_WORKSPACE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache
            .as_ref()
            .is_none_or(|scratch| !Arc::ptr_eq(&scratch.pool, pool))
        {
            // Keep workspaces for only the current pool, releasing allocations
            // from unrelated contexts when the caller switches pools.
            *cache = Some(RangeRadixWorkspace {
                by_rows: Vec::new(),
                pool: pool.clone(),
            });
        }
        let scratch = cache.as_mut().unwrap();
        let index = match scratch.by_rows.iter().position(|(len, _)| *len == rows) {
            Some(index) => index,
            None => {
                let workspace =
                    rccl_cub::device_radix_sort_pairs_buffer_workspace::<u32, u32>(launch, rows)?;
                scratch.by_rows.push((rows, workspace));
                scratch.by_rows.len() - 1
            }
        };
        // RCCL clears the entire lookback allocation each pass, so retain exact
        // row-sized controls rather than clearing the largest provider's scratch.
        // The retained pool fixes the stream, so successive uses remain ordered.
        sort(&mut scratch.by_rows[index].1)
    })
}

/// Per-provider count owners and descriptor storage. Construct during program
/// setup; published results retain their owners across later builds.
pub struct RangeWorkspace {
    row_count: Arc<Buffer<u64>>,
    offset_count: Arc<Buffer<u64>>,
    payload_views: Arc<Buffer<ColumnView>>,
    host_views: Vec<ColumnView>,
    row_value: usize,
    base_rows: usize,
    entries: usize,
}
impl RangeWorkspace {
    pub fn new(pool: Arc<Pool>, descriptor: &RangeDescriptor) -> Result<Self> {
        let first = descriptor
            .key
            .first()
            .ok_or("range provider needs at least one key")?;
        let entries = first
            .domain_size
            .checked_add(1)
            .ok_or("range domain overflow")?;
        Ok(Self {
            row_count: Arc::new(Buffer::from_host(
                pool.clone(),
                &[first.column.rows as u64],
            )?),
            offset_count: Arc::new(Buffer::from_host(pool.clone(), &[entries as u64])?),
            payload_views: Arc::new(unsafe {
                Buffer::uninitialized(pool, descriptor.payload.len())?
            }),
            host_views: Vec::with_capacity(descriptor.payload.len()),
            row_value: first.column.rows,
            base_rows: first.column.rows,
            entries,
        })
    }
    pub fn matches(&self, pool: &Arc<Pool>, descriptor: &RangeDescriptor) -> bool {
        Arc::ptr_eq(self.row_count.pool(), pool)
            && descriptor.key.first().is_some_and(|first| {
                first.column.rows == self.base_rows
                    && first.domain_size.checked_add(1) == Some(self.entries)
            })
            && descriptor.payload.len() == self.payload_views.len()
    }
}

/// Reuse loaded build/RCCL programs across providers on the existing pool.
/// Fresh builders also share the named module and radix controls on this thread.
pub struct RangeBuilder<'pool> {
    pool: &'pool Arc<Pool>,
    module: kernels::build::LoadedModule,
    thrust: rccl_thrust::cuda::Program<'pool>,
    cub: rccl_cub::cuda::Program<'pool>,
}
impl<'pool> RangeBuilder<'pool> {
    pub fn new(pool: &'pool Arc<Pool>) -> Result<Self> {
        Ok(Self {
            module: build_module(pool)?,
            thrust: rccl_thrust::cuda::Program::load(pool.context())?,
            cub: rccl_cub::cuda::Program::load(pool.context())?,
            pool,
        })
    }
    pub fn pool(&self) -> &Arc<Pool> {
        self.pool
    }
    pub fn from_catalog(
        &self,
        catalog: &Catalog,
        descriptor: &CatalogRangeDescriptor,
    ) -> Result<RangeProvider> {
        self.build(&RangeDescriptor::from_catalog(catalog, descriptor)?)
    }
    pub fn build(&self, descriptor: &RangeDescriptor) -> Result<RangeProvider> {
        let mut workspace = RangeWorkspace::new(self.pool.clone(), descriptor)?;
        self.build_with_workspace(descriptor, &mut workspace)
    }
    /// Build on the existing stream, reusing setup-time count and view owners.
    pub fn build_with_workspace(
        &self,
        descriptor: &RangeDescriptor,
        workspace: &mut RangeWorkspace,
    ) -> Result<RangeProvider> {
        if !workspace.matches(self.pool, descriptor) {
            return Err("range workspace does not match the provider descriptor".into());
        }
        let first = descriptor
            .key
            .first()
            .ok_or("range provider needs at least one key")?;
        let base_rows = first.column.rows;
        if base_rows > u32::MAX as usize || descriptor.payload.len() > u32::MAX as usize {
            return Err("range provider exceeds the u32 row/column domain".into());
        }
        let check_column = |column: &ColumnDescriptor| -> Result<()> {
            if column.rows != base_rows || !Arc::ptr_eq(column.column.pool(), &self.pool) {
                return Err(
                    "range columns must have identical live row counts and share the builder pool"
                        .into(),
                );
            }
            Ok(())
        };
        for key in &descriptor.key {
            check_column(&key.column)?;
            if key.domain_size >= u32::MAX as usize {
                return Err("dense key domain must leave u32::MAX available as a sentinel".into());
            }
            match key.column.column.bounds() {
                Bounds::Empty if base_rows == 0 => {}
                Bounds::Inclusive { min, max }
                    if min >= 0 && max >= min && max < key.domain_size as i64 => {}
                _ => return Err("key bounds do not fit the declared dense domain".into()),
            }
        }
        if let Some(selection) = &descriptor.selection {
            check_column(&selection.column)?;
        }
        for payload in &descriptor.payload {
            check_column(&payload.column)?;
            let (low, high) = match payload.kind {
                PayloadKind::U32 => (0, u32::MAX as i64),
                PayloadKind::I32 => (i32::MIN as i64, i32::MAX as i64),
            };
            match payload.column.column.bounds() {
                Bounds::Empty if base_rows == 0 => {}
                Bounds::Inclusive { min, max } if min >= low && min <= max && max <= high => {}
                _ => return Err("payload bounds do not fit the declared lane type".into()),
            }
        }
        let pool = self.pool;
        let stream = pool.stream();
        let exec = self.thrust.par_in(stream, pool.resource())?;
        let launch = self.cub.on(stream)?;
        let module = &self.module;
        let (rows, selected) = match &descriptor.selection {
            Some(selection) if base_rows > 0 => {
                let flags = temporary::<u32>(stream, base_rows)?;
                module.range_flags(
                    stream,
                    &module.prepare_range_flags(config(base_rows))?,
                    selection.column.view(),
                    base_rows as u32,
                    selection.start,
                    selection.end,
                    pointer(&flags),
                )?;
                let mut selected = temporary::<u32>(stream, base_rows)?;
                let mut count = temporary::<u64>(stream, 1)?;
                select_flagged(
                    &exec,
                    arithmetic_range(0_u32, 1, base_rows),
                    flags.view(),
                    selected.view_mut(),
                    &mut count,
                )?;
                let rows = count.to_host_vec(stream)?[0] as usize;
                release(stream, flags)?;
                release(stream, count)?;
                (rows, Some(selected))
            }
            _ => (base_rows, None),
        };
        let entries = first.domain_size + 1;
        if rows != workspace.row_value {
            if Arc::strong_count(&workspace.row_count) != 1 {
                workspace.row_count =
                    Arc::new(Buffer::from_host_async(pool.clone(), &[rows as u64])?);
            } else {
                // The workspace is the only owner; all prior reads on this
                // pool stream precede this asynchronous update.
                unsafe { workspace.row_count.write_async(&[rows as u64])? };
            }
            workspace.row_value = rows;
        }
        let row_count = workspace.row_count.clone();
        let offset_count = workspace.offset_count.clone();
        // Empty selections still own a complete all-zero directory.
        if rows == 0 {
            let zeros = Buffer::<i64>::zeroed(pool.clone(), entries)?;
            let offsets = unsafe {
                EncodedColumn::from_device(
                    pool.clone(),
                    zeros.ptr(),
                    entries,
                    offset_count,
                    0,
                    Codec::For,
                    Bounds::singleton(0),
                )?
            };
            let payload = descriptor
                .payload
                .iter()
                .map(|payload| unsafe {
                    pack_payload(pool, payload, std::ptr::null(), 0, row_count.clone())
                })
                .collect::<Result<Vec<_>>>()?;
            if let Some(selected) = selected {
                release(stream, selected)?;
            }
            return finish(offsets, payload, first.domain_size, rows, workspace);
        }
        let mut keys = temporary::<u32>(stream, rows)?;
        let mut order = temporary::<u32>(stream, rows)?;
        let source = selected
            .as_ref()
            .map_or(std::ptr::null(), |selected| pointer(selected) as *const u32);
        if descriptor.key.len() > 1 {
            let plain = descriptor
                .key
                .iter()
                .map(|key| {
                    let out = temporary::<u32>(stream, rows)?;
                    module.sequence_decode_column(
                        stream,
                        &module.prepare_sequence_decode_column(config(rows))?,
                        key.column.view(),
                        source,
                        rows as u32,
                        pointer(&out),
                    )?;
                    Ok(out)
                })
                .collect::<Result<Vec<_>>>()?;
            let widths = descriptor
                .key
                .iter()
                .map(|key| key_bits(key.domain_size))
                .collect::<Vec<_>>();
            let mut permutation = temporary::<u32>(stream, rows)?;
            sort_columns_lexicographic(
                &exec,
                &plain.iter().collect::<Vec<_>>(),
                &widths,
                &mut permutation,
                None,
            )?;
            let base_order = match &selected {
                Some(selected) => {
                    let mut mapped = temporary::<u32>(stream, rows)?;
                    gather(
                        &exec,
                        selected.view(),
                        permutation.view(),
                        mapped.view_mut(),
                    )?;
                    release(stream, permutation)?;
                    mapped
                }
                None => permutation,
            };
            module.keys_and_rows(
                stream,
                &module.prepare_keys_and_rows(config(rows))?,
                first.column.view(),
                pointer(&base_order),
                rows as u32,
                pointer(&keys),
                pointer(&order),
            )?;
            for buffer in plain.into_iter().chain([base_order]) {
                release(stream, buffer)?;
            }
        } else {
            module.keys_and_rows(
                stream,
                &module.prepare_keys_and_rows(config(rows))?,
                first.column.view(),
                source,
                rows as u32,
                pointer(&keys),
                pointer(&order),
            )?;
            let bits = key_bits(first.domain_size);
            if rows > 1 && bits > 0 {
                let mut alternate_keys = temporary::<u32>(stream, rows)?;
                let mut alternate_order = temporary::<u32>(stream, rows)?;
                let mut pairs = DoubleBuffer::paired(
                    (&mut keys, &mut order),
                    (&mut alternate_keys, &mut alternate_order),
                )
                .ok_or("radix double buffer extents differ")?;
                with_range_radix_workspace(pool, &launch, rows, |workspace| {
                    rccl_cub::device_radix_sort_pairs_buffer(
                        &launch,
                        &mut pairs,
                        0..bits,
                        workspace,
                    )?;
                    Ok(())
                })?;
                let swapped = !pairs.current_is_first();
                drop(pairs);
                if swapped {
                    std::mem::swap(&mut keys, &mut alternate_keys);
                    std::mem::swap(&mut order, &mut alternate_order);
                }
                release(stream, alternate_keys)?;
                release(stream, alternate_order)?;
            }
        }
        let mut offsets = temporary::<u64>(stream, entries)?;
        lower_bound(
            &exec,
            keys.view(),
            arithmetic_range(0_u32, 1, entries),
            offsets.view_mut(),
        )?;
        let payload = descriptor
            .payload
            .iter()
            .map(|payload| unsafe {
                pack_payload(pool, payload, pointer(&order), rows, row_count.clone())
            })
            .collect::<Result<Vec<_>>>()?;
        let packed_offsets = unsafe {
            EncodedColumn::from_device(
                pool.clone(),
                pointer(&offsets) as *const i64,
                entries,
                offset_count,
                0,
                Codec::For,
                Bounds::Inclusive {
                    min: 0,
                    max: rows as i64,
                },
            )?
        };
        for buffer in [keys, order].into_iter().chain(selected) {
            release(stream, buffer)?;
        }
        release(stream, offsets)?;
        finish(packed_offsets, payload, first.domain_size, rows, workspace)
    }
}

fn finish(
    offsets: EncodedColumn,
    payload: Vec<EncodedColumn>,
    domain: usize,
    len: usize,
    workspace: &mut RangeWorkspace,
) -> Result<RangeProvider> {
    workspace.host_views.clear();
    workspace
        .host_views
        .extend(payload.iter().map(|column| unsafe { column.view() }));
    let views = &workspace.host_views;
    if Arc::strong_count(&workspace.payload_views) != 1 {
        workspace.payload_views = Arc::new(Buffer::from_host_async(offsets.pool().clone(), views)?);
    } else {
        unsafe { workspace.payload_views.write_async(views)? };
    }
    let payload_views = workspace.payload_views.clone();
    Ok(RangeProvider {
        offsets,
        payload,
        payload_views,
        domain: domain as u32,
        len: len as u32,
    })
}
unsafe fn pack_payload(
    pool: &Arc<Pool>,
    payload: &PayloadDescriptor,
    order: *const u32,
    rows: usize,
    count: Arc<Buffer<u64>>,
) -> Result<EncodedColumn> {
    let bounds = payload.column.column.bounds();
    match payload.kind {
        PayloadKind::U32 => unsafe {
            EncodedColumn::from_gather::<_, u32>(
                pool.clone(),
                payload.column.view(),
                order,
                rows,
                count,
                0,
                bounds,
            )
        },
        PayloadKind::I32 => unsafe {
            EncodedColumn::from_gather::<_, i32>(
                pool.clone(),
                payload.column.view(),
                order,
                rows,
                count,
                0,
                bounds,
            )
        },
    }
}
fn config(rows: usize) -> LaunchConfig1D {
    LaunchConfig1D::new(rows.div_ceil(256).clamp(1, 568 * 4) as u32, 256, 0)
}
fn key_bits(domain: usize) -> u32 {
    if domain <= 1 {
        0
    } else {
        usize::BITS - (domain - 1).leading_zeros()
    }
}
struct TemporaryCache<T: DeviceCopy> {
    stream: Arc<CudaStream>,
    buffers: Vec<DeviceBuffer<T>>,
}
impl<T: DeviceCopy> Drop for TemporaryCache<T> {
    fn drop(&mut self) {
        for buffer in self.buffers.drain(..) {
            self.stream
                .context()
                .record_err(unsafe { buffer.drop_async(&self.stream) });
        }
    }
}
thread_local! {
    static TEMP_U32: RefCell<Option<TemporaryCache<u32>>> = const { RefCell::new(None) };
    static TEMP_U64: RefCell<Option<TemporaryCache<u64>>> = const { RefCell::new(None) };
}
trait TemporaryElement: DeviceCopy + 'static {
    fn cache() -> &'static std::thread::LocalKey<RefCell<Option<TemporaryCache<Self>>>>;
}
impl TemporaryElement for u32 {
    fn cache() -> &'static std::thread::LocalKey<RefCell<Option<TemporaryCache<Self>>>> {
        &TEMP_U32
    }
}
impl TemporaryElement for u64 {
    fn cache() -> &'static std::thread::LocalKey<RefCell<Option<TemporaryCache<Self>>>> {
        &TEMP_U64
    }
}
fn temporary<T: TemporaryElement>(stream: &Arc<CudaStream>, len: usize) -> Result<DeviceBuffer<T>> {
    T::cache().with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache
            .as_ref()
            .is_none_or(|cache| !Arc::ptr_eq(&cache.stream, stream))
        {
            *cache = Some(TemporaryCache {
                stream: stream.clone(),
                buffers: Vec::new(),
            });
            stream.context().bind_to_thread()?;
        }
        let buffers = &mut cache.as_mut().unwrap().buffers;
        if let Some(index) = buffers.iter().position(|buffer| buffer.len() == len) {
            return Ok(buffers.swap_remove(index));
        }
        Ok(unsafe { DeviceBuffer::uninitialized_async(stream, len)? })
    })
}
fn pointer<T: DeviceCopy>(buffer: &DeviceBuffer<T>) -> *mut T {
    buffer.cu_deviceptr() as *mut T
}
fn release<T: TemporaryElement>(stream: &CudaStream, buffer: DeviceBuffer<T>) -> Result<()> {
    T::cache().with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(cache) = cache.as_mut() {
            if cache.stream.cu_stream() == stream.cu_stream()
                && Arc::ptr_eq(cache.stream.context(), stream.context())
            {
                cache.buffers.push(buffer);
                return Ok(());
            }
        }
        unsafe { buffer.drop_async(stream)? };
        Ok(())
    })
}
