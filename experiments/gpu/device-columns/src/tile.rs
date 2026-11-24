//! Physical-order scans over the resident FastLanes layout (not S1).
//! A block owns one vector; lane `l` owns `l + k * 256`, for `k < 4`.
//! Prepare frames and windows outside the lane-position loop. No helper uses
//! collectives, so dead lanes may skip predicate and survivor payload reads.
use crate::{logical_len, original_position, transposed_position, Codec, ColumnView};
use cuda_device::device;

pub const BLOCK_THREADS: usize = 256;

/// Payload access is injectable without changing header access or layout.
pub trait PayloadRead: Copy {
    /// # Safety
    /// `address` must name a readable payload word.
    unsafe fn load(self, address: *const u64) -> u64;
}

#[derive(Clone, Copy)]
pub struct RawPayload;
impl PayloadRead for RawPayload {
    #[inline(always)]
    unsafe fn load(self, address: *const u64) -> u64 {
        unsafe { address.read() }
    }
}

/// Instrument the same payload path; the counter counts u64 memory loads,
/// including both words when a packed code straddles a word boundary.
#[derive(Clone, Copy)]
pub struct CountingPayload {
    pub loads: *mut u64,
}

#[device]
#[inline(always)]
unsafe fn count_load(loads: *mut u64) {
    use cuda_device::atomic::{AtomicOrdering, DeviceAtomicU64};
    unsafe { DeviceAtomicU64::from_ptr(loads).fetch_add(1, AtomicOrdering::Relaxed); }
}

// Trait methods remain ordinary inline Rust: #[device] emits free-function
// wrappers; monomorphized callers bring these implementations onto the device.
impl PayloadRead for CountingPayload {
    #[inline(always)]
    unsafe fn load(self, address: *const u64) -> u64 {
        unsafe {
            count_load(self.loads);
            address.read()
        }
    }
}

/// Per-vector header snapshot. Native columns use the same physical row order
/// as encoded columns, even though their backing allocation is logical-order.
#[derive(Clone, Copy)]
pub struct Frame {
    data: *const u64,
    base: i64,
    width: u32,
    codec: u32,
    rows: usize,
    native: bool,
}

/// Snapshot the uniform header and live extent once, before visiting positions.
/// # Safety
/// The view must remain live and describe a valid resident column.
#[device]
#[inline(always)]
pub unsafe fn frame(view: ColumnView, vector: usize) -> Frame {
    let begin = vector as u64 * 1024;
    let rows = unsafe { logical_len(view) }.saturating_sub(begin).min(1024) as usize;
    if rows == 0 {
        return Frame { data: view.data, base: 0, width: 0, codec: view.codec, rows: 0, native: true };
    }
    if view.headers.is_null() {
        return Frame {
            data: unsafe { view.data.add(begin as usize) },
            base: 0, width: 64, codec: Codec::Plain as u32, rows, native: true,
        };
    }
    let header = unsafe { view.headers.add(vector).read() };
    Frame {
        data: unsafe { view.data.add(header.offset as usize) },
        base: header.base,
        width: header.width,
        codec: header.codec,
        rows: rows.min(header.rows as usize),
        native: false,
    }
}

/// Tail validity is logical, not a prefix of transposed physical positions.
#[device]
#[inline(always)]
pub fn live_mask(frame: Frame, lane: usize) -> u8 {
    if frame.rows == 1024 {
        return 15;
    }
    let mut mask = 0;
    let mut k = 0;
    while k < 4 {
        if original_position(lane + k * BLOCK_THREADS) < frame.rows {
            mask |= 1 << k;
        }
        k += 1;
    }
    mask
}

const EMPTY: u32 = 0;
const ALL: u32 = 1;
const CODES: u32 = 2;
const SIGNED: u32 = 3;
const LOGICAL: u32 = 4;

/// Inclusive code bounds, or a payload-free Empty/All classification.
#[derive(Clone, Copy)]
pub struct Window {
    low: u64,
    high: u64,
    kind: u32,
}

#[device]
#[inline(always)]
fn ordinal(value: i64) -> u64 {
    value as u64 ^ (1u64 << 63)
}

// Formal rule GPU-NarrowCarrier: FOR bounds translate once per vector into
// unsigned residuals; comparisons never reconstruct base + code.
// DESIGN contract8-for-native-carriers
/// Translate signed endpoints without overflowing or losing i64 extremes.
/// Plain compares its stored signed bits; Delta/native use logical fallback.
#[device]
#[inline(always)]
pub fn window(frame: Frame, low: i64, low_inclusive: bool, high: i64, high_inclusive: bool) -> Window {
    let empty = Window { low: 0, high: 0, kind: EMPTY };
    if frame.rows == 0 || (!low_inclusive && low == i64::MAX)
        || (!high_inclusive && high == i64::MIN)
    {
        return empty;
    }
    let low = if low_inclusive { low } else { low + 1 };
    let high = if high_inclusive { high } else { high - 1 };
    if low > high {
        return empty;
    }
    let lo = ordinal(low);
    let hi = ordinal(high);
    if frame.native || frame.codec == Codec::Delta as u32 || frame.codec == Codec::Plain as u32 {
        return Window {
            low: lo,
            high: hi,
            kind: if lo == 0 && hi == u64::MAX { ALL }
                else if frame.native || frame.codec == Codec::Delta as u32 { LOGICAL }
                else { SIGNED },
        };
    }
    // Signed order becomes unsigned order under sign-bit bias. Subtracting
    // biased endpoints is exact even for MIN..MAX (a u64-wide residual).
    let base = ordinal(frame.base);
    let maximum = if frame.width == 64 { u64::MAX } else { (1u64 << frame.width) - 1 };
    let maximum = maximum.min(u64::MAX - base);
    if hi < base || lo.saturating_sub(base) > maximum {
        return empty;
    }
    let low = lo.saturating_sub(base);
    let high = (hi - base).min(maximum);
    Window { low, high, kind: if low == 0 && high == maximum { ALL } else { CODES } }
}

#[device]
#[inline(always)]
unsafe fn unpack<P: PayloadRead>(data: *const u64, width: u32, physical: usize, payload: P) -> u64 {
    if width == 0 {
        return 0;
    }
    let p = physical as u32;
    let bit = (p / 16) * width;
    let word = (bit / 64) * 16 + p % 16;
    let shift = bit % 64;
    let mut value = unsafe { payload.load(data.add(word as usize)) } >> shift;
    if shift + width > 64 {
        value |= unsafe { payload.load(data.add((word + 16) as usize)) } << (64 - shift);
    }
    if width < 64 {
        value &= (1u64 << width) - 1;
    }
    value
}

#[device]
#[inline(always)]
// DECODE ?: Delta/native fallback serves both tile predicates and authorized survivor reads.
unsafe fn fallback<P: PayloadRead>(frame: Frame, physical: usize, payload: P) -> i64 {
    let local = original_position(physical);
    if frame.native {
        return unsafe { payload.load(frame.data.add(local)) } as i64;
    }
    let lane = local / 64;
    let position = local % 64;
    let mut value = unsafe { payload.load(frame.data.add(lane)) };
    if frame.width == 0 {
        return value.wrapping_add((frame.base as u64).wrapping_mul(position as u64)) as i64;
    }
    let packed = unsafe { frame.data.add(16) };
    let mut i = 1;
    while i <= position {
        value = value.wrapping_add(frame.base as u64).wrapping_add(unsafe {
            unpack(packed, frame.width, transposed_position(lane * 64 + i), payload)
        });
        i += 1;
    }
    value as i64
}

// Formal rule GPU-TilePredicate: each result bit is the intersection of the
// incoming live mask and the recorded predicate at the same physical row.
/// Intersect four physical-position predicates with the existing alive mask.
/// Empty/All and dead lanes perform no payload access.
/// # Safety
/// `frame` must be live, `lane < 256`, `alive` must be a subset of
/// `live_mask(frame, lane)`, and payload reads must be valid.
#[device]
#[inline(always)]
pub unsafe fn window_lanes<P: PayloadRead>(frame: Frame, lane: usize, alive: u8, window: Window, payload: P) -> u8 {
    if alive == 0 || window.kind == EMPTY {
        return 0;
    }
    if window.kind == ALL {
        return alive;
    }
    let mut result = 0;
    let mut k = 0;
    while k < 4 {
        if alive & (1 << k) != 0 {
            let physical = lane + k * BLOCK_THREADS;
            let code = if window.kind == LOGICAL {
                // DECODE V: P6 Delta/native predicate inverse reconstructs logical values instead of comparing codes.
                ordinal(unsafe { fallback(frame, physical, payload) })
            } else {
                let code = unsafe { unpack(frame.data, frame.width, physical, payload) };
                if window.kind == SIGNED { code ^ (1u64 << 63) } else { code }
            };
            if window.low <= code && code <= window.high {
                result |= 1 << k;
            }
        }
        k += 1;
    }
    result
}

/// Reconstruct only a surviving physical position, at its recorded first use.
/// # Safety
/// The position must be live and the consumer must authorize logical reading.
#[device]
#[inline(always)]
// DECODE ?: survivor inverse is shared by recorded opaque reads and residual or aggregate consumers.
pub unsafe fn read<P: PayloadRead>(frame: Frame, physical: usize, payload: P) -> i64 {
    if frame.native || frame.codec == Codec::Delta as u32 {
        unsafe { fallback(frame, physical, payload) }
    } else {
        (frame.base as u64).wrapping_add(unsafe {
            unpack(frame.data, frame.width, physical, payload)
        }) as i64
    }
}
