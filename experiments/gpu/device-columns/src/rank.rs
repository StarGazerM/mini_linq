//! Device-built union membership and ordered Rank domains.
//!
//! Rank retains one bitmap bit/key-span plus a u32 prefix per bitmap word:
//! 1/4 byte per span position, versus four bytes per distinct key for a u32
//! dictionary. At TPC-H's 4x order-key sparsity this is one byte per distinct
//! key. Offset releases the bitmap and all prefix storage after measurement;
//! its normalization and range validation need only the scalar descriptor.
//! Hierarchical scan totals and each u32 translation buffer are stream-local
//! scratch. No row sorting or deduplication is performed on the host.
//! Final host publication lazily mirrors only the structural index, once per
//! owning domain, and reports that transfer separately from load/query time.

use crate::domain::{Domain, DomainView, use_rank};
use crate::native::{Input, NativeView, ELEMENT_ENCODED};
use crate::{BaseLayout, Bounds, Buffer, Codec, ColumnView, EncodedColumn, Pool, Result};
use cuda_core::{CudaContext, LaunchConfig1D};
use std::{cell::{Cell, OnceCell, RefCell}, rc::Rc, sync::Arc, time::Instant};

#[path = "rank_kernels.rs"]
mod kernels;

struct Mirror {
    bitmap: Vec<u32>,
    directory: Vec<u32>,
}

pub(crate) struct Index {
    bitmap: Buffer<u32>,
    directory: Buffer<u32>,
    mirror: OnceCell<Mirror>,
}

thread_local! {
    static MODULE: RefCell<Option<(Arc<CudaContext>, Rc<kernels::rank_load::LoadedModule>)>> = const { RefCell::new(None) };
    static PUBLICATION: Cell<(usize, f64)> = const { Cell::new((0, 0.0)) };
}

/// Bytes and wall milliseconds spent lazily mirroring structural domain
/// metadata for genuine host Rust reads or final publication since the last take.
pub fn take_host_inverse_report() -> (usize, f64) {
    PUBLICATION.with(|report| report.replace((0, 0.0)))
}

fn module(pool: &Pool) -> Result<Rc<kernels::rank_load::LoadedModule>> {
    MODULE.with(|cache| {
        if let Some((context, module)) = cache.borrow().as_ref() {
            if Arc::ptr_eq(context, pool.context()) { return Ok(module.clone()); }
        }
        let module = Rc::new(unsafe { kernels::rank_load::load(pool.context())? });
        *cache.borrow_mut() = Some((pool.context().clone(), module.clone()));
        Ok(module)
    })
}

pub(crate) fn release_cached_kernels() {
    MODULE.with(|cache| cache.borrow_mut().take());
}

fn encoded_input(column: &EncodedColumn) -> Input {
    Input { column: unsafe { column.view() }, element: ELEMENT_ENCODED, stride: 0 }
}

fn native_input(source: NativeView, rows: usize, stride: usize) -> Input {
    Input {
        column: ColumnView {
            headers: std::ptr::null(), data: source.data.cast(), len: rows as u64,
            count: std::ptr::null(), codec: Codec::Plain as u32,
        },
        element: source.element, stride,
    }
}

impl Domain {
    /// Freeze the union of exactly these bound members. Scalar bounds and
    /// cardinality are the only construction downloads. PK/FK status is irrelevant.
    pub fn record(pool: Arc<Pool>, id: u32, members: &[&EncodedColumn]) -> Result<Self> {
        if members.iter().any(|member| !Arc::ptr_eq(member.pool(), &pool)) {
            return Err("domain members must share the output pool".into());
        }
        let bounds: Vec<_> = members.iter().map(|member| member.bounds()).collect();
        let offset = crate::native::record_domain(pool.clone(), id, &bounds)?;
        if offset.bound == 0 {
            return Ok(Self { id, minimum: 0, bound: 0, bits: 0, offset, index: None, distinct: 0 });
        }
        let bitmap = Buffer::<u32>::zeroed(pool.clone(), (offset.bound as usize).div_ceil(32))?;
        let module = module(&pool)?;
        for member in members {
            if member.capacity() == 0 { continue; }
            let launch = module.prepare_domain_rank_bitmap(crate::config(member.capacity()))?;
            unsafe { module.domain_rank_bitmap(pool.stream(), &launch, encoded_input(member), offset.minimum, bitmap.ptr())?; }
        }
        let directory = unsafe { Buffer::<u32>::uninitialized(pool.clone(), bitmap.len())? };
        let launch = module.prepare_domain_rank_counts(crate::config(directory.len()))?;
        unsafe { module.domain_rank_counts(pool.stream(), &launch, bitmap.ptr(), bitmap.len(), directory.ptr())?; }
        let mut levels = vec![directory];
        loop {
            let current = levels.last().unwrap();
            let next = unsafe { Buffer::<u32>::uninitialized(pool.clone(), current.len().div_ceil(256))? };
            let launch = module.prepare_domain_rank_scan(LaunchConfig1D::new(next.len().min(568) as u32, 256, 1024))?;
            unsafe { module.domain_rank_scan(pool.stream(), &launch, current.ptr(), current.len(), next.ptr())?; }
            let root = next.len() == 1;
            levels.push(next);
            if root { break; }
        }
        let distinct = levels.last().unwrap().to_host()?[0];
        let rank = use_rank(offset.bound, distinct);
        let index = if rank {
            for level in (0..levels.len().saturating_sub(2)).rev() {
                let current = &levels[level];
                let launch = module.prepare_domain_rank_add(crate::config(current.len()))?;
                unsafe { module.domain_rank_add(pool.stream(), &launch, current.ptr(), current.len(), levels[level + 1].ptr())?; }
            }
            Some(Arc::new(Index { bitmap, directory: levels.remove(0), mirror: OnceCell::new() }))
        } else {
            None
        };
        let bound = if rank { distinct } else { offset.bound };
        let bits = (32 - (bound - 1).leading_zeros()).max(1);
        Ok(Self {
            id, minimum: offset.minimum, bound, bits, offset, distinct,
            index,
        })
    }

    pub fn is_rank(&self) -> bool {
        self.index.is_some()
    }
    pub fn distinct(&self) -> u32 { self.distinct }
    pub fn span(&self) -> u32 { self.offset.bound }
    pub fn bounds(&self) -> Bounds {
        if self.bound == 0 { Bounds::Empty } else { Bounds::Inclusive { min: 0, max: i64::from(self.bound - 1) } }
    }
    pub fn bytes(&self) -> usize {
        self.index.as_ref().map_or(0, |index| index.bitmap.bytes() + index.directory.bytes())
    }
    pub fn view(&self) -> DomainView {
        DomainView {
            minimum: self.minimum, bound: self.bound, span: self.offset.bound,
            bitmap: self.index.as_ref().map_or(std::ptr::null(), |index| index.bitmap.ptr().cast_const()),
            directory: self.index.as_ref().map_or(std::ptr::null(), |index| index.directory.ptr().cast_const()),
        }
    }

    /// Rank uses one tiny GPU call for an authored constant; Offset is scalar.
    /// Neither path performs a per-row query operation.
    pub fn lookup(&self, value: i64) -> Result<(u32, u32, u32)> {
        let Some(index) = &self.index else {
            return Ok((self.offset.exact(value), self.offset.lower(value), self.offset.upper(value)));
        };
        let pool = index.bitmap.pool();
        let result = unsafe { Buffer::<u32>::uninitialized(pool.clone(), 3)? };
        let module = module(pool)?;
        let launch = module.prepare_domain_rank_lookup(crate::config(1))?;
        unsafe { module.domain_rank_lookup(pool.stream(), &launch, self.view(), value, result.ptr())?; }
        let result = result.to_host()?;
        Ok((result[0], result[1], result[2]))
    }
    pub fn exact(&self, value: i64) -> u32 { self.lookup(value).expect("domain constant lookup").0 }
    pub fn lower(&self, value: i64) -> u32 {
        if !self.is_rank() { self.offset.lower(value) } else { self.lookup(value).expect("domain constant lookup").1 }
    }
    pub fn upper(&self, value: i64) -> u32 {
        if !self.is_rank() { self.offset.upper(value) } else { self.lookup(value).expect("domain constant lookup").2 }
    }

    /// Host R/P inverse. Rank mirrors structural metadata on first use,
    /// never at load time, and thereafter performs no GPU calls per output row.
    pub fn value(&self, code: u32) -> i64 {
        // DECODE ?: lazy host domain inverse serves checked Rust reads and final answer publication.
        assert!(code < self.bound, "key domain {}: invalid code {code}", self.id);
        if !self.is_rank() { return self.offset.value(code); }
        let index = self.index.as_ref().unwrap();
        let mirror = index.mirror.get_or_init(|| {
            let started = Instant::now();
            let bitmap = index.bitmap.to_host().expect("publication bitmap mirror");
            let directory = index.directory.to_host().expect("publication rank mirror");
            PUBLICATION.with(|report| {
                let (bytes, millis) = report.get();
                report.set((bytes + (bitmap.len() + directory.len()) * 4, millis + started.elapsed().as_secs_f64() * 1000.0));
            });
            Mirror { bitmap, directory }
        });
        let word = mirror.directory.partition_point(|prefix| *prefix <= code) - 1;
        let mut remaining = code - mirror.directory[word];
        let mut bits = mirror.bitmap[word];
        while remaining != 0 { bits &= bits - 1; remaining -= 1; }
        self.minimum + (word * 32 + bits.trailing_zeros() as usize) as i64
    }

    /// Offset changes FOR headers only. Rank translates into u32 device scratch,
    /// FOR-encodes it once, then releases scratch and the original member storage.
    pub fn normalize(&self, column: &mut EncodedColumn) -> Result<()> {
        if column.codec() != Codec::For {
            return Err("key-domain normalization requires FOR headers".into());
        }
        if !self.is_rank() { return column.normalize_key(self.offset); }
        let pool = column.pool().clone();
        let codes = self.translate(pool.clone(), encoded_input(column), column.capacity())?;
        *column = unsafe { EncodedColumn::from_native(pool, NativeView { data: codes.ptr().cast(), element: crate::ELEMENT_U32 }, codes.len(), None, BaseLayout::For)? };
        Ok(())
    }

    fn translate(&self, pool: Arc<Pool>, source: Input, rows: usize) -> Result<Buffer<u32>> {
        if self.index.as_ref().is_some_and(|index| !Arc::ptr_eq(index.bitmap.pool(), &pool)) {
            return Err("domain translation must use the domain pool".into());
        }
        let codes = unsafe { Buffer::<u32>::uninitialized(pool.clone(), rows)? };
        if rows == 0 { return Ok(codes); }
        let invalid = Buffer::<u32>::zeroed(pool.clone(), 1)?;
        let module = module(&pool)?;
        let launch = module.prepare_domain_rank_translate(crate::config(rows))?;
        unsafe { module.domain_rank_translate(pool.stream(), &launch, source, self.view(), codes.ptr(), invalid.ptr())?; }
        if invalid.to_host()?[0] != 0 {
            return Err(format!("key domain {}: value outside frozen membership or outside recorded range [{}, {})", self.id, self.minimum, i128::from(self.minimum) + i128::from(self.offset.bound)).into());
        }
        Ok(codes)
    }

    /// Validate uploaded logical keys against Rank membership or the Offset
    /// range, then encode on device. No host key scan is introduced.
    ///
    /// # Safety
    /// Each `source.data + row * stride` must address a valid native lane on this
    /// pool's stream; its allocation must survive submission of the encode.
    pub unsafe fn encode_native(&self, pool: Arc<Pool>, source: NativeView, rows: usize, stride: usize) -> Result<EncodedColumn> {
        if !matches!(source.element, crate::ELEMENT_I32 | crate::ELEMENT_U32 | crate::ELEMENT_I64) {
            return Err("unsupported native domain element type".into());
        }
        if self.is_rank() {
            let codes = self.translate(pool.clone(), native_input(source, rows, stride), rows)?;
            unsafe { EncodedColumn::from_native(pool, NativeView { data: codes.ptr().cast(), element: crate::ELEMENT_U32 }, rows, None, BaseLayout::For) }
        } else {
            let mut column = unsafe { crate::native::load_strided(pool, source, rows, stride, None, BaseLayout::For)? };
            column.normalize_key(self.offset)?;
            Ok(column)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparse_union_constants_normalization_and_inverse() -> Result<()> {
        let pool = Pool::new(0, 0, 0)?;
        let mut primary = EncodedColumn::from_host(pool.clone(), &[-8i64, 9, 9], Codec::For)?;
        let mut foreign = EncodedColumn::from_host(pool.clone(), &[-3i64, 9], Codec::For)?;
        let domain = Domain::record(pool.clone(), 7, &[&primary, &foreign])?;
        assert!(domain.is_rank());
        assert_eq!((domain.minimum, domain.span(), domain.bound, domain.bits), (-8, 18, 3, 2));
        for (value, expected) in [(-9, (u32::MAX, 0, 0)), (-8, (0, 0, 1)), (-4, (u32::MAX, 1, 1)), (-3, (1, 1, 2)), (9, (2, 2, 3)), (10, (u32::MAX, 3, 3)), (i64::MIN, (u32::MAX, 0, 0)), (i64::MAX, (u32::MAX, 3, 3))] {
            assert_eq!(domain.lookup(value)?, expected);
        }
        domain.normalize(&mut primary)?;
        domain.normalize(&mut foreign)?;
        assert_eq!(primary.to_host()?, [0, 2, 2]);
        assert_eq!(foreign.to_host()?, [1, 2]);
        take_host_inverse_report();
        let retained = domain.clone();
        drop(domain);
        assert_eq!((retained.value(0), retained.value(1), retained.value(2)), (-8, -3, 9));
        assert_eq!(take_host_inverse_report().0, retained.bytes());
        assert_eq!(retained.value(1), -3);
        assert_eq!(take_host_inverse_report().0, 0);
        Ok(())
    }

    #[test]
    fn empty_singleton_threshold_and_frozen_holes() -> Result<()> {
        let pool = Pool::new(0, 0, 0)?;
        for (values, rank, bound) in [(&[][..], false, 0), (&[-9i64][..], false, 1), (&[0, 1, 3][..], false, 4), (&[0, 1, 5][..], true, 3)] {
            let mut column = EncodedColumn::from_host(pool.clone(), values, Codec::For)?;
            let domain = Domain::record(pool.clone(), 2, &[&column])?;
            assert_eq!((domain.is_rank(), domain.bound), (rank, bound));
            domain.normalize(&mut column)?;
            let restored: Vec<_> = column.to_host()?.into_iter().map(|code| domain.value(code as u32)).collect();
            assert_eq!(restored, values);
            if !rank { assert_eq!(domain.bytes(), 0); }
            if values == [0, 1, 3] {
                assert_eq!(domain.lookup(2)?, (2, 2, 3));
                let hole = Buffer::from_host(pool.clone(), &[2i64])?;
                let uploaded = unsafe { domain.encode_native(pool.clone(), NativeView { data: hole.ptr().cast(), element: crate::ELEMENT_I64 }, 1, 8) }?;
                assert_eq!(uploaded.to_host()?, [2]);
                let outside = Buffer::from_host(pool.clone(), &[4i64])?;
                assert!(unsafe { domain.encode_native(pool.clone(), NativeView { data: outside.ptr().cast(), element: crate::ELEMENT_I64 }, 1, 8) }.is_err());
            }
        }
        Ok(())
    }

    #[test]
    fn rank_directory_crosses_scan_levels_and_empty_blocks() -> Result<()> {
        let pool = Pool::new(0, 0, 0)?;
        let values = [-100i64, -69, -68, 155, 156, 65_500, 200_000];
        let mut column = EncodedColumn::from_host(pool.clone(), &values, Codec::For)?;
        let domain = Domain::record(pool, 4, &[&column])?;
        domain.normalize(&mut column)?;
        assert_eq!(column.to_host()?, [0, 1, 2, 3, 4, 5, 6]);
        for (code, value) in values.into_iter().enumerate() {
            assert_eq!(domain.exact(value), code as u32);
            assert_eq!(domain.value(code as u32), value);
        }
        Ok(())
    }
}
