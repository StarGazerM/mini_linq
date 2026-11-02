//! Explicit host-encoded S1 uploads. This path has no default compiler selector
//! and never aliases the legacy FastLanes header/codec interpretation.
//!
//! Uploads use pool-owned pinned staging and publish readiness by event; mask
//! membership uploads and launches are ordered on the view's execution lane.
//! Neither path synchronizes the host. The fixed mask kernel contract is 256
//! threads per block, one S1 vector (1024 rows) per tile, no dynamic shared
//! memory, and `min(ceil(live_count / 1024), 568)` blocks. Zero live rows launch
//! nothing; capped grids traverse the remaining tiles by grid stride. The cap
//! is a declared kernel constant, not a device/occupancy-dependent choice.
use crate::{Buffer, Pool, Result};
use cq_storage::scan::format::{CheckedScanView, Domain, SnapshotId};
use cq_storage::scan::{
    BlockHeader, CodeConstraint, CodecId, Family, MinMax, OrdinalDecode, Predicate, ScanColumn,
    TranslatedPredicate, Width, VECTOR_ROWS,
};
use cuda_core::{CudaEvent, DeviceCopy, LaunchConfig1D};
use std::{cell::RefCell, rc::Rc, sync::Arc};

const MASK_TILE_ROWS: usize = VECTOR_ROWS;
const MASK_BLOCK_THREADS: u32 = 256;
const MASK_GRID_CAP: u32 = 568;

fn mask_grid_blocks(live_count: usize) -> u32 {
    live_count
        .div_ceil(MASK_TILE_ROWS)
        .min(MASK_GRID_CAP as usize) as u32
}

thread_local! {
    static MODULE: RefCell<Option<(Arc<cuda_core::CudaContext>, Rc<kernels::shared_scan::LoadedModule>)>> = const { RefCell::new(None) };
}
fn module(pool: &Pool) -> Result<Rc<kernels::shared_scan::LoadedModule>> {
    MODULE.with(|cache| {
        if let Some((context, module)) = cache.borrow().as_ref() {
            if Arc::ptr_eq(context, pool.context()) {
                return Ok(module.clone());
            }
        }
        let module = Rc::new(unsafe { kernels::shared_scan::load(pool.context())? });
        *cache.borrow_mut() = Some((pool.context().clone(), module.clone()));
        Ok(module)
    })
}
pub(crate) fn release_cached_kernels() {
    MODULE.with(|cache| cache.borrow_mut().take());
}
/// Initialized H2D extents, excluding allocator capacity and pinned staging.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransferBytes {
    pub payload: usize,
    pub headers: usize,
    pub block_summaries: usize,
    pub vector_summaries: usize,
}

/// An immutable encoded publication retaining its exact source snapshot, its
/// initialized buffers, and the event recorded after the last upload.
pub struct DeviceScanColumn<T: OrdinalDecode, F: Family> {
    source: Arc<ScanColumn<T, F>>,
    domain: Domain,
    live_count: usize,
    payload: Buffer<u8>,
    headers: Buffer<u8>,
    block_summaries: Option<Buffer<u8>>,
    vector_summaries: Option<Buffer<u8>>,
    ready: CudaEvent,
}
impl<T: OrdinalDecode, F: Family> DeviceScanColumn<T, F> {
    // Formal rule GPU-BlockTransfer: S1 initialized byte extents cross once;
    // publication follows every successful upload and retains source/owners.
    pub fn upload(
        pool: Arc<Pool>,
        source: Arc<ScanColumn<T, F>>,
        domain: Domain,
        live_count: usize,
    ) -> Result<Arc<Self>> {
        let view = source.checked_view(domain, live_count)?;
        let payload = Buffer::from_host_async(pool.clone(), view.payload())?;
        let headers = Buffer::from_host_async(pool.clone(), view.header_bytes())?;
        let upload_summary = |s: Option<&[MinMax]>| -> Result<Option<Buffer<u8>>> {
            s.map(|s| Buffer::from_host_async(pool.clone(), CheckedScanView::summary_bytes(s)))
                .transpose()
        };
        let block_summaries = upload_summary(view.metadata().block_summaries)?;
        let vector_summaries = upload_summary(view.metadata().vector_summaries)?;
        let ready = pool.context().new_event(None)?;
        ready.record(pool.stream())?;
        Ok(Arc::new(Self {
            source,
            domain,
            live_count,
            payload,
            headers,
            block_summaries,
            vector_summaries,
            ready,
        }))
    }
    pub fn snapshot(&self) -> SnapshotId {
        self.source.snapshot()
    }
    pub fn source(&self) -> &Arc<ScanColumn<T, F>> {
        &self.source
    }
    pub fn live_count(&self) -> usize {
        self.live_count
    }
    pub fn capacity(&self) -> usize {
        self.source.len()
    }
    pub fn domain(&self) -> Domain {
        self.domain
    }
    pub fn transfer_bytes(&self) -> TransferBytes {
        TransferBytes {
            payload: self.payload.len(),
            headers: self.headers.len(),
            block_summaries: self.block_summaries.as_ref().map_or(0, |b| b.len()),
            vector_summaries: self.vector_summaries.as_ref().map_or(0, |b| b.len()),
        }
    }
    /// A diagnostic encoded-byte readback, not logical materialization or an
    /// implicit answer adapter. Headers/domain remain in the retained source.
    pub fn download_payload(&self) -> Result<Vec<u8>> {
        self.payload.to_host()
    }

    /// Shared allocation is checked separately from readiness. The view waits
    /// for publication, retains the source, and orders release after lane use.
    pub fn view_on(self: &Arc<Self>, lane: Arc<Pool>) -> Result<DeviceScanView<T, F>> {
        if !self.payload.pool().shares_memory(&lane) {
            return Err("scan view and execution lane must share memory".into());
        }
        let done = lane.context().new_event(None)?;
        lane.stream().wait(&self.ready)?;
        Ok(DeviceScanView {
            owner: self.clone(),
            lane,
            done,
        })
    }
}

/// Safe retained lease. Raw pointers never escape through a safe method.
pub struct DeviceScanView<T: OrdinalDecode, F: Family> {
    owner: Arc<DeviceScanColumn<T, F>>,
    lane: Arc<Pool>,
    done: CudaEvent,
}
impl<T: OrdinalDecode, F: Family> DeviceScanView<T, F> {
    pub fn snapshot(&self) -> SnapshotId {
        self.owner.snapshot()
    }
    pub fn filter_mask(
        &self,
        domain: cq_storage::scan::format::DomainId,
        predicate: &Predicate,
    ) -> Result<Buffer<u8>> {
        if domain != self.owner.domain.id {
            return Err(cq_storage::scan::format::FormatError::Domain.into());
        }
        if matches!(predicate, Predicate::Window { .. }) && !self.owner.domain.order {
            return Err(cq_storage::scan::format::FormatError::Predicate.into());
        }
        let output = unsafe { Buffer::uninitialized(self.lane.clone(), self.owner.live_count)? };
        if self.owner.live_count == 0 {
            return Ok(output);
        }
        // Constants enter once in their declared ordinal domain. Block frames
        // derive saturated residual bounds; no logical Rust inverse is used.
        let global = BlockHeader {
            offset: 0,
            base: 0,
            rows: 0,
            width: Width::W64,
            codec: CodecId::Plain,
            reserved: [0; 2],
        };
        let (kind, lo, hi, keys) = if let Predicate::Membership(keys) = predicate {
            (2, 0, 0, keys.as_slice())
        } else {
            match predicate.translate(&global) {
                TranslatedPredicate::Range(CodeConstraint::Empty) => (0, 0, 0, &[][..]),
                TranslatedPredicate::Range(CodeConstraint::All) => (1, 0, u64::MAX, &[][..]),
                TranslatedPredicate::Range(CodeConstraint::Range { lo, hi }) => {
                    (1, lo, hi, &[][..])
                }
                TranslatedPredicate::Membership { .. } => unreachable!(),
            }
        };
        let key_count = keys.len() as u64;
        let keys = if keys.is_empty() {
            None
        } else {
            Some(Buffer::from_host_async(self.lane.clone(), keys)?)
        };
        let predicate = ScanPredicate {
            kind,
            reserved: 0,
            lo,
            hi,
            keys: keys.as_ref().map_or(std::ptr::null(), |b| b.ptr()),
            key_count,
        };
        let module = module(&self.lane)?;
        let launch = module.prepare_mask(LaunchConfig1D::new(
            mask_grid_blocks(self.owner.live_count),
            MASK_BLOCK_THREADS,
            0,
        ))?;
        unsafe {
            module.mask(
                self.lane.stream(),
                &launch,
                self.abi(),
                predicate,
                output.ptr(),
            )?;
        }
        Ok(output)
    }
    /// # Safety
    /// Launch only on this lease's lane; retain the lease until submission is
    /// complete, never mutate its pointers or inspect beyond live_count. The
    /// descriptor contains S1 headers, not legacy VectorHeader objects.
    pub unsafe fn abi(&self) -> ScanDeviceDescriptor {
        ScanDeviceDescriptor {
            payload: self.owner.payload.ptr(),
            headers: self.owner.headers.ptr().cast(),
            block_summaries: self
                .owner
                .block_summaries
                .as_ref()
                .map_or(std::ptr::null(), |b| b.ptr().cast()),
            vector_summaries: self
                .owner
                .vector_summaries
                .as_ref()
                .map_or(std::ptr::null(), |b| b.ptr().cast()),
            capacity: self.owner.capacity() as u64,
            live_count: self.owner.live_count as u64,
            block_rows: self.owner.source.block_rows().get() as u64,
            snapshot: self.owner.snapshot().get(),
        }
    }
}
impl<T: OrdinalDecode, F: Family> Drop for DeviceScanView<T, F> {
    fn drop(&mut self) {
        let result = self
            .done
            .record(self.lane.stream())
            .and_then(|()| self.owner.payload.pool().stream().wait(&self.done));
        if let Err(error) = result {
            self.lane.context().record_err::<()>(Err(error));
            if let Err(error) = self.lane.stream().synchronize() {
                self.lane.context().record_err::<()>(Err(error));
                // CUDA cannot establish completion. Keep all source/buffer
                // owners rather than racing a possibly live reader with free.
                std::mem::forget(self.owner.clone());
            }
        }
    }
}

/// Pointer/extent realization of the shared cq-storage format, not a second
/// interpretation or codec registry. Construction is via the checked lease.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ScanDeviceDescriptor {
    payload: *const u8,
    headers: *const BlockHeader,
    block_summaries: *const MinMax,
    vector_summaries: *const MinMax,
    capacity: u64,
    live_count: u64,
    block_rows: u64,
    snapshot: u64,
}
unsafe impl DeviceCopy for ScanDeviceDescriptor {}
#[repr(C)]
#[derive(Clone, Copy)]
struct ScanPredicate {
    kind: u32,
    reserved: u32,
    lo: u64,
    hi: u64,
    keys: *const u64,
    key_count: u64,
}
unsafe impl DeviceCopy for ScanPredicate {}

mod kernels {
    #[cuda_device::cuda_module]
    pub(crate) mod shared_scan {
        use super::super::{
            ScanDeviceDescriptor, ScanPredicate, MASK_BLOCK_THREADS, MASK_TILE_ROWS,
        };
        use cuda_device::{device, kernel, launch_bounds, launch_contract, thread};

        #[device]
        unsafe fn intersects(predicate: ScanPredicate, lo: u64, hi: u64) -> bool {
            if predicate.kind == 0 {
                return false;
            }
            if predicate.kind == 1 {
                return predicate.lo <= hi && lo <= predicate.hi;
            }
            let mut i = 0;
            while i < predicate.key_count {
                let key = unsafe { predicate.keys.add(i as usize).read() };
                if lo <= key && key <= hi {
                    return true;
                }
                i += 1;
            }
            false
        }
        #[device]
        unsafe fn matches(predicate: ScanPredicate, base: u64, code: u64) -> bool {
            if predicate.kind == 0 {
                return false;
            }
            if predicate.kind == 1 {
                return predicate.hi >= base
                    && code >= predicate.lo.saturating_sub(base)
                    && code <= predicate.hi - base;
            }
            let mut i = 0;
            while i < predicate.key_count {
                let key = unsafe { predicate.keys.add(i as usize).read() };
                if key >= base && key - base == code {
                    return true;
                }
                i += 1;
            }
            false
        }
        // The module-level contract fixes tile size and the 568-block cap;
        // prepare_mask checks the block shape, and the host realizes the
        // declared grid formula from the live extent without device queries.
        #[kernel]
        #[launch_bounds(256)]
        #[launch_contract(domain = 1, block = (256, 1, 1))]
        pub unsafe fn mask(view: ScanDeviceDescriptor, predicate: ScanPredicate, output: *mut u8) {
            let mut vector = thread::blockIdx_x() as u64;
            let lane = thread::threadIdx_x() as u64;
            while vector < view.live_count.div_ceil(MASK_TILE_ROWS as u64) {
                let start = vector * MASK_TILE_ROWS as u64;
                let block = start / view.block_rows;
                let header = unsafe { view.headers.add(block as usize).read() };
                let rows = (view.live_count - start).min(MASK_TILE_ROWS as u64);
                let mut possible = predicate.kind != 0;
                if possible && !view.block_summaries.is_null() {
                    let bounds = unsafe { view.block_summaries.add(block as usize).read() };
                    possible = unsafe { intersects(predicate, bounds.lo, bounds.hi) };
                }
                if possible && !view.vector_summaries.is_null() {
                    let bounds = unsafe { view.vector_summaries.add(vector as usize).read() };
                    possible = unsafe { intersects(predicate, bounds.lo, bounds.hi) };
                }
                let offset = header.offset + (start % view.block_rows) * header.width as u64;
                // Width dispatch is per vector, not a logical scalar decoder.
                macro_rules! lanes {
                    ($ty:ty) => {{
                        let mut local = lane;
                        while local < rows {
                            let selected = if possible {
                                let code = unsafe {
                                    (view.payload.add(offset as usize) as *const $ty)
                                        .add(local as usize)
                                        .read()
                                } as u64;
                                unsafe { matches(predicate, header.base, code) }
                            } else {
                                false
                            };
                            unsafe {
                                output.add((start + local) as usize).write(selected as u8);
                            }
                            local += MASK_BLOCK_THREADS as u64;
                        }
                    }};
                }
                match header.width as u8 {
                    1 => lanes!(u8),
                    2 => lanes!(u16),
                    4 => lanes!(u32),
                    8 => lanes!(u64),
                    _ => {
                        let selected = possible && unsafe { matches(predicate, header.base, 0) };
                        let mut local = lane;
                        while local < rows {
                            unsafe {
                                output.add((start + local) as usize).write(selected as u8);
                            }
                            local += MASK_BLOCK_THREADS as u64;
                        }
                    }
                }
                vector += thread::gridDim_x() as u64;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_grid_obeys_fixed_kernel_contract() {
        assert_eq!(MASK_TILE_ROWS, 1024);
        assert_eq!(MASK_BLOCK_THREADS, 256);
        assert_eq!(MASK_GRID_CAP, 568);
        for (rows, blocks) in [
            (0, 0),
            (1, 1),
            (1024, 1),
            (1025, 2),
            (568 * 1024 - 1, 568),
            (568 * 1024, 568),
            (568 * 1024 + 1, 568),
            (2 * 568 * 1024 + 17, 568),
            (usize::MAX, 568),
        ] {
            assert_eq!(mask_grid_blocks(rows), blocks, "live rows: {rows}");
        }
    }
}
