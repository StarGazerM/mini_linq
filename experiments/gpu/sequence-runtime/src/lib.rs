//! Sequence execution primitives over resident columns. Callers supply the
//! recorded key order, selection and payload; this crate makes no plan choices.
pub mod canon;
pub mod device;
pub mod group;
pub mod index_bytes;
mod kernels;
pub mod offsets;
pub mod selected;
pub use selected::{select, SelectedRows};

pub use cq_gpu_runtime::{Bounds, Buffer, Codec, ColumnView, DeviceColumns, EncodedColumn, Pool};
use cuda_core::{CudaContext, CudaStream, DeviceCopy, LaunchConfig1D};
type DeviceBuffer<T> = rccl_memory::DeviceBuffer<'static, T, &'static MemoryResource>;
use gpu_compress::{Catalog, Domain};
use gpu_compress_host::Kind;
pub use gpu_device_columns::{MemoryResource, OutputCount};
use rccl_cub::DoubleBuffer;
use rccl_thrust::{
    arithmetic_range, gather, lower_bound, select_flagged, sort_columns_lexicographic,
};
use std::{cell::RefCell, sync::Arc};

pub type Result<T, E = Box<dyn std::error::Error>> = std::result::Result<T, E>;

/// Launch geometry of one auxiliary kernel family, printed from the GPU realization record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub block_threads: u32,
    pub shared_bytes: u32,
}

impl Geometry {
    /// The family's launch configuration over `blocks` blocks.
    pub(crate) fn config(self, blocks: u32) -> LaunchConfig1D {
        LaunchConfig1D::new(blocks, self.block_threads, self.shared_bytes)
    }

    /// Resident grid for a grid-stride kernel over `items` elements.
    pub(crate) fn grid(self, function: &cuda_core::CudaFunction, items: usize) -> Result<u32> {
        cq_gpu_runtime::launch_grid_for_kernel(
            function,
            items.div_ceil(self.block_threads as usize),
            self.block_threads,
            self.shared_bytes,
        )
    }

    /// Reject a recorded geometry the family's kernels cannot execute.
    pub(crate) fn check(self, family: &str, supported: Geometry) -> Result<Self> {
        if self == supported {
            Ok(self)
        } else {
            Err(format!(
                "unsupported {family} auxiliary geometry {self:?}; its kernels support only {supported:?}"
            )
            .into())
        }
    }
}

/// Per-family auxiliary kernel geometry (formal rule GPU-Realization auxiliary record).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Auxiliary {
    /// `range_flags`: selection predicates of `select`, `radix_layout` and range builds.
    pub selection: Geometry,
    /// `offsets::scan` adjacent-prefix check.
    pub offsets: Geometry,
    /// Range-provider key extraction (`keys_and_rows`, `sequence_decode_column`).
    pub range: Geometry,
    /// Canon element kernels (`gather_lane`, `unpack_field`).
    pub canon: Geometry,
    /// Group element kernels (`identity`, `gather_key`, `boundaries`).
    pub group: Geometry,
    /// Group `reduce`: one block per group and column.
    pub group_reduce: Geometry,
    /// `numeric_bounds` reduction of uploads and measured publications.
    pub bounds: Geometry,
}

impl Auxiliary {
    /// Supported-value table. Every auxiliary kernel's launch contract and
    /// `launch_bounds` fix 256-thread blocks, with matching grid strides and
    /// tree reductions. Element kernels use no shared memory; `reduce` stages
    /// 256 i64 lanes (4096 B); `numeric_bounds` writes one partial pair per
    /// warp. There is deliberately no public default: callers pass the record.
    pub(crate) const SUPPORTED: Auxiliary = Auxiliary {
        selection: Geometry {
            block_threads: 256,
            shared_bytes: 0,
        },
        offsets: Geometry {
            block_threads: 256,
            shared_bytes: 0,
        },
        range: Geometry {
            block_threads: 256,
            shared_bytes: 0,
        },
        canon: Geometry {
            block_threads: 256,
            shared_bytes: 0,
        },
        group: Geometry {
            block_threads: 256,
            shared_bytes: 0,
        },
        group_reduce: Geometry {
            block_threads: 256,
            shared_bytes: 4096,
        },
        bounds: Geometry {
            block_threads: 256,
            shared_bytes: 0,
        },
    };
    /// `GlobalFold::finish` combine: 256 i64 values, then 256 u32 presence
    /// flags at byte 2048 (3072 B), one block per aggregate.
    pub(crate) const SUPPORTED_COMBINE: Geometry = Geometry {
        block_threads: 256,
        shared_bytes: 3072,
    };

    /// Reject any family geometry its kernels cannot execute; never adjusts.
    pub fn validate(&self) -> Result<()> {
        let supported = Self::SUPPORTED;
        self.selection.check("selection", supported.selection)?;
        self.offsets.check("offsets", supported.offsets)?;
        self.range.check("range", supported.range)?;
        self.canon.check("canon", supported.canon)?;
        self.group.check("group", supported.group)?;
        self.group_reduce
            .check("group_reduce", supported.group_reduce)?;
        self.bounds.check("bounds", supported.bounds)?;
        Ok(())
    }
}

/// How an uploaded column's bounds are established.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UploadBounds {
    /// One device reduction and one readback over the uploaded column.
    Measure,
    /// The column scalar type's full range, without a readback.
    Scalar,
}

/// Device codec of an uploaded column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UploadCodec {
    /// Frame-of-reference over the column's established bounds.
    For,
}

/// Recorded representation of one uploaded numeric column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UploadColumn {
    pub codec: UploadCodec,
    pub bounds: UploadBounds,
}

thread_local! {
    // Scratch capacity only: every invocation repopulates every live lane.
    static NUMERIC_UPLOAD_SCRATCH: RefCell<Vec<Vec<i64>>> = const { RefCell::new(Vec::new()) };
}

thread_local! {
    static QUERY_CLEANUP: RefCell<Option<Vec<fn()>>> = const { RefCell::new(None) };
}

/// Own the lifetime of generated thread-local caches for one complete query.
/// Keep this scope alive across warmup and measured iterations, then drop it
/// after all query outputs and inputs. It must stay on its creating thread.
pub struct QueryScope(std::marker::PhantomData<std::rc::Rc<()>>);

impl QueryScope {
    pub fn new() -> Self {
        QUERY_CLEANUP.with(|cleanup| {
            let mut cleanup = cleanup.borrow_mut();
            assert!(cleanup.is_none(), "GPU query scopes cannot be nested");
            *cleanup = Some(Vec::new());
        });
        Self(std::marker::PhantomData)
    }
}

impl Default for QueryScope {
    fn default() -> Self {
        Self::new()
    }
}

/// Register an emitted cache's release hook once per active query scope.
#[doc(hidden)]
pub fn register_query_cleanup(release: fn()) {
    QUERY_CLEANUP.with(|cleanup| {
        if let Some(cleanup) = cleanup.borrow_mut().as_mut() {
            if !cleanup
                .iter()
                .any(|&existing| std::ptr::fn_addr_eq(existing, release))
            {
                cleanup.push(release);
            }
        }
    });
}

/// Codegen partitioning for generated query bodies.
///
/// Rustc ignores modules nested in functions when assigning codegen units, and
/// LLVM inlines once-called closures into their caller. Each module-level
/// runner keeps its closure out of the generated caller and gives it a distinct
/// codegen-unit home. Callers mark the closure `#[inline]`: a non-inline closure
/// body is homed with its defining function, not with this runner.
#[doc(hidden)]
pub mod __codegen {
    macro_rules! partitions {
        ($($name:ident),* $(,)?) => {$(
            pub mod $name {
                #[inline(never)]
                pub fn run<R>(evaluate: impl FnOnce() -> R) -> R {
                    evaluate()
                }
            }
        )*};
    }
    partitions!(
        partition_0,
        partition_1,
        partition_2,
        partition_3,
        partition_4,
        partition_5,
        partition_6,
        partition_7,
        partition_8,
        partition_9,
        partition_10,
        partition_11,
        partition_12,
        partition_13,
        partition_14,
        partition_15,
    );
}

impl Drop for QueryScope {
    fn drop(&mut self) {
        let cleanup = QUERY_CLEANUP.with(|cleanup| cleanup.borrow_mut().take());
        for release in cleanup.into_iter().flatten() {
            release();
        }
        RADIX_WORKSPACES.with(|cache| cache.borrow_mut().clear());
        TEMP_U32.with(|cache| cache.borrow_mut().take());
        TEMP_U64.with(|cache| cache.borrow_mut().take());
        gpu_datalog_runtime::radix_run::release_set_scratch();
        BUILD_MODULE.with(|cache| cache.borrow_mut().take());
        gpu_device_columns::release_cached_kernels();
        NUMERIC_UPLOAD_SCRATCH.with(|cache| cache.borrow_mut().clear());
    }
}

/// Upload scalar numeric lanes emitted by a host Sequence boundary.
/// The translator supplies one i64 lane per declared field, in layout order,
/// and one recorded [`UploadColumn`] per field. Measured columns run one
/// `numeric_bounds` reduction and readback each, with the recorded `bounds`
/// family `geometry`; scalar columns use their type's full range.
/// Scalar codecs retain Decimal coefficients, Date epoch days, and published
/// symbol codes without staging a tuple vector. Scratch capacities are reused,
/// and all realization and upload work remains inside the provider's build timing.
#[doc(hidden)]
pub fn upload_numeric_columns<Row: gpu_datalog_runtime::ScalarTuple, F>(
    pool: Arc<Pool>,
    fill: F,
    columns: &[UploadColumn],
    geometry: Geometry,
) -> Result<DeviceColumns<'static, Row>>
where
    F: FnOnce(&mut [Vec<i64>]) -> Result<()>,
{
    if columns.len() != Row::COLUMNS.len() {
        return Err("numeric upload records must describe every row column".into());
    }
    geometry.check("bounds", Auxiliary::SUPPORTED.bounds)?;
    // Check out the scratch rather than holding a RefCell borrow across host
    // evaluation: an opaque host expression may invoke another query.
    let mut raw = NUMERIC_UPLOAD_SCRATCH.with(|cache| std::mem::take(&mut *cache.borrow_mut()));
    raw.resize_with(Row::COLUMNS.len(), Vec::new);
    let result = (|| {
        fill(&mut raw)?;
        upload_numeric_lanes(pool, &raw, columns, geometry)
    })();
    for column in &mut raw {
        column.clear();
    }
    NUMERIC_UPLOAD_SCRATCH.with(|cache| *cache.borrow_mut() = raw);
    result
}

/// The full range of a scalar lane, which must be representable in i64.
fn scalar_bounds(column: gpu_datalog_runtime::Column) -> Result<Bounds> {
    let bits = column.bits();
    let (min, max) = if column.is_signed() {
        (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1)
    } else {
        (0, (1i128 << bits) - 1)
    };
    match (i64::try_from(min), i64::try_from(max)) {
        (Ok(min), Ok(max)) => Ok(Bounds::Inclusive { min, max }),
        _ => Err(format!("{column:?} upload lanes have no i64 scalar range").into()),
    }
}

fn upload_numeric_lanes<Row: gpu_datalog_runtime::ScalarTuple>(
    pool: Arc<Pool>,
    raw: &[Vec<i64>],
    records: &[UploadColumn],
    geometry: Geometry,
) -> Result<DeviceColumns<'static, Row>> {
    let rows = raw.first().ok_or("numeric upload needs a column")?.len();
    if raw.len() != Row::COLUMNS.len() || raw.iter().any(|column| column.len() != rows) {
        return Err("numeric upload columns must match the row layout".into());
    }
    if rows > u32::MAX as usize {
        return Err("numeric upload exceeds the u32 row domain".into());
    }
    let metadata = Arc::new(Buffer::from_host(
        pool.clone(),
        &[gpu_device_columns::OutputCount {
            rows: rows as u64,
            error: 0,
            padding: 0,
        }],
    )?);
    let columns = raw
        .iter()
        .zip(records)
        .zip(Row::COLUMNS)
        .map(|((values, record), &column)| {
            let input = Buffer::from_host(pool.clone(), values)?;
            let bounds = match record.bounds {
                UploadBounds::Measure => source_bounds(
                    &pool,
                    ColumnDescriptor::native_source(input.ptr().cast(), rows),
                    rows,
                    geometry,
                )?,
                UploadBounds::Scalar => scalar_bounds(column)?,
            };
            let bounds = if rows == 0 { Bounds::Empty } else { bounds };
            let codec = match record.codec {
                UploadCodec::For => Codec::For,
            };
            // The upload initialized the complete prefix and already measured
            // its source bounds (or supplied a proven scalar range). Reuse that
            // enclosure without another publication reduction or readback.
            unsafe {
                EncodedColumn::from_device_with_bounds(
                    pool.clone(),
                    input.ptr(),
                    rows,
                    metadata.clone(),
                    0,
                    codec,
                    bounds,
                )
                .map(Arc::new)
            }
        })
        .collect::<Result<Vec<_>>>()?;
    let bounds = columns.iter().map(|column| column.bounds()).collect();
    let host_views: Vec<_> = columns
        .iter()
        .map(|column| unsafe { column.view() })
        .collect();
    let views = Arc::new(Buffer::from_host(pool.clone(), &host_views)?);
    // Columns are initialized on this pool's stream; the table retains every
    // owner and its exact row count. Primitive lanes need no dictionary IDs.
    Ok(unsafe {
        DeviceColumns::from_parts(
            gpu_device_columns::Table {
                columns,
                views,
                metadata,
                capacity: rows,
                bounds,
                statuses: Vec::new(),
            },
            Arc::new(cq_gpu_runtime::Dictionary::default()),
            pool,
        )
    })
}

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
    // DECODE ?: domain ordinal inverse has no local caller read/publication evidence.
    pub fn decode(&self, id: u32) -> Option<i64> {
        // DECODE ?: checked domain lookup reconstructs a source key for an unspecified consumer.
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
/// Bounds enclose the live values; a subset need not retain either endpoint.
#[derive(Clone)]
pub struct ColumnDescriptor {
    owner: ColumnOwner,
    rows: usize,
    bounds: Bounds,
    source: gpu_device_columns::SourceView,
}
#[derive(Clone)]
enum ColumnOwner {
    Encoded(Arc<EncodedColumn>),
    Rows(Arc<Buffer<i64>>),
    Packed(Arc<Buffer<u64>>),
}
impl ColumnDescriptor {
    pub fn from_catalog(catalog: &Catalog, table: &str, column: &str) -> Result<Self> {
        unsafe {
            Self::new(
                catalog.column(table, column)?.column.clone(),
                catalog.rows(table)?,
            )
        }
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
        Ok(Self {
            bounds: column.bounds(),
            source: unsafe { column.view() }.into(),
            owner: ColumnOwner::Encoded(column),
            rows,
        })
    }
    /// Project a field from resident row-major storage without copying.
    ///
    /// # Safety
    /// The first `count` rows must be initialized on the buffer's pool stream,
    /// remain immutable during the build, and have values enclosed by `bounds`.
    pub unsafe fn from_rows(
        rows: Arc<Buffer<i64>>,
        count: usize,
        width: usize,
        slot: usize,
        bounds: Bounds,
    ) -> Result<Self> {
        if slot >= width || count.checked_mul(width).is_none_or(|len| len > rows.len()) {
            return Err("stored row projection exceeds its buffer".into());
        }
        let data = rows
            .ptr()
            .wrapping_add(if count == 0 { 0 } else { slot })
            .cast();
        let mut source = Self::native_source(data, count);
        source.stride = width as u64;
        Ok(Self {
            owner: ColumnOwner::Rows(rows),
            rows: count,
            bounds,
            source,
        })
    }
    /// Project a recorded canonical-key field without unpacking its column.
    ///
    /// # Safety
    /// The first `count` keys must be initialized on the buffer's pool stream
    /// and immutable during the build. Decoded values must fit i64 and `bounds`.
    pub unsafe fn from_packed(
        keys: Arc<Buffer<u64>>,
        count: usize,
        lanes: usize,
        field: canon::PackedField,
        bounds: Bounds,
    ) -> Result<Self> {
        if lanes == 0
            || count > keys.len() / lanes
            || field.bits > 64
            || field.shift as usize > lanes.saturating_mul(64)
            || field.bits as usize > lanes.saturating_mul(64) - field.shift as usize
        {
            return Err("packed source projection exceeds its buffer or key".into());
        }
        let mut source = Self::native_source(keys.ptr(), count);
        source.packed = 1;
        source.stride = lanes as u64;
        source.shift = field.shift;
        source.mask = if field.bits == 64 {
            u64::MAX
        } else {
            (1u64 << field.bits) - 1
        };
        source.base = field.base;
        Ok(Self {
            owner: ColumnOwner::Packed(keys),
            rows: count,
            bounds,
            source,
        })
    }
    fn native_source(data: *const u64, rows: usize) -> gpu_device_columns::SourceView {
        ColumnView {
            headers: std::ptr::null(),
            data,
            len: rows as u64,
            count: std::ptr::null(),
            codec: Codec::Plain as u32,
        }
        .into()
    }
    pub fn pool(&self) -> &Arc<Pool> {
        match &self.owner {
            ColumnOwner::Encoded(column) => column.pool(),
            ColumnOwner::Rows(rows) => rows.pool(),
            ColumnOwner::Packed(keys) => keys.pool(),
        }
    }
    pub fn bounds(&self) -> Bounds {
        self.bounds
    }
    /// Measure a computed publication when its declared scalar range is not
    /// a usable provider domain, with the recorded `bounds` family geometry.
    /// Only reduction metadata crosses to the host.
    pub fn measure_bounds(&mut self, geometry: Geometry) -> Result<()> {
        geometry.check("bounds", Auxiliary::SUPPORTED.bounds)?;
        // DECODE V: measuring a computed packed publication reconstructs values before provider planning; P6 reuse recorded summaries.
        self.bounds = source_bounds(self.pool(), self.source, self.rows, geometry)?;
        Ok(())
    }
    pub fn rows(&self) -> usize {
        self.rows
    }
    fn view(&self) -> gpu_device_columns::SourceView {
        self.source
    }
}

// DECODE ?: source-bound reduction accepts identity and packed sources; caller representation facts decide whether it decodes.
/// `geometry` is the checked `bounds` family geometry.
fn source_bounds(
    pool: &Arc<Pool>,
    source: gpu_device_columns::SourceView,
    rows: usize,
    geometry: Geometry,
) -> Result<Bounds> {
    if rows == 0 {
        return Ok(Bounds::Empty);
    }
    let rows_u32 = u32::try_from(rows).map_err(|_| "numeric bounds exceed the u32 row domain")?;
    let module = build_module(pool)?;
    let shape = module.prepare_numeric_bounds(geometry.config(1))?;
    let blocks = geometry.grid(shape.function(), rows)?;
    // One (min, max) partial per warp of every launched block.
    let partials = blocks as usize * (geometry.block_threads as usize / 32) * 2;
    let partial = unsafe { Buffer::<i64>::uninitialized(pool.clone(), partials)? };
    // DECODE V: bounds discovery launches source reconstruction before index building; P6 consume recorded summaries instead.
    module.numeric_bounds(
        pool.stream(),
        &module.prepare_numeric_bounds(geometry.config(blocks))?,
        source,
        rows_u32,
        partial.ptr(),
    )?;
    let partial = partial.to_host()?;
    let (min, max) = partial
        .chunks_exact(2)
        .fold((i64::MAX, i64::MIN), |(min, max), pair| {
            (min.min(pair[0]), max.max(pair[1]))
        });
    Ok(Bounds::Inclusive { min, max })
}

/// One dense u32 key level, in lexicographic order. Values are translated by
/// the recorded column minimum before indexing; payload values stay unchanged.
#[derive(Clone)]
pub struct KeyDescriptor {
    pub column: ColumnDescriptor,
    pub domain_size: usize,
}

fn key_base(bounds: Bounds) -> i64 {
    match bounds {
        Bounds::Empty => 0,
        Bounds::Inclusive { min, .. } => min,
    }
}

/// Checked directory extent for a possibly signed, dense integer domain.
pub fn dense_key_domain(bounds: Bounds) -> Result<usize> {
    match bounds {
        Bounds::Empty => Ok(0),
        Bounds::Inclusive { min, max } => {
            let span = max
                .checked_sub(key_base(bounds))
                .and_then(|span| span.checked_add(1))
                .filter(|&span| min <= max && span > 0 && span < u32::MAX as i64)
                .ok_or("dense key domain must leave u32::MAX available as a sentinel")?;
            Ok(usize::try_from(span)?)
        }
    }
}

#[cfg(test)]
mod dense_key_tests {
    use super::{dense_key_domain, key_base, Bounds};

    #[test]
    fn radix_fields_use_recorded_value_bits_not_carrier_widths() {
        use gpu_datalog_runtime::Column;
        assert_eq!(super::radix_source_field(Bounds::Inclusive { min: 3, max: 127 }, Column::U32).unwrap(), (7, false));
        assert_eq!(super::radix_source_field(Bounds::Inclusive { min: -128, max: 127 }, Column::I32).unwrap(), (8, true));
        assert_eq!(super::radix_source_field(Bounds::Inclusive { min: -1, max: 0 }, Column::I32).unwrap(), (1, true));
        assert_eq!(super::radix_source_field(Bounds::Empty, Column::U32).unwrap(), (1, false));
        assert!(super::radix_source_field(Bounds::Inclusive { min: -1, max: 4 }, Column::U32).is_err());
        assert!(super::radix_source_field(Bounds::Inclusive { min: 0, max: 1i64 << 40 }, Column::I64).is_err());
    }

    #[test]
    fn domains_translate_from_their_recorded_minimum() {
        for (min, max, base, size) in [
            (-3, -1, -3, 3),
            (-3, 2, -3, 6),
            (i64::MIN, i64::MIN, i64::MIN, 1),
            (7, 100, 7, 94),
        ] {
            let bounds = Bounds::Inclusive { min, max };
            assert_eq!(key_base(bounds), base);
            assert_eq!(dense_key_domain(bounds).unwrap(), size);
        }
        assert_eq!(key_base(Bounds::Empty), 0);
        assert_eq!(dense_key_domain(Bounds::Empty).unwrap(), 0);
    }

    #[test]
    fn unrepresentable_dense_domains_fail_before_key_normalization() {
        for (min, max) in [
            (i64::MIN, i64::MAX),
            (i64::MIN, 0),
            (0, i64::MAX),
            (-1, u32::MAX as i64 - 2),
            (0, u32::MAX as i64 - 1),
            (2, 1),
        ] {
            assert!(dense_key_domain(Bounds::Inclusive { min, max }).is_err());
        }
        assert_eq!(
            dense_key_domain(Bounds::Inclusive {
                min: -1,
                max: u32::MAX as i64 - 3
            })
            .unwrap(),
            u32::MAX as usize - 1,
        );
    }
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
    I64,
}
#[derive(Clone)]
pub struct PayloadDescriptor {
    pub column: ColumnDescriptor,
    pub kind: PayloadKind,
}
#[derive(Clone)]
pub struct RangeDescriptor {
    pub key: Vec<KeyDescriptor>,
    /// Checked allocation size, including the terminal lower-bound entry.
    pub directory_entries: usize,
    /// A half-open predicate applied before sorting, not a post-build filter.
    pub selection: Option<RangeSelection>,
    pub payload: Vec<PayloadDescriptor>,
}

impl RangeDescriptor {
    fn base_rows(&self) -> Result<usize> {
        self.key
            .first()
            .map(|key| &key.column)
            .or_else(|| self.payload.first().map(|payload| &payload.column))
            .or_else(|| self.selection.as_ref().map(|selection| &selection.column))
            .map(|column| column.rows)
            .ok_or_else(|| "range descriptor needs a column establishing its live row count".into())
    }

    fn directory_entries(&self) -> Option<usize> {
        let valid = match self.key.first() {
            None => self.directory_entries == 0,
            Some(key) => self.directory_entries.checked_sub(1) == Some(key.domain_size),
        };
        valid.then_some(self.directory_entries)
    }
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
    pub directory_entries: usize,
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
            directory_entries: descriptor.directory_entries,
            key,
            selection,
            payload,
        })
    }
}

fn radix_source_field(bounds: Bounds, column: gpu_datalog_runtime::Column) -> Result<(u32, bool)> {
    let signed = column.is_signed();
    let bits = match bounds {
        Bounds::Empty => 1,
        Bounds::Inclusive { min, max } if min <= max => {
            if signed {
                let negative = if min < 0 { (!min) as u64 } else { 0 };
                1 + (64 - negative.max(max.max(0) as u64).leading_zeros())
            } else {
                if min < 0 { return Err("unsigned radix key has negative recorded bounds".into()); }
                (64 - (max as u64).leading_zeros()).max(1)
            }
        }
        _ => return Err("radix key has invalid recorded bounds".into()),
    };
    if bits > 32 || bits > column.bits() {
        return Err("recorded radix key exceeds its <=32-bit code carrier".into());
    }
    Ok((bits, signed))
}

/// Formal rule GPU-RadixSourceBuild over resident CQ sources: one radix
/// publication of the key and value descriptor columns, restricted to the
/// recorded half-open selection when present. Every column shares `pool`
/// and the same live row count; keys and lanes follow the caller's recorded
/// schema (`K`, `C`), checked against the layout's types by the runtime.
/// `selection_geometry` is the recorded `selection` family geometry.
pub fn radix_layout<K, V, W, O, C>(
    pool: &Arc<Pool>,
    params: gpu_datalog_runtime::RadixRunParams,
    keys: &[ColumnDescriptor],
    values: &[ColumnDescriptor],
    selection: Option<&RangeSelection>,
    selection_geometry: Geometry,
) -> Result<gpu_datalog_runtime::RadixLayout<K, V, W, O, C>>
where
    K: gpu_datalog_runtime::ScalarTuple,
    V: gpu_datalog_runtime::ScalarTuple,
    W: gpu_datalog_runtime::Word + 'static,
    O: gpu_datalog_runtime::OffsetWord,
    C: gpu_datalog_runtime::SourceItemColumns,
{
    let rows = keys
        .iter()
        .chain(values)
        .map(|column| column.rows)
        .next()
        .ok_or("radix layout needs a source column")?;
    if keys
        .iter()
        .chain(values)
        .any(|column| column.rows != rows || !Arc::ptr_eq(column.pool(), pool))
    {
        return Err("radix source columns must share the pool and live row count".into());
    }
    let selected = selection
        .map(|selection| select(pool.clone(), selection, selection_geometry))
        .transpose()?;
    let (positions, count) = match &selected {
        Some(selected) if selected.is_empty() => (std::ptr::null(), 0),
        Some(selected) => (selected.ptr(), selected.len()),
        None => (std::ptr::null(), rows),
    };
    if keys.len() != K::COLUMNS.len() {
        return Err("radix key descriptors differ from the carrier schema".into());
    }
    let fields = keys.iter().zip(K::COLUMNS).map(|(key, &column)| {
        radix_source_field(key.bounds(), column)
    }).collect::<Result<Vec<_>>>()?;
    let packing = gpu_datalog_runtime::RadixLayout::<K, V, W, O, C>::derive_packing_with_fields(&fields)?;
    let keys = keys.iter().map(ColumnDescriptor::view).collect::<Vec<_>>();
    let values = values
        .iter()
        .map(ColumnDescriptor::view)
        .collect::<Vec<_>>();
    // SAFETY: descriptors own their columns, whose live prefixes are ready on
    // `pool`'s stream (ColumnDescriptor contract); selected positions are
    // produced on the same stream and index that prefix.
    unsafe {
        gpu_datalog_runtime::RadixLayout::from_sources_with_packing(
            pool.clone(),
            params,
            packing,
            &keys,
            &values,
            positions,
            count,
        )
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
    pub base: i64,
    pub len: u32,
}

/// An immutable, duplicate-preserving sorted range index. A keyed lookup is
/// [offsets[key], offsets[key+1]); an empty key denotes [0,len) without a directory.
pub struct RangeProvider {
    offsets: EncodedColumn,
    payload: Vec<EncodedColumn>,
    payload_views: Arc<Buffer<ColumnView>>,
    domain: u32,
    base: i64,
    len: u32,
}
impl RangeProvider {
    /// Construction follows the selected provider, including its reusable workspace ABI.
    pub fn builder<'pool>(pool: &'pool Arc<Pool>, aux: &Auxiliary) -> Result<RangeBuilder<'pool>> {
        RangeBuilder::new(pool, aux)
    }
    pub fn build(pool: Arc<Pool>, descriptor: &RangeDescriptor, aux: &Auxiliary) -> Result<Self> {
        RangeBuilder::new(&pool, aux)?.build(descriptor)
    }
    pub fn from_catalog(
        pool: Arc<Pool>,
        catalog: &Catalog,
        descriptor: &CatalogRangeDescriptor,
        aux: &Auxiliary,
    ) -> Result<Self> {
        Self::build(
            pool,
            &RangeDescriptor::from_catalog(catalog, descriptor)?,
            aux,
        )
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
            base: self.base,
            len: self.len,
        }
    }
}

thread_local! {
    static BUILD_MODULE: RefCell<Option<(Arc<CudaContext>, kernels::build::LoadedModule)>> =
        const { RefCell::new(None) };
    static RADIX_WORKSPACES: RefCell<Vec<RadixWorkspace>> =
        const { RefCell::new(Vec::new()) };
}

/// Release this thread's idle range-sort and grouped-sort device scratch.
///
/// Call after dropping stratum-local providers, before [`Pool::release_unused`].
/// Published results and checked-out providers retain their own allocations.
/// Frees follow prior uses on the matching pool's stream; this does not wait
/// for completion. Pooled library scratch is included in pool memory stats.
pub fn release_cached_scratch(pool: &Arc<Pool>) {
    RADIX_WORKSPACES.with(|cache| {
        cache.borrow_mut().retain(|scratch| !Arc::ptr_eq(&scratch.pool, pool));
    });
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

type PooledRadixWorkspace = rccl_cub::DeviceRadixSortBufferWorkspace<
    rccl_memory::DeviceBuffer<'static, u64, &'static MemoryResource>,
    rccl_memory::DeviceBuffer<'static, i32, &'static MemoryResource>,
    rccl_memory::DeviceBuffer<'static, i32, &'static MemoryResource>,
>;
struct RadixWorkspace {
    rows: usize,
    wide_keys: bool,
    workspace: PooledRadixWorkspace,
    pool: Arc<Pool>,
}

/// Retain grow-only controls for each key width on the active pool.
/// RCCL clears controls before each sort; sharing is stream ordered.
/// Published results never borrow this scratch.
fn with_radix_workspace(
    pool: &Arc<Pool>,
    launch: &rccl_cub::cuda::Launch<'_, '_>,
    rows: usize,
    wide_keys: bool,
    sort: impl FnOnce(&mut PooledRadixWorkspace) -> Result<()>,
) -> Result<()> {
    RADIX_WORKSPACES.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.retain(|scratch| Arc::ptr_eq(&scratch.pool, pool));
        let index = match cache.iter().position(|scratch| {
            scratch.rows >= rows && scratch.wide_keys == wide_keys
        }) {
            Some(index) => {
                let scratch = cache.remove(index);
                cache.push(scratch);
                cache.len() - 1
            }
            None => {
                cache.retain(|scratch| scratch.wide_keys != wide_keys);
                pool.context().bind_to_thread()?;
                // SAFETY: workspace drops before its retained stable-address pool.
                let retained: &'static Pool = unsafe { &*Arc::as_ptr(pool) };
                assert!(std::ptr::eq(launch.stream(), retained.stream().as_ref()));
                let launch: rccl_cub::cuda::Launch<'static, '_> =
                    unsafe { std::mem::transmute(*launch) };
                let workspace = if wide_keys {
                    rccl_cub::device_radix_sort_pairs_buffer_workspace_in::<u64, u32, _>(retained.resource(), &launch, rows)?
                } else {
                    rccl_cub::device_radix_sort_pairs_buffer_workspace_in::<u32, u32, _>(retained.resource(), &launch, rows)?
                };
                cache.push(RadixWorkspace {
                    rows,
                    wide_keys,
                    workspace,
                    pool: pool.clone(),
                });
                cache.len() - 1
            }
        };
        sort(&mut cache[index].workspace)
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
        let rows = descriptor.base_rows()?;
        let entries = descriptor
            .directory_entries()
            .ok_or("range domain overflow")?;
        Ok(Self {
            row_count: Arc::new(Buffer::from_host(pool.clone(), &[rows as u64])?),
            offset_count: Arc::new(Buffer::from_host(pool.clone(), &[entries as u64])?),
            payload_views: Arc::new(unsafe {
                Buffer::uninitialized(pool, descriptor.payload.len())?
            }),
            host_views: Vec::with_capacity(descriptor.payload.len()),
            row_value: rows,
            base_rows: rows,
            entries,
        })
    }
    pub fn matches(&self, pool: &Arc<Pool>, descriptor: &RangeDescriptor) -> bool {
        Arc::ptr_eq(self.row_count.pool(), pool)
            && descriptor.base_rows().ok() == Some(self.base_rows)
            && descriptor.directory_entries() == Some(self.entries)
            && descriptor.payload.len() == self.payload_views.len()
    }
}

/// Reuse loaded build/RCCL programs across providers on the existing pool.
/// Fresh builders also share the named module and radix controls on this thread.
/// Every build launches with the recorded `selection` and `range` geometries.
pub struct RangeBuilder<'pool> {
    pool: &'pool Arc<Pool>,
    module: kernels::build::LoadedModule,
    thrust: rccl_thrust::cuda::Program<'pool>,
    cub: rccl_cub::cuda::Program<'pool>,
    selection: Geometry,
    range: Geometry,
}
impl<'pool> RangeBuilder<'pool> {
    pub fn new(pool: &'pool Arc<Pool>, aux: &Auxiliary) -> Result<Self> {
        aux.validate()?;
        Ok(Self {
            module: build_module(pool)?,
            thrust: rccl_thrust::cuda::Program::load(pool.context())?,
            cub: rccl_cub::cuda::Program::load(pool.context())?,
            pool,
            selection: aux.selection,
            range: aux.range,
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
        let base_rows = descriptor.base_rows()?;
        if base_rows > u32::MAX as usize || descriptor.payload.len() > u32::MAX as usize {
            return Err("range provider exceeds the u32 row/column domain".into());
        }
        let check_column = |column: &ColumnDescriptor| -> Result<()> {
            if column.rows != base_rows || !Arc::ptr_eq(column.pool(), &self.pool) {
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
            match key.column.bounds() {
                Bounds::Empty if base_rows == 0 => {}
                Bounds::Inclusive { min, max }
                    if max >= min && dense_key_domain(key.column.bounds())? <= key.domain_size => {}
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
                PayloadKind::I64 => (i64::MIN, i64::MAX),
            };
            match payload.column.bounds() {
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
                let flags = temporary::<u32>(pool, base_rows)?;
                let shape = module.prepare_range_flags(self.selection.config(1))?;
                let blocks = self.selection.grid(shape.function(), base_rows)?;
                // DECODE V: index selection launch reconstructs values before range comparison; P6 translate predicates to codes.
                module.range_flags(
                    stream,
                    &module.prepare_range_flags(self.selection.config(blocks))?,
                    selection.column.view(),
                    base_rows as u32,
                    selection.start,
                    selection.end,
                    pointer(&flags),
                )?;
                let mut selected = temporary::<u32>(pool, base_rows)?;
                let mut count = temporary::<u64>(pool, 1)?;
                select_flagged(
                    &exec,
                    arithmetic_range(0_u32, 1, base_rows),
                    &flags,
                    &mut selected,
                    &mut count,
                )?;
                let rows = count.to_host_vec()?[0] as usize;
                release(stream, flags)?;
                release(stream, count)?;
                (rows, Some(selected))
            }
            _ => (base_rows, None),
        };
        let entries = workspace.entries;
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
        if descriptor.key.is_empty() {
            let source = selected
                .as_ref()
                .map_or(std::ptr::null(), |selected| pointer(selected) as *const u32);
            let payload = descriptor
                .payload
                .iter()
                .map(|payload| unsafe {
                    // DECODE I: keyless payload publication decodes then FOR-reencodes selected rows; avoidable reorganization, P4 keep index payloads plain.
                    pack_payload(pool, payload, source, rows, row_count.clone())
                })
                .collect::<Result<Vec<_>>>()?;
            let offsets = unsafe {
                EncodedColumn::from_device_with_bounds(
                    pool.clone(),
                    std::ptr::null::<i64>(),
                    0,
                    offset_count,
                    0,
                    Codec::For,
                    Bounds::Empty,
                )?
            };
            if let Some(selected) = selected {
                release(stream, selected)?;
            }
            return finish(offsets, payload, 0, 0, rows, workspace);
        }
        let first = &descriptor.key[0];
        // Empty selections still own a complete all-zero directory.
        if rows == 0 {
            let zeros = Buffer::<i64>::zeroed(pool.clone(), entries)?;
            let offsets = unsafe {
                EncodedColumn::from_device_with_bounds(
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
            return finish(
                offsets,
                payload,
                first.domain_size,
                key_base(first.column.bounds()),
                rows,
                workspace,
            );
        }
        let mut keys = temporary::<u32>(pool, rows)?;
        let mut order = temporary::<u32>(pool, rows)?;
        let source = selected
            .as_ref()
            .map_or(std::ptr::null(), |selected| pointer(selected) as *const u32);
        let shape = module.prepare_keys_and_rows(self.range.config(1))?;
        let blocks = self.range.grid(shape.function(), rows)?;
        let keys_prepared = module.prepare_keys_and_rows(self.range.config(blocks))?;
        if descriptor.key.len() > 1 {
            let shape = module.prepare_sequence_decode_column(self.range.config(1))?;
            let blocks = self.range.grid(shape.function(), rows)?;
            let decode_prepared =
                module.prepare_sequence_decode_column(self.range.config(blocks))?;
            let plain = descriptor
                .key
                .iter()
                .map(|key| {
                    let out = temporary::<u32>(pool, rows)?;
                    // DECODE V: multi-key index build launches full column reconstruction before sorting; P6 sort retained encoded fields.
                    module.sequence_decode_column(
                        stream,
                        &decode_prepared,
                        key.column.view(),
                        key_base(key.column.bounds()),
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
            let mut permutation = temporary::<u32>(pool, rows)?;
            sort_columns_lexicographic(
                &exec,
                &plain.iter().collect::<Vec<_>>(),
                &widths,
                &mut permutation,
                None,
            )?;
            let base_order = match &selected {
                Some(selected) => {
                    let mut mapped = temporary::<u32>(pool, rows)?;
                    gather(
                        &exec,
                        selected,
                        &permutation,
                        &mut mapped,
                    )?;
                    release(stream, permutation)?;
                    mapped
                }
                None => permutation,
            };
            // DECODE V: leading-key reconstruction follows tuple sort for directory building; P6 retain encoded key order.
            module.keys_and_rows(
                stream,
                &keys_prepared,
                first.column.view(),
                key_base(first.column.bounds()),
                pointer(&base_order),
                rows as u32,
                pointer(&keys),
                pointer(&order),
            )?;
            for buffer in plain.into_iter().chain([base_order]) {
                release(stream, buffer)?;
            }
        } else {
            // DECODE V: single-key reconstruction precedes radix sort; P6 sort recorded codes or use plain indexes.
            module.keys_and_rows(
                stream,
                &keys_prepared,
                first.column.view(),
                key_base(first.column.bounds()),
                source,
                rows as u32,
                pointer(&keys),
                pointer(&order),
            )?;
            let bits = key_bits(first.domain_size);
            if rows > 1 && bits > 0 {
                let mut alternate_keys = temporary::<u32>(pool, rows)?;
                let mut alternate_order = temporary::<u32>(pool, rows)?;
                let mut pairs = DoubleBuffer::paired(
                    (&mut keys, &mut order),
                    (&mut alternate_keys, &mut alternate_order),
                )
                .ok_or("radix double buffer extents differ")?;
                with_radix_workspace(pool, &launch, rows, false, |workspace| {
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
        let mut offsets = temporary::<u64>(pool, entries)?;
        lower_bound(
            &exec,
            &keys,
            arithmetic_range(0_u32, 1, entries),
            &mut offsets,
        )?;
        let payload = descriptor
            .payload
            .iter()
            .map(|payload| unsafe {
                // DECODE I: sorted index payload publication decodes then FOR-reencodes rows; avoidable reorganization, P4 keep index runs plain.
                pack_payload(pool, payload, pointer(&order), rows, row_count.clone())
            })
            .collect::<Result<Vec<_>>>()?;
        let packed_offsets = unsafe {
            // Lower-bound positions lie in 0..=rows, including empty buckets.
            EncodedColumn::from_device_with_bounds(
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
        finish(
            packed_offsets,
            payload,
            first.domain_size,
            key_base(first.column.bounds()),
            rows,
            workspace,
        )
    }
}

fn finish(
    offsets: EncodedColumn,
    payload: Vec<EncodedColumn>,
    domain: usize,
    base: i64,
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
    let provider = RangeProvider {
        offsets,
        payload,
        payload_views,
        domain: domain as u32,
        base,
        len: len as u32,
    };
    if index_bytes::enabled() {
        let bytes = provider.bytes();
        index_bytes::record("range", bytes, bytes);
    }
    Ok(provider)
}
// DECODE I: payload packing reconstructs source values then emits new FOR frames; avoidable reorganization, P4 keep index runs plain.
unsafe fn pack_payload(
    pool: &Arc<Pool>,
    payload: &PayloadDescriptor,
    order: *const u32,
    rows: usize,
    count: Arc<Buffer<u64>>,
) -> Result<EncodedColumn> {
    // A permutation or subset preserves the source enclosure, not necessarily
    // its extrema. A known empty prefix has no values to enclose.
    let bounds = if rows == 0 { Bounds::Empty } else { payload.column.bounds() };
    match payload.kind {
        PayloadKind::U32 => unsafe {
            // DECODE I: u32 payload gather decodes then re-encodes a permutation; avoidable reorganization, P4 prefer plain indexes.
            EncodedColumn::from_gather_source_with_bounds::<_, u32>(
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
            // DECODE I: i32 payload gather decodes then re-encodes a permutation; avoidable reorganization, P4 prefer plain indexes.
            EncodedColumn::from_gather_source_with_bounds::<_, i32>(
                pool.clone(),
                payload.column.view(),
                order,
                rows,
                count,
                0,
                bounds,
            )
        },
        PayloadKind::I64 => unsafe {
            // DECODE I: i64 payload gather decodes then re-encodes a permutation; avoidable reorganization, P4 prefer plain indexes.
            EncodedColumn::from_gather_source_with_bounds::<_, i64>(
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
fn key_bits(domain: usize) -> u32 {
    if domain <= 1 {
        0
    } else {
        usize::BITS - (domain - 1).leading_zeros()
    }
}
struct TemporaryCache<T: DeviceCopy> {
    buffers: Vec<DeviceBuffer<T>>,
    pool: Arc<Pool>,
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
fn temporary<T: TemporaryElement>(pool: &Arc<Pool>, len: usize) -> Result<DeviceBuffer<T>> {
    T::cache().with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache
            .as_ref()
            .is_none_or(|cache| !Arc::ptr_eq(&cache.pool, pool))
        {
            *cache = Some(TemporaryCache {
                pool: pool.clone(),
                buffers: Vec::new(),
            });
            pool.context().bind_to_thread()?;
        }
        let buffers = &mut cache.as_mut().unwrap().buffers;
        if let Some(index) = buffers.iter().position(|buffer| buffer.len() == len) {
            return Ok(buffers.swap_remove(index));
        }
        // SAFETY: all private callers retain this pool until returned scratch
        // drops or is returned to this cache. Published SelectedRows retains it.
        let retained: &'static Pool = unsafe { &*Arc::as_ptr(pool) };
        Ok(DeviceBuffer::zeroed_in(retained.resource(), retained.stream(), len)?)
    })
}
fn pointer<T: DeviceCopy>(buffer: &DeviceBuffer<T>) -> *mut T {
    buffer.cu_deviceptr() as *mut T
}
fn release<T: TemporaryElement>(stream: &CudaStream, buffer: DeviceBuffer<T>) -> Result<()> {
    T::cache().with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(cache) = cache.as_mut() {
            if cache.pool.stream().cu_stream() == stream.cu_stream()
                && Arc::ptr_eq(cache.pool.context(), stream.context())
            {
                cache.buffers.push(buffer);
                return Ok(());
            }
        }
        drop(buffer);
        Ok(())
    })
}
