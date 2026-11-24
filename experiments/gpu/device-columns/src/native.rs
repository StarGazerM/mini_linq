//! GPU-first loading of native I32/U32/I64 storage or selected encoded rows.
//! Selection is read directly by the encoder without a full decoded buffer.
//! The only downloads are load-time scalar facts.

use crate::{
    BaseLayout, Bounds, Buffer, Codec, ColumnData, ColumnView, EncodedColumn, Pool, Result,
    VectorHeader, ELEMENT_I32, ELEMENT_I64, ELEMENT_U32,
};
use cuda_core::{CudaContext, DeviceCopy, LaunchConfig1D};
use std::{cell::RefCell, rc::Rc, sync::Arc};

#[path = "native_kernels.rs"]
mod kernels;

/// Non-owning contiguous native lane descriptor. Its allocation must remain
/// alive and immutable until all consuming work on the pool's stream completes.
/// `element` is one of `ELEMENT_I32`, `ELEMENT_U32`, or `ELEMENT_I64`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NativeView {
    pub data: *const u8,
    pub element: u32,
}
unsafe impl DeviceCopy for NativeView {}

pub(crate) const ELEMENT_ENCODED: u32 = u32::MAX;

/// Internal tagged input: native lanes use `column.data` and `element`;
/// encoded lanes use the complete column descriptor.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Input {
    pub column: ColumnView,
    pub element: u32,
    pub stride: usize,
}
unsafe impl DeviceCopy for Input {}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Summary {
    pub min: i64,
    pub max: i64,
    pub delta_min: i64,
    pub delta_max: i64,
    pub descending: u64,
    pub invalid: u64,
}
unsafe impl DeviceCopy for Summary {}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Facts {
    pub min: i64,
    pub max: i64,
    pub codec: u32,
    pub invalid: u32,
}
unsafe impl DeviceCopy for Facts {}

thread_local! {
    static MODULE: RefCell<Option<(Arc<CudaContext>, Rc<kernels::native_load::LoadedModule>)>> = const { RefCell::new(None) };
}
fn module(pool: &Pool) -> Result<Rc<kernels::native_load::LoadedModule>> {
    MODULE.with(|cache| {
        if let Some((context, module)) = cache.borrow().as_ref() {
            if Arc::ptr_eq(context, pool.context()) {
                return Ok(module.clone());
            }
        }
        let module = Rc::new(unsafe { kernels::native_load::load(pool.context())? });
        *cache.borrow_mut() = Some((pool.context().clone(), module.clone()));
        Ok(module)
    })
}
pub(crate) fn release_cached_kernels() {
    MODULE.with(|cache| cache.borrow_mut().take());
}

// Both scalar downloads share one stream synchronization; no per-vector facts
// or values are transferred to the host.
fn download_facts(facts: &Buffer<Facts>, extent: Option<&Buffer<u64>>) -> Result<(Facts, usize)> {
    let pool = facts.pool();
    let mut host = Facts { min: 0, max: 0, codec: 0, invalid: 0 };
    let mut words = 0u64;
    unsafe {
        cuda_core::simt::memory::memcpy_dtoh_async(
            &mut host,
            facts.cu_deviceptr(),
            std::mem::size_of::<Facts>(),
            pool.stream().cu_stream(),
        )?;
        if let Some(extent) = extent {
            cuda_core::simt::memory::memcpy_dtoh_async(
                &mut words,
                extent.cu_deviceptr(),
                std::mem::size_of::<u64>(),
                pool.stream().cu_stream(),
            )?;
        }
    }
    pool.stream().synchronize()?;
    if host.invalid != 0 {
        return Err("selected position exceeds source logical rows".into());
    }
    Ok((host, usize::try_from(words)?))
}

/// Merge only scalar member summaries on device; no rows cross the host.
pub fn record_domain(pool: Arc<Pool>, id: u32, members: &[Bounds]) -> Result<crate::KeyDomain> {
    let members: Vec<_> = members.iter().map(|bounds| match *bounds {
        Bounds::Empty => Facts { min: 0, max: 0, codec: 0, invalid: 1 },
        Bounds::Inclusive { min, max } => Facts { min, max, codec: 0, invalid: 0 },
    }).collect();
    let source = Buffer::from_host(pool.clone(), &members)?;
    let result = unsafe { Buffer::<Facts>::uninitialized(pool.clone(), 1)? };
    let module = module(&pool)?;
    let launch = module.prepare_domain_bounds(crate::config(1))?;
    unsafe { module.domain_bounds(pool.stream(), &launch, source.ptr(), members.len(), result.ptr())?; }
    let (facts, _) = download_facts(&result, None)?;
    crate::KeyDomain::from_bounds(id, if facts.codec != 0 { Bounds::Empty } else {
        Bounds::Inclusive { min: facts.min, max: facts.max }
    })
}

impl EncodedColumn {
    /// Normalize an exclusively owned FOR column by changing header bases only.
    /// Packing, offsets and payload remain byte-for-byte unchanged.
    pub fn normalize_key(&mut self, domain: crate::KeyDomain) -> Result<()> {
        if self.codec != Codec::For {
            return Err("key-domain normalization requires FOR headers; delta/plain64/native key layouts are unsupported".into());
        }
        let bounds = match self.bounds {
            Bounds::Empty => Bounds::Empty,
            Bounds::Inclusive { min, max } => {
                let low = domain.exact(min);
                let high = domain.exact(max);
                if low == u32::MAX || high == u32::MAX {
                    return Err(format!("key domain {}: column range [{min}, {max}] outside recorded range [{}, {})",
                        domain.id, domain.minimum, i128::from(domain.minimum) + i128::from(domain.bound)).into());
                }
                Bounds::Inclusive { min: i64::from(low), max: i64::from(high) }
            }
        };
        if self.capacity != 0 && domain.minimum != 0 {
            let pool = self.pool();
            let module = module(pool)?;
            let headers = self.headers.as_ref().ok_or("key-domain normalization requires FOR headers")?;
            let launch = module.prepare_domain_rebase(crate::config(headers.len()))?;
            unsafe {
                module.domain_rebase(pool.stream(), &launch, headers.ptr(), headers.len(), domain.minimum)?;
            }
        }
        self.bounds = bounds;
        Ok(())
    }
}

pub(crate) unsafe fn load(
    pool: Arc<Pool>,
    source: NativeView,
    source_rows: usize,
    positions: Option<&Buffer<u32>>,
    layout: BaseLayout,
) -> Result<EncodedColumn> {
    let stride = if source.element == ELEMENT_I64 { 8 } else { 4 };
    unsafe { load_strided(pool, source, source_rows, stride, positions, layout) }
}

/// Encode a native field directly from uploaded row storage.
///
/// # Safety
/// `source.data + row * stride` must identify a valid lane for every source
/// row, and the source must remain alive until this stream finishes encoding.
pub unsafe fn load_strided(
    pool: Arc<Pool>,
    source: NativeView,
    source_rows: usize,
    stride: usize,
    positions: Option<&Buffer<u32>>,
    layout: BaseLayout,
) -> Result<EncodedColumn> {
    if layout == BaseLayout::HostFor {
        return Err("HostFor requires host values; native device loading cannot host-pack".into());
    }
    if !matches!(source.element, ELEMENT_I32 | ELEMENT_U32 | ELEMENT_I64) {
        return Err("unsupported native element type".into());
    }
    if positions.is_some_and(|positions| !Arc::ptr_eq(&pool, positions.pool())) {
        return Err("native positions and output must share a pool".into());
    }
    let input = Input {
        column: ColumnView {
            headers: std::ptr::null(),
            data: source.data.cast(),
            len: source_rows as u64,
            count: std::ptr::null(),
            codec: Codec::Plain as u32,
        },
        element: source.element,
        stride,
    };
    unsafe { encode(pool, input, source_rows, positions, layout) }
}

pub(crate) fn selected(
    pool: Arc<Pool>,
    source: &EncodedColumn,
    positions: &Buffer<u32>,
    layout: BaseLayout,
) -> Result<EncodedColumn> {
    if !Arc::ptr_eq(&pool, source.pool()) {
        return Err("selected source and output must share a pool".into());
    }
    if !Arc::ptr_eq(&pool, positions.pool()) {
        return Err("selected positions and output must share a pool".into());
    }
    let layout = if layout == BaseLayout::HostFor { BaseLayout::For } else { layout };
    // The borrow retains source/count ownership through submission. Both input
    // owners free on this same stream, after every consuming kernel, even if
    // the caller drops them immediately after this function returns.
    let input = Input { column: unsafe { source.view() }, element: ELEMENT_ENCODED, stride: 0 };
    // DECODE I: selected encoded rows are reconstructed only to gather and re-encode resident storage.
    unsafe { encode(pool, input, source.capacity(), Some(positions), layout) }
}

unsafe fn encode(
    pool: Arc<Pool>,
    source: Input,
    source_rows: usize,
    positions: Option<&Buffer<u32>>,
    layout: BaseLayout,
) -> Result<EncodedColumn> {
    let rows = positions.map_or(source_rows, Buffer::len);
    let order = positions.map_or(std::ptr::null(), |positions| positions.ptr().cast_const());
    let plain = matches!(layout, BaseLayout::Plain64 | BaseLayout::Native);
    let default_codec = if plain { Codec::Plain } else { Codec::For };
    if rows == 0 {
        let native = layout == BaseLayout::Native;
        return Ok(EncodedColumn {
            headers: if native { None } else { Some(unsafe { Buffer::uninitialized(pool.clone(), 0)? }) },
            data: if native {
                ColumnData::Plain(unsafe { Buffer::uninitialized(pool.clone(), 0)? })
            } else {
                ColumnData::Packed(unsafe { Buffer::uninitialized(pool.clone(), 0)? })
            },
            extent: if native { None } else { Some(Buffer::zeroed(pool, 1)?) },
            _count_owner: None,
            capacity: 0,
            count: std::ptr::null(),
            codec: default_codec,
            bounds: Bounds::Empty,
        });
    }
    let vectors = rows.div_ceil(1024);
    let summaries = unsafe { Buffer::<Summary>::uninitialized(pool.clone(), vectors)? };
    let facts = unsafe { Buffer::<Facts>::uninitialized(pool.clone(), 1)? };
    let native = if layout == BaseLayout::Native {
        Some(unsafe { Buffer::<i64>::uninitialized(pool.clone(), rows)? })
    } else {
        None
    };
    let module = module(&pool)?;
    let launch = module.prepare_native_summarize(crate::config(vectors * 32))?;
    unsafe {
        module.native_summarize(
            pool.stream(), &launch, source, order, rows,
            (layout == BaseLayout::Delta) as u32,
            native.as_ref().map_or(std::ptr::null_mut(), Buffer::ptr), summaries.ptr(),
        )?;
    }
    let launch = module.prepare_native_reduce(LaunchConfig1D::new(1, 256, 384))?;
    unsafe {
        module.native_reduce(
            pool.stream(), &launch, summaries.ptr(), vectors, rows,
            default_codec as u32, (layout == BaseLayout::Delta) as u32, facts.ptr(),
        )?;
    }
    let (headers, data, extent, host) = if let Some(native) = native {
        let (host, _) = download_facts(&facts, None)?;
        (None, ColumnData::Plain(native), None, host)
    } else {
        let headers = unsafe { Buffer::<VectorHeader>::uninitialized(pool.clone(), vectors)? };
        let launch = module.prepare_native_headers(crate::config(vectors))?;
        unsafe {
            module.native_headers(pool.stream(), &launch, summaries.ptr(), facts.ptr(), rows, headers.ptr())?;
        }
        let extent = EncodedColumn::make_offsets(&headers, rows, std::ptr::null())?;
        let (host, words) = download_facts(&facts, Some(&extent))?;
        let data = unsafe { Buffer::<u64>::uninitialized(pool.clone(), words)? };
        let launch = module.prepare_native_encode(crate::config(vectors * 16))?;
        unsafe {
            module.native_encode(pool.stream(), &launch, source, order, headers.ptr(), vectors, data.ptr())?;
        }
        (Some(headers), ColumnData::Packed(data), Some(extent), host)
    };
    let codec = match host.codec {
        value if value == Codec::Delta as u32 => Codec::Delta,
        value if value == Codec::Plain as u32 => Codec::Plain,
        _ => Codec::For,
    };
    Ok(EncodedColumn {
        headers, data, extent, _count_owner: None, capacity: rows,
        count: std::ptr::null(), codec,
        bounds: Bounds::Inclusive { min: host.min, max: host.max },
    })
}

#[cfg(test)]
mod domain_tests {
    use super::*;

    #[test]
    fn shared_device_minimum_rebases_headers_without_touching_payload() -> Result<()> {
        let pool = Pool::new(0, 0, 0)?;
        let mut left = EncodedColumn::from_host(pool.clone(), &[-8i64, 2, 4], Codec::For)?;
        let mut right = EncodedColumn::from_host(pool.clone(), &[-3i64, 9], Codec::For)?;
        let payload = match &right.data {
            ColumnData::Packed(data) => data.to_host()?,
            _ => unreachable!(),
        };
        let domain = record_domain(pool.clone(), 7, &[left.bounds(), Bounds::Empty, right.bounds()])?;
        assert_eq!((domain.minimum, domain.bound, domain.bits), (-8, 18, 5));
        left.normalize_key(domain)?;
        right.normalize_key(domain)?;
        assert_eq!(left.to_host()?, [0, 10, 12]);
        assert_eq!(right.to_host()?, [5, 17]);
        match &right.data {
            ColumnData::Packed(data) => assert_eq!(data.to_host()?, payload),
            _ => unreachable!(),
        }
        assert_eq!(record_domain(pool, 8, &[Bounds::Empty])?.bounds(), Bounds::Empty);
        Ok(())
    }
}
