//! General hand-written H05 reference over a compressed, resident representation.
//!
//! Base tables are dictionary-encoded (dense u32 IDs per join domain) and FOR
//! bit-packed at load. Each run builds sorted indexes with RCCL (stable radix
//! sort by key, lower_bound for directory offsets), then runs the compiler's
//! recorded R N C O L S program. No key is assumed unique: every lookup
//! returns a range and every range is expanded in full; the closing
//! Supplier probe binds no new column and is an existence test. Counts and
//! offsets are u32; any sum that would exceed u32 sets a device flag and
//! fails the run instead of wrapping.
//!
//! These kernels are representation access (decoding key IDs), the query's
//! Orders selection flags, and the query itself; sorting, compaction, scans
//! and deduplication are RCCL's, and gathered packing is device-columns'
//! `EncodedColumn::from_gather`.

use cuda_device::atomic::{AtomicOrdering, DeviceAtomicU32};
use cuda_device::{device, thread};
use gpu_device_columns::{decode, decode_u32, ColumnView};
use gpu_tiles::owner;
use rccl_cub::__private::block_scan_sum_256;
use rccl_memory::device::BlockSharedMemory;

/// Metadata words (u32), cleared before every query.
pub const META_SEGMENTS: usize = 0;
pub const META_DRIVER_ROWS: usize = 1;
pub const META_CURSOR: usize = 2;
pub const META_EMITTED: usize = 3;
/// Nonzero when a u32 count, offset or capacity bound would be exceeded.
pub const META_OVERFLOW: usize = 4;
pub const META_WORDS: usize = 8;

pub const EXPAND_SHARED_BYTES: u32 = 19552;
/// RCCL block-scan scratch: eight u32 warp aggregates at the shared base.
const SCAN_BYTES: usize = 32;
const SH_TILE: usize = 32;
const SH_BASE: usize = 36;
const P1_BEGIN: usize = 64;
const P1_NATION: usize = 1088;
const P1_OFFSET: usize = 2112;
const P2_BEGIN: usize = 3152;
const P2_NATION: usize = 7248;
const P2_ORDER: usize = 11344;
const P2_OFFSET: usize = 15440;

/// Columns read by the query: the Region base table and the five CSR indexes.
/// `*_offsets` have domain+1 entries; the other columns are index rows in key order.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Indexes {
    /// Region by name: names in sorted order (searched) and their region IDs.
    pub region_name: ColumnView,
    pub region_id: ColumnView,
    pub region_rows: u32,
    /// Nation by region ID.
    pub nation_offsets: ColumnView,
    pub nation_id: ColumnView,
    /// Customer by nation ID.
    pub customer_offsets: ColumnView,
    pub customer_id: ColumnView,
    /// Date-selected Orders by customer ID.
    pub order_offsets: ColumnView,
    pub order_id: ColumnView,
    /// Lineitem by order ID.
    pub line_offsets: ColumnView,
    pub line_supplier: ColumnView,
    /// Supplier by (nation ID, supplier ID): offsets by nation; supplier IDs
    /// ascending within each nation's range.
    pub supplier_offsets: ColumnView,
    pub supplier_id: ColumnView,
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct Constants {
    pub asia: i32,
}

/// Customer ranges of qualifying nations, the virtual driver of the expansion.
/// `offsets` has `capacity + 1` entries.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Segments {
    pub begin: *mut u32,
    pub nation: *mut u32,
    pub offsets: *mut u32,
    pub capacity: u32,
}

/// Answer tuples as packed keys `order << order_shift | supplier << supplier_shift
/// | nation`; the shifts are the dictionary ID widths. Rows beyond `capacity`
/// are counted, never written.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Answers {
    pub keys: *mut u64,
    pub capacity: u32,
    pub order_shift: u32,
    pub supplier_shift: u32,
}

#[device]
#[inline(always)]
unsafe fn range(offsets: ColumnView, key: u32) -> (u32, u32) {
    unsafe { (decode_u32(offsets, key as u64), decode_u32(offsets, key as u64 + 1)) }
}

/// First position in [begin, end) of sorted `column` whose value is not below
/// `value` (`strict`: above `value`).
#[device]
#[inline(always)]
unsafe fn search_i32(column: ColumnView, mut begin: u32, mut end: u32, value: i32, strict: bool) -> u32 {
    while begin < end {
        let middle = begin + (end - begin) / 2;
        let probe = unsafe { decode(column, middle as u64) } as i32;
        if probe < value || (strict && probe == value) {
            begin = middle + 1;
        } else {
            end = middle;
        }
    }
    begin
}

/// Exact membership of `value` in ascending u32 `column` over [begin, end).
#[device]
#[inline(always)]
unsafe fn contains_u32(column: ColumnView, begin: u32, end: u32, value: u32) -> bool {
    let (mut low, mut high) = (begin, end);
    while low < high {
        let middle = low + (high - low) / 2;
        if unsafe { decode_u32(column, middle as u64) } < value {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    low < end && unsafe { decode_u32(column, low as u64) } == value
}

#[device]
#[inline(always)]
unsafe fn flag(meta: *mut u32, word: usize) {
    unsafe { DeviceAtomicU32::from_ptr(meta.add(word)).store(1, AtomicOrdering::Relaxed) };
}

/// RCCL u32 block scan (exclusive, aggregate) that flags instead of wrapping:
/// exactly one thread observes its inclusive prefix wrap whenever the true
/// total reaches 2^32. RCCL's scan ends without a barrier; the trailing one
/// makes its scratch reusable by the next call.
#[device]
#[inline(always)]
unsafe fn checked_scan(value: u32, meta: *mut u32) -> (u32, u32) {
    let shared = BlockSharedMemory::new(SCAN_BYTES);
    let scanned = block_scan_sum_256(value, 256, &shared);
    if scanned.exclusive.wrapping_add(value) < scanned.exclusive {
        unsafe { flag(meta, META_OVERFLOW) };
    }
    thread::sync_threads();
    (scanned.exclusive, scanned.aggregate)
}

#[device]
#[inline(always)]
unsafe fn emit(out: Answers, destination: u32, order: u32, supplier: u32, nation: u32) {
    if destination < out.capacity {
        let key = ((order as u64) << out.order_shift) | ((supplier as u64) << out.supplier_shift) | nation as u64;
        unsafe { *out.keys.add(destination as usize) = key };
    }
}

#[cuda_device::cuda_module]
pub mod reference {
    use super::*;
    use cuda_device::{kernel, launch_bounds, launch_contract, DynamicSharedArray};

    // ---------------------------------------------------------------- index build

    /// Materialize a packed key column as u32 IDs and its row numbers, the
    /// (key, row) pairs RCCL's radix sort orders.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn keys_and_rows(key: ColumnView, rows: u32, keys: *mut u32, order: *mut u32) {
        let mut row = thread::index_1d().get() as u32;
        let stride = thread::gridDim_x() * 256;
        while row < rows {
            unsafe {
                *keys.add(row as usize) = decode_u32(key, row as u64);
                *order.add(row as usize) = row;
            }
            row = match row.checked_add(stride) {
                Some(next) => next,
                None => break,
            };
        }
    }

    /// (key, row) pairs through a row list or permutation `selected[0..rows)`:
    /// a selection's rows, or the order of a previous stable sort pass.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn selected_keys_and_rows(key: ColumnView, selected: *const u32, rows: u32, keys: *mut u32, order: *mut u32) {
        let mut i = thread::index_1d().get() as u32;
        let stride = thread::gridDim_x() * 256;
        while i < rows {
            unsafe {
                let row = *selected.add(i as usize);
                *keys.add(i as usize) = decode_u32(key, row as u64);
                *order.add(i as usize) = row;
            }
            i = match i.checked_add(stride) {
                Some(next) => next,
                None => break,
            };
        }
    }

    /// (value - min as u32, row) pairs of an i32 key: order-preserving.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn signed_keys_and_rows(key: ColumnView, min: i32, rows: u32, keys: *mut u32, order: *mut u32) {
        let mut row = thread::index_1d().get() as u32;
        let stride = thread::gridDim_x() * 256;
        while row < rows {
            unsafe {
                *keys.add(row as usize) = (decode(key, row as u64) as i32).wrapping_sub(min) as u32;
                *order.add(row as usize) = row;
            }
            row = match row.checked_add(stride) {
                Some(next) => next,
                None => break,
            };
        }
    }

    /// The query's Orders selection `date_start <= o_orderdate < date_end`,
    /// as one u32 flag per row for RCCL's flagged select.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn date_flags(date: ColumnView, rows: u32, start: i32, end: i32, flags: *mut u32) {
        let mut row = thread::index_1d().get() as u32;
        let stride = thread::gridDim_x() * 256;
        while row < rows {
            unsafe {
                let value = decode(date, row as u64) as i32;
                *flags.add(row as usize) = (value >= start && value < end) as u32;
            }
            row = match row.checked_add(stride) {
                Some(next) => next,
                None => break,
            };
        }
    }

    // ---------------------------------------------------------------- query

    /// Region by name (probe with the bound ASIA ID) -> Nation by region ->
    /// Customer range by nation. One block. Each (region row, nation row)
    /// pair with customers is a segment; the exclusive prefix of segment
    /// lengths is the driver domain.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn segments(ix: Indexes, constants: Constants, segments: Segments, meta: *mut u32) {
        let tid = thread::threadIdx_x();
        unsafe {
            let count = DeviceAtomicU32::from_ptr(meta.add(META_SEGMENTS));
            let first_region = search_i32(ix.region_name, 0, ix.region_rows, constants.asia, false);
            let last_region = search_i32(ix.region_name, first_region, ix.region_rows, constants.asia, true);
            let mut row = first_region + tid;
            while row < last_region {
                let region = decode_u32(ix.region_id, row as u64);
                let (first, last) = range(ix.nation_offsets, region);
                let mut nation_row = first;
                while nation_row < last {
                    let nation = decode_u32(ix.nation_id, nation_row as u64);
                    let (begin, end) = range(ix.customer_offsets, nation);
                    if end > begin {
                        let slot = count.fetch_add(1, AtomicOrdering::Relaxed);
                        if slot < segments.capacity {
                            *segments.begin.add(slot as usize) = begin;
                            *segments.nation.add(slot as usize) = nation;
                            *segments.offsets.add(slot as usize) = end - begin;
                        } else {
                            flag(meta, META_OVERFLOW);
                        }
                    }
                    nation_row += 1;
                }
                row += 256;
            }
            thread::sync_threads();
            if tid == 0 {
                let n = count.load(AtomicOrdering::Relaxed).min(segments.capacity);
                let mut sum = 0u32;
                let mut i = 0;
                while i < n {
                    let length = *segments.offsets.add(i as usize);
                    *segments.offsets.add(i as usize) = sum;
                    match sum.checked_add(length) {
                        Some(next) => sum = next,
                        None => flag(meta, META_OVERFLOW),
                    }
                    i += 1;
                }
                *segments.offsets.add(n as usize) = sum;
                count.store(n, AtomicOrdering::Relaxed);
                *meta.add(META_DRIVER_ROWS) = sum;
            }
        }
    }

    /// Customer positions -> date-selected Orders by customer -> Lineitem by
    /// order -> Supplier existence by (nation, supplier). Blocks pull
    /// 256-position driver tiles; nested ranges are expanded in 1024-position
    /// sub-tiles.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1), dynamic_shared_range = (19552, 19552))]
    pub fn expand(ix: Indexes, segments: Segments, out: Answers, meta: *mut u32) {
        let tid = thread::threadIdx_x() as usize;
        unsafe {
            let shared = DynamicSharedArray::<u8, 16>::get_raw();
            let tile_slot = shared.add(SH_TILE) as *mut u32;
            let base_slot = shared.add(SH_BASE) as *mut u32;
            let p1_begin = shared.add(P1_BEGIN) as *mut u32;
            let p1_nation = shared.add(P1_NATION) as *mut u32;
            let p1_offset = shared.add(P1_OFFSET) as *mut u32;
            let p2_begin = shared.add(P2_BEGIN) as *mut u32;
            let p2_nation = shared.add(P2_NATION) as *mut u32;
            let p2_order = shared.add(P2_ORDER) as *mut u32;
            let p2_offset = shared.add(P2_OFFSET) as *mut u32;
            let segment_count = *meta.add(META_SEGMENTS);
            let driver_rows = *meta.add(META_DRIVER_ROWS);
            let tiles = driver_rows.div_ceil(256);
            let cursor = DeviceAtomicU32::from_ptr(meta.add(META_CURSOR));
            let emitted = DeviceAtomicU32::from_ptr(meta.add(META_EMITTED));
            loop {
                if tid == 0 {
                    *tile_slot = cursor.fetch_add(1, AtomicOrdering::Relaxed);
                }
                thread::sync_threads();
                let tile = *tile_slot;
                if tile >= tiles {
                    break;
                }
                // Level C: driver position -> Customer row -> Orders range.
                let position = tile * 256 + tid as u32;
                let mut count = 0u32;
                let mut begin = 0u32;
                let mut nation = 0u32;
                if position < driver_rows {
                    let segment = owner(segments.offsets, segment_count, position) as usize;
                    let customer_row = *segments.begin.add(segment) + (position - *segments.offsets.add(segment));
                    nation = *segments.nation.add(segment);
                    let customer = decode_u32(ix.customer_id, customer_row as u64);
                    let (order_begin, order_end) = range(ix.order_offsets, customer);
                    begin = order_begin;
                    count = order_end - order_begin;
                }
                *p1_begin.add(tid) = begin;
                *p1_nation.add(tid) = nation;
                let (offset, orders) = checked_scan(count, meta);
                *p1_offset.add(tid) = offset;
                if tid == 0 {
                    *p1_offset.add(256) = orders;
                }
                thread::sync_threads();
                // Level O: date-selected Orders rows -> Lineitem range.
                let mut order_base = 0u32;
                while order_base < orders {
                    let mut lines = 0u32;
                    let mut stripe = 0usize;
                    while stripe < 4 {
                        let slot = stripe * 256 + tid;
                        let q = order_base.wrapping_add(slot as u32);
                        let mut count = 0u32;
                        let mut begin = 0u32;
                        let mut nation = 0u32;
                        let mut order = 0u32;
                        if q < orders && q >= order_base {
                            let parent = owner(p1_offset, 256, q) as usize;
                            let order_row = *p1_begin.add(parent) + (q - *p1_offset.add(parent));
                            order = decode_u32(ix.order_id, order_row as u64);
                            let (line_begin, line_end) = range(ix.line_offsets, order);
                            begin = line_begin;
                            count = line_end - line_begin;
                            nation = *p1_nation.add(parent);
                        }
                        *p2_begin.add(slot) = begin;
                        *p2_nation.add(slot) = nation;
                        *p2_order.add(slot) = order;
                        let (offset, total) = checked_scan(count, meta);
                        *p2_offset.add(slot) = lines.wrapping_add(offset);
                        let next = lines.wrapping_add(total);
                        if next < lines && tid == 0 {
                            flag(meta, META_OVERFLOW);
                        }
                        lines = next;
                        stripe += 1;
                    }
                    if tid == 0 {
                        *p2_offset.add(1024) = lines;
                    }
                    thread::sync_threads();
                    // Level L: Lineitem rows -> Supplier(supplier, nation). Both
                    // are bound, so the full-key probe is an existence check:
                    // binary search of the supplier within its nation's range.
                    let mut line_base = 0u32;
                    while line_base < lines {
                        let mut order = [0u32; 4];
                        let mut supplier = [0u32; 4];
                        let mut nation = [0u32; 4];
                        let mut found = [false; 4];
                        let mut item = 0usize;
                        #[unroll]
                        while item < 4 {
                            let q = line_base.wrapping_add((item * 256 + tid) as u32);
                            if q < lines && q >= line_base {
                                let parent = owner(p2_offset, 1024, q) as usize;
                                let line_row = *p2_begin.add(parent) + (q - *p2_offset.add(parent));
                                let s = decode_u32(ix.line_supplier, line_row as u64);
                                order[item] = *p2_order.add(parent);
                                supplier[item] = s;
                                nation[item] = *p2_nation.add(parent);
                                let (supplier_begin, supplier_end) = range(ix.supplier_offsets, nation[item]);
                                found[item] = contains_u32(ix.supplier_id, supplier_begin, supplier_end, s);
                            }
                            item += 1;
                        }
                        let hits = found[0] as u32 + found[1] as u32 + found[2] as u32 + found[3] as u32;
                        let (offset, block_total) = checked_scan(hits, meta);
                        if tid == 0 {
                            *base_slot = if block_total > 0 {
                                let base = emitted.fetch_add(block_total, AtomicOrdering::Relaxed);
                                if base.checked_add(block_total).is_none() {
                                    flag(meta, META_OVERFLOW);
                                }
                                base
                            } else {
                                0
                            };
                        }
                        thread::sync_threads();
                        let mut destination = (*base_slot).wrapping_add(offset);
                        item = 0;
                        #[unroll]
                        while item < 4 {
                            if found[item] {
                                emit(out, destination, order[item], supplier[item], nation[item]);
                                destination = destination.wrapping_add(1);
                            }
                            item += 1;
                        }
                        thread::sync_threads();
                        line_base = match line_base.checked_add(1024) {
                            Some(next) => next,
                            None => break,
                        };
                    }
                    order_base = match order_base.checked_add(1024) {
                        Some(next) => next,
                        None => break,
                    };
                }
            }
        }
    }
}
