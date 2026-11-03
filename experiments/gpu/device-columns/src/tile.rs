//! Cooperative S1 scans: 256 lanes own four lane-strided rows of a 1024-row vector.
use crate::{scan::{DeviceScanView, ScanDeviceDescriptor}, Buffer, Pool, Result};
use cq_storage::scan::{
    BlockHeader, CodeConstraint, CodecId, Family, OrdinalDecode, Predicate,
    TranslatedPredicate, Width,
};
use cuda_core::{DeviceCopy, LaunchConfig1D};
use cuda_device::device;
use std::{cell::RefCell, rc::Rc, sync::Arc};

pub const VECTOR_ROWS: u64 = 1024;
pub const LANES: u32 = 256;
pub const ROWS_PER_LANE: u32 = 4;
pub const DISQUALIFIED: u32 = 0;
pub const QUALIFIED: u32 = 1;
pub const AMBIVALENT: u32 = 2;
const GRID_CAP: u32 = 568;

/// Host-translated inclusive global-ordinal window: kind 0 is empty, kind 1 is [lo, hi].
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub kind: u32,
    pub reserved: u32,
    pub lo: u64,
    pub hi: u64,
}
unsafe impl DeviceCopy for Window {}
impl Window {
    pub const fn empty() -> Window {
        Window { kind: 0, reserved: 0, lo: 0, hi: 0 }
    }
    pub const fn all() -> Window {
        Window { kind: 1, reserved: 0, lo: 0, hi: u64::MAX }
    }
    /// Translate once against a frameless 64-bit header. Membership is not a window.
    pub fn from_predicate(p: &Predicate) -> Result<Window> {
        let header = BlockHeader {
            offset: 0, base: 0, rows: 0, width: Width::W64,
            codec: CodecId::Plain, reserved: [0; 2],
        };
        match p.translate(&header) {
            TranslatedPredicate::Range(CodeConstraint::Empty) => Ok(Self::empty()),
            TranslatedPredicate::Range(CodeConstraint::All) => Ok(Self::all()),
            TranslatedPredicate::Range(CodeConstraint::Range { lo, hi }) => {
                Ok(Window { kind: 1, reserved: 0, lo, hi })
            }
            TranslatedPredicate::Membership { .. } => Err("membership is not a tile window".into()),
        }
    }
}

/// One frame, uniform across the thread block. Construct with `vector`.
#[derive(Clone, Copy)]
pub struct Vector {
    pub codes: *const u8,
    pub base: u64,
    pub width: u32,
    pub rows: u32,
}

/// Device-only element reads; implementations must read exactly one native-width element.
pub trait PayloadRead: Copy {
    unsafe fn u8(self, p: *const u8) -> u8;
    unsafe fn u16(self, p: *const u16) -> u16;
    unsafe fn u32(self, p: *const u32) -> u32;
    unsafe fn u64(self, p: *const u64) -> u64;
}
#[derive(Clone, Copy)]
pub struct RawPayload;
#[derive(Clone, Copy)]
pub struct CountingPayload {
    pub loads: *mut u64,
}

// Trait methods are inline ordinary Rust, reached through the monomorphized
// device helper. #[device] itself emits free-function forwarding wrappers.
macro_rules! payload_reads {
    ($name:ident, $ty:ty) => {
        #[inline(always)]
        unsafe fn $name(self, p: *const $ty) -> $ty {
            unsafe { p.read() }
        }
    };
}
impl PayloadRead for RawPayload {
    payload_reads!(u8, u8);
    payload_reads!(u16, u16);
    payload_reads!(u32, u32);
    payload_reads!(u64, u64);
}
#[device]
#[inline(always)]
unsafe fn count_payload_load(loads: *mut u64) {
    use cuda_device::atomic::{AtomicOrdering, DeviceAtomicU64};
    unsafe { DeviceAtomicU64::from_ptr(loads).fetch_add(1, AtomicOrdering::Relaxed); }
}
macro_rules! counting_reads {
    ($name:ident, $ty:ty) => {
        #[inline(always)]
        unsafe fn $name(self, p: *const $ty) -> $ty {
            unsafe {
                count_payload_load(self.loads);
                p.read()
            }
        }
    };
}
impl PayloadRead for CountingPayload {
    counting_reads!(u8, u8);
    counting_reads!(u16, u16);
    counting_reads!(u32, u32);
    counting_reads!(u64, u64);
}

/// Inclusive residual bounds, clipped to both the native carrier and the ordinal domain.
/// The pinned cuda-macros device expansion retains the ordinary Rust body and
/// forwarding wrapper; without device intrinsics this same function runs on host.
#[device]
#[inline(always)]
pub fn residual_window(w: Window, base: u64, width: u32) -> Option<(u64, u64)> {
    let max = match width {
        0 => 0,
        1 => u8::MAX as u64,
        2 => u16::MAX as u64,
        4 => u32::MAX as u64,
        8 => u64::MAX,
        _ => return None,
    };
    let frame_hi = base.saturating_add(max);
    if w.kind != 1 || w.lo > w.hi || w.hi < base || w.lo > frame_hi {
        None
    } else {
        Some((w.lo.saturating_sub(base), w.hi.min(frame_hi) - base))
    }
}

/// # Safety
/// The descriptor must be from a live S1 lease on the current execution lane.
#[device]
#[inline(always)]
pub unsafe fn vectors(d: &ScanDeviceDescriptor) -> u64 {
    d.live_count.div_ceil(VECTOR_ROWS)
}
/// # Safety
/// `v < vectors(d)`; the checked S1 block geometry is a multiple of VECTOR_ROWS.
#[device]
#[inline(always)]
pub unsafe fn vector(d: &ScanDeviceDescriptor, v: u64) -> Vector {
    let start = v * VECTOR_ROWS;
    let header = unsafe { d.headers.add((start / d.block_rows) as usize).read() };
    let width = header.width as u32;
    let offset = header.offset + (start % d.block_rows) * width as u64;
    Vector {
        codes: if width == 0 { d.payload } else { unsafe { d.payload.add(offset as usize) } },
        base: header.base,
        width,
        rows: (d.live_count - start).min(VECTOR_ROWS) as u32,
    }
}
/// # Safety
/// `lane < LANES`, and `v` is a checked vector.
#[device]
#[inline(always)]
pub unsafe fn live(v: Vector, lane: u32) -> u32 {
    let mut mask = 0;
    let mut k = 0;
    while k < ROWS_PER_LANE {
        if k * LANES + lane < v.rows { mask |= 1 << k; }
        k += 1;
    }
    mask
}
/// Reads only summaries, preferring a vector summary over its enclosing block.
/// # Safety
/// `v < vectors(d)` and the descriptor's summaries belong to its retained snapshot.
#[device]
#[inline(always)]
pub unsafe fn classify(d: &ScanDeviceDescriptor, v: u64, w: Window) -> u32 {
    // Formal rule GPU-VectorSummary: enclosure permits rejection/qualification
    // without touching payload, including a live prefix of a stored vector.
    if w.kind != 1 || w.lo > w.hi { return DISQUALIFIED; }
    let bounds = if !d.vector_summaries.is_null() {
        unsafe { d.vector_summaries.add(v as usize).read() }
    } else if !d.block_summaries.is_null() {
        unsafe { d.block_summaries.add((v * VECTOR_ROWS / d.block_rows) as usize).read() }
    } else {
        return AMBIVALENT;
    };
    if w.hi < bounds.lo || bounds.hi < w.lo {
        DISQUALIFIED
    } else if w.lo <= bounds.lo && bounds.hi <= w.hi {
        QUALIFIED
    } else {
        AMBIVALENT
    }
}
/// Test only the surviving live bits, comparing native residual carriers.
/// # Safety
/// `lane < LANES`, `mask` is a subset of `live(v, lane)`, and the payload is live.
#[device]
pub unsafe fn test<P: PayloadRead>(p: P, v: Vector, lane: u32, w: Window, mask: u32) -> u32 {
    // Formal rule GPU-TilePredicate: one saturated frame translation preserves
    // exactly the input survivor positions matching this inclusive window.
    if mask == 0 { return 0; }
    let Some((lo, hi)) = residual_window(w, v.base, v.width) else { return 0; };
    // Formal rule GPU-NarrowCarrier: dispatch outside the lane loop; loads and
    // comparisons stay u8/u16/u32/u64, without adding the frame base.
    macro_rules! native {
        ($ty:ty, $read:ident) => {{
            let lo = lo as $ty;
            let hi = hi as $ty;
            let mut result = 0;
            let mut k = 0;
            while k < ROWS_PER_LANE {
                let bit = 1 << k;
                if mask & bit != 0 {
                    let code = unsafe { p.$read((v.codes as *const $ty).add((k * LANES + lane) as usize)) };
                    if lo <= code && code <= hi { result |= bit; }
                }
                k += 1;
            }
            result
        }};
    }
    match v.width {
        0 => mask,
        1 => native!(u8, u8),
        2 => native!(u16, u16),
        4 => native!(u32, u32),
        8 => native!(u64, u64),
        _ => 0,
    }
}
/// Read a global ordinal for an explicitly authorized identity or Formula read.
/// # Safety
/// `local < v.rows` and the payload/frame belong to a live checked vector.
#[device]
pub unsafe fn ordinal<P: PayloadRead>(p: P, v: Vector, local: u32) -> u64 {
    let residual = unsafe {
        match v.width {
            0 => 0,
            1 => p.u8(v.codes.add(local as usize)) as u64,
            2 => p.u16((v.codes as *const u16).add(local as usize)) as u64,
            4 => p.u32((v.codes as *const u32).add(local as usize)) as u64,
            8 => p.u64((v.codes as *const u64).add(local as usize)),
            _ => 0,
        }
    };
    v.base + residual
}
/// The logical value of one live row of an authored identity Plain column:
/// the declared ordinal bijection's inverse, read only for its opaque consumer.
/// # Safety
/// As `ordinal`.
#[device]
#[inline(always)]
pub unsafe fn value<T: OrdinalDecode, P: PayloadRead>(p: P, v: Vector, local: u32) -> T {
    // DECODE R: an OpaqueRustRead record authorizes exactly this identity-declared field for its first opaque consumer; frameless Plain ordinals only.
    T::from_ordinal(unsafe { ordinal(p, v, local) })
}

/// Host translation of one closed operand into the column's ordinal domain.
pub fn encode_operand<T: OrdinalDecode>(value: &T) -> u64 {
    cq_storage::OrderKey::ordinal(value)
}

/// The window of an intersection of ordinal bounds `(code, inclusive)`.
pub fn window(lower: &[(u64, bool)], upper: &[(u64, bool)]) -> Result<Window> {
    use std::ops::Bound;
    let bound = |(code, inclusive): (u64, bool)| if inclusive { Bound::Included(code) } else { Bound::Excluded(code) };
    let lo = lower.iter().copied().map(bound).fold(Bound::Unbounded, cq_storage::intersect_lower);
    let hi = upper.iter().copied().map(bound).fold(Bound::Unbounded, cq_storage::intersect_upper);
    Window::from_predicate(&Predicate::Window { lo, hi })
}

/// The window of one ordinal point.
pub fn point(code: u64) -> Result<Window> {
    Window::from_predicate(&Predicate::Point(code))
}

#[cuda_device::cuda_module]
pub mod kernels {
    use super::{ScanDeviceDescriptor, Window, PayloadRead, RawPayload, CountingPayload};
    use super::{vectors, vector, live, classify, test, VECTOR_ROWS, QUALIFIED, AMBIVALENT};
    use cuda_device::{device, kernel, launch_bounds, launch_contract, thread};
    use cuda_device::atomic::{AtomicOrdering, DeviceAtomicU64};

    #[device]
    unsafe fn count<P: PayloadRead>(p: P, view: ScanDeviceDescriptor, window: Window, use_summary: u32, out: *mut u64) {
        let lane = thread::threadIdx_x();
        let mut v = thread::blockIdx_x() as u64;
        let mut total = 0u64;
        while v < unsafe { vectors(&view) } {
            let class = if use_summary != 0 { unsafe { classify(&view, v, window) } } else { AMBIVALENT };
            if class == QUALIFIED {
                if lane == 0 { total += (view.live_count - v * VECTOR_ROWS).min(VECTOR_ROWS); }
            } else if class == AMBIVALENT {
                let frame = unsafe { vector(&view, v) };
                let mask = unsafe { live(frame, lane) };
                total += unsafe { test(p, frame, lane, window, mask) }.count_ones() as u64;
            }
            v += thread::gridDim_x() as u64;
        }
        if total != 0 {
            unsafe { DeviceAtomicU64::from_ptr(out).fetch_add(total, AtomicOrdering::Relaxed); }
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn count_raw(view: ScanDeviceDescriptor, window: Window, use_summary: u32, out: *mut u64) {
        unsafe { count(RawPayload, view, window, use_summary, out); }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn count_counting(view: ScanDeviceDescriptor, window: Window, use_summary: u32, out: *mut u64, loads: *mut u64) {
        unsafe { count(CountingPayload { loads }, view, window, use_summary, out); }
    }
}

fn grid_blocks(live_count: u64) -> u32 {
    live_count.div_ceil(VECTOR_ROWS).min(GRID_CAP as u64) as u32
}
thread_local! {
    static MODULE: RefCell<Option<(Arc<cuda_core::CudaContext>, Rc<kernels::LoadedModule>)>> = const { RefCell::new(None) };
}
fn module(pool: &Pool) -> Result<Rc<kernels::LoadedModule>> {
    MODULE.with(|cache| {
        if let Some((context, module)) = cache.borrow().as_ref() {
            if Arc::ptr_eq(context, pool.context()) { return Ok(module.clone()); }
        }
        let module = Rc::new(unsafe { kernels::load(pool.context())? });
        *cache.borrow_mut() = Some((pool.context().clone(), module.clone()));
        Ok(module)
    })
}
pub(crate) fn release_cached_kernels() {
    MODULE.with(|cache| cache.borrow_mut().take());
}
impl<T: OrdinalDecode, F: Family> DeviceScanView<T, F> {
    /// Queue a count on this view's lane, without host synchronization.
    /// `loads`, if supplied, must be initialized on the same lane and contain at
    /// least one u64. It accumulates element loads; this method does not reset it.
    /// Summary pruning requires stored block or vector summaries.
    pub fn count_matches(&self, window: Window, use_summary: bool, loads: Option<&Buffer<u64>>) -> Result<Buffer<u64>> {
        let view = unsafe { self.abi() };
        if use_summary && !self.has_summaries() {
            return Err("tile summary pruning requires summaries".into());
        }
        if let Some(loads) = loads {
            if loads.is_empty() || !Arc::ptr_eq(loads.pool(), self.lane()) {
                return Err("tile load counter must contain a u64 on the view's lane".into());
            }
        }
        let output = Buffer::zeroed(self.lane().clone(), 1)?;
        let blocks = grid_blocks(view.live_count);
        if blocks == 0 { return Ok(output); }
        let module = module(self.lane())?;
        let config = LaunchConfig1D::new(blocks, LANES, 0);
        unsafe {
            if let Some(loads) = loads {
                let launch = module.prepare_count_counting(config)?;
                module.count_counting(self.lane().stream(), &launch, view, window, use_summary as u32, output.ptr(), loads.ptr())?;
            } else {
                let launch = module.prepare_count_raw(config)?;
                module.count_raw(self.lane().stream(), &launch, view, window, use_summary as u32, output.ptr())?;
            }
        }
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ops::Bound;

    #[test]
    fn tile_grid_obeys_fixed_kernel_contract() {
        assert_eq!(LANES * ROWS_PER_LANE, VECTOR_ROWS as u32);
        for (rows, blocks) in [
            (0, 0), (1, 1), (1024, 1), (1025, 2),
            (568 * 1024 - 1, 568), (568 * 1024, 568),
            (568 * 1024 + 1, 568), (2 * 568 * 1024 + 17, 568), (u64::MAX, 568),
        ] {
            assert_eq!(grid_blocks(rows), blocks);
        }
    }
    #[test]
    fn residual_window_matches_storage_translation() {
        let mut seed = 0x71e5_9a23_609f_d814u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for width in [Width::W0, Width::W8, Width::W16, Width::W32, Width::W64] {
            let max = match width {
                Width::W0 => 0, Width::W8 => u8::MAX as u64,
                Width::W16 => u16::MAX as u64, Width::W32 => u32::MAX as u64,
                Width::W64 => u64::MAX,
            };
            for i in 0..1024 {
                let base = match i { 0 => 0, 1 => u64::MAX, 2 => 1 << 63, _ => next() };
                let header = BlockHeader {
                    offset: 0, base, rows: 1024, width,
                    codec: CodecId::For, reserved: [0; 2],
                };
                let a = next();
                let b = next();
                let predicates = [
                    Predicate::Point(base), Predicate::Point(0), Predicate::Point(u64::MAX),
                    Predicate::Window { lo: Bound::Unbounded, hi: Bound::Unbounded },
                    Predicate::Window { lo: Bound::Excluded(u64::MAX), hi: Bound::Unbounded },
                    Predicate::Window { lo: Bound::Unbounded, hi: Bound::Excluded(0) },
                    Predicate::Window { lo: Bound::Included(a), hi: Bound::Included(b) },
                    Predicate::Window { lo: Bound::Excluded(a.min(b)), hi: Bound::Excluded(a.max(b)) },
                    Predicate::Window { lo: Bound::Included(base), hi: Bound::Included(base.saturating_add(max / 2)) },
                ];
                for predicate in predicates {
                    let expected = match predicate.translate(&header) {
                        TranslatedPredicate::Range(CodeConstraint::Empty) => None,
                        TranslatedPredicate::Range(CodeConstraint::All) => Some((0, max.min(u64::MAX - base))),
                        TranslatedPredicate::Range(CodeConstraint::Range { lo, hi }) => Some((lo, hi)),
                        _ => unreachable!(),
                    };
                    assert_eq!(residual_window(Window::from_predicate(&predicate).unwrap(), base, width as u32), expected);
                }
            }
        }
        assert!(Window::from_predicate(&Predicate::Membership(vec![1])).is_err());
        assert_eq!(residual_window(Window::empty(), 0, 8), None);
        assert_eq!(residual_window(Window::all(), u64::MAX, 8), Some((0, 0)));
    }
}
