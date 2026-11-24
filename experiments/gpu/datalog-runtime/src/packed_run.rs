//! Experimental immutable block-compressed runs. Existing radix paths are unchanged.
//! Frames and fences are runtime metadata; payloads contain no fixed-width row slots.
use crate::{PackedField, Packing, Result, Word};
use cuda_core::{DeviceCopy, LaunchConfig1D};
use gpu_device_columns::{Buffer, Pool};
use std::sync::Arc;
mod merge;
mod leaf;
mod geometry;
mod set;
mod permute;
mod small;
pub use small::MAX_SMALL_CANDIDATES;
mod sort;
pub use sort::release_sort_scratch;
pub use set::{OperationTimes, PackedSet};
pub mod replay_support;
pub mod read_support;
pub mod read_diagnostics;
/// Default selected by the real Xalan block-size study; dimensions remain runtime data.
pub const DEFAULT_BLOCK_ROWS: u32 = 256;
/// Small runs avoid frame/header allocations and sealing/readback launches.
/// The Xalan replay selects 32K rows to amortize packed metadata round trips.
pub const SMALL_RUN_ROWS: usize = 32768;
/// Frame-selection controls used by the exactness and compression replay.
#[derive(Clone, Copy, Default)]
pub enum MergeFrames {
    #[default]
    Metadata,
    ExactCodes,
    ZeroPlain,
    /// Measurement only: separately replay copy, fitting and widening leaves.
    LeafPaths,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Header {
    pub offset: u64,
    pub first: u64,
    pub rows: u32,
    pub bits: u32,
    /// 0: plain word lanes, 1: per-column frame of reference.
    pub encoding: u32,
    pub reserved: u32,
}
impl Header {
    /// Payload length is derived, keeping the hot header in one 32-byte sector.
    #[inline(always)]
    pub fn words(&self) -> u64 { (self.rows as u64 * self.bits as u64).div_ceil(64) }
}
unsafe impl DeviceCopy for Header {}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Frame {
    pub base: u64,

    pub bits: u8,
    stride: u8,
    shift: u16,
    /// Plans and wide leaves: inclusive column width. Published narrow
    /// leaves: low 32 bits of the leaf's payload word offset.
    pub end: u32,
}
impl Frame {
    #[inline(always)]
    fn new(base: u64, width: u32) -> Self {
        Self { base, bits: width as _, ..Self::default() }
    }
    #[inline(always)]
    pub fn width(self) -> u32 { self.bits as u32 }
    /// In MS-first column order, distance back from the row end to the
    /// field's low bit. Occupies the old padding word, including for copies.
    #[inline(always)]
    fn after(self, preceding: u32) -> Self {
        Self { end: preceding + self.width(), ..self }
    }
    /// Narrow published frames carry the bit address of this field in the
    /// first row relative to a payload page, plus the byte-sized row stride.
    /// Plans (and wider carriers) instead carry the inclusive column width.
    #[inline(always)]
    unsafe fn published<W: Word>(self, h: Header, fields: *const PackedField, column: usize) -> Self {
        if W::BITS <= 128 {
            let (base, bits, shift) = if h.encoding == 0 {
                let f = *fields.add(column);
                (0, f.bits, f.shift)
            } else { (self.base, self.width(), h.bits - self.end) };
            return Self { base, bits: bits as u8, stride: h.bits as u8, shift: shift as u16, end: h.offset as u32 };
        }

        self
    }
}
unsafe impl DeviceCopy for Frame {}

/// Published bit addresses are relative to the page containing the leaf start.
pub(crate) const FRAME_PAGE_MASK: u64 = u32::MAX as u64;

// Published frames support one aligned 128-bit device read. The residual
// iterator's transient Frame does not have this stronger alignment.
#[repr(C)]
#[repr(align(16))]
#[derive(Clone, Copy)]
struct StoredFrame(Frame);
unsafe impl DeviceCopy for StoredFrame {}

/// Borrowed device ABI. Owners must survive every launch using this view.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PackedView<W: Word> {
    pub plain: *const W,
    pub payload: *const u64,
    pub headers: *const Header,
    pub row_directory: *const u32,
    /// Published frames are 16-byte aligned when packed radix reads are enabled.
    pub frames: *const Frame,
    pub fences: *const W,
    pub fields: *const PackedField,
    pub rows: u64,
    pub blocks: u64,
    /// Initial fixed geometry, or zero for cumulative-header variable leaves.
    pub block_rows: u32,
    pub columns: u32,
}
unsafe impl<W: Word> DeviceCopy for PackedView<W> {}
// Immutable device addresses; the publication retains all pointed-to storage.
unsafe impl<W: Word> Send for PackedView<W> {}
unsafe impl<W: Word> Sync for PackedView<W> {}

struct Modules {
    codec: kernels::LoadedModule,
    small: small::small_kernels::LoadedModule,
    sort: sort::sort_kernels::LoadedModule,
    leaf: leaf::leaf_kernels::LoadedModule,
    runs: crate::runs::run_kernels::LoadedModule,
    // Scratch can outlive a launch only on this module's owning stream.
    partitions: std::sync::Mutex<Option<Buffer<usize>>>,
    rccl: rccl_cub::cuda::Program<'static>,
    // The program borrows this stable pool's context. Keep its owner last.
    _pool: Arc<Pool>,
}
impl Modules {
    fn load(pool: &Arc<Pool>) -> Result<Arc<Self>> {
        crate::phases::admission_scope!(c"admit/module-load");
        let owner = pool.clone();
        // Arc keeps Pool at a stable address; the program drops before owner.
        let retained: &'static Pool = unsafe { &*Arc::as_ptr(&owner) };
        Ok(Arc::new(unsafe { Self { codec: kernels::load(pool.context())?,
            small: small::small_kernels::load(pool.context())?, sort: sort::sort_kernels::load(pool.context())?,
            leaf: leaf::leaf_kernels::load(pool.context())?,
            runs: crate::runs::run_kernels::load(pool.context())?,
            partitions: std::sync::Mutex::new(None),
            rccl: rccl_cub::cuda::Program::load(retained.context())?, _pool: owner } }))
    }
}

pub struct PackedRun<W: Word> {
    pool: Arc<Pool>,
    modules: Arc<Modules>,
    plain: Option<crate::SortedBatch<W>>,
    payload: Buffer<u64>,
    headers: Buffer<Header>,
    row_directory: Buffer<u32>,
    frames: Buffer<StoredFrame>,
    fences: Buffer<W>,
    fields: Option<Arc<Buffer<PackedField>>>,
    packing: Packing,
    rows: usize,
    block_rows: u32,
    variable: bool,
}
impl<W: Word> PackedRun<W> {
    /// Seal an initialized sorted unique prefix whose unused packing bits are zero.
    ///
    /// # Safety
    /// Input must obey that invariant and remain immutable through this stream's work.
    pub unsafe fn seal(input: &Buffer<W>, rows: usize, packing: &Packing, block_rows: u32) -> Result<Self> {
        Self::seal_with_modules(input, rows, packing, block_rows, Modules::load(input.pool())?)
    }
    unsafe fn seal_with_modules(input: &Buffer<W>, rows: usize, packing: &Packing, block_rows: u32, modules: Arc<Modules>) -> Result<Self> {
        Self::seal_in_frame(input, rows, packing, block_rows, modules, false)
    }
    unsafe fn seal_in_frame(input: &Buffer<W>, rows: usize, packing: &Packing, block_rows: u32, modules: Arc<Modules>, framed: bool) -> Result<Self> {
        crate::phases::admission_scope!(c"admit/seal", rows);
        if rows > input.len() || packing.bits() > W::BITS || block_rows == 0 {
            return Err("invalid packed run dimensions".into());
        }
        let pool = input.pool().clone();
        let blocks = rows.div_ceil(block_rows as usize);
        let fields = Buffer::from_host_async(pool.clone(), &packing.packed_fields().collect::<Vec<_>>())?;
        // Each producing kernel writes every element before any consumer runs.
        let headers = Buffer::uninitialized(pool.clone(), blocks)?;
        let frames = Buffer::<StoredFrame>::uninitialized(pool.clone(), blocks * packing.len())?;
        let fences = Buffer::uninitialized(pool.clone(), blocks)?;
        let total = Buffer::uninitialized(pool.clone(), 1)?;
        let module = &modules.codec;
        let cfg = LaunchConfig1D::new(blocks.max(1).min(65535) as u32, 256, 0);
        if blocks != 0 {
            let analyze_cfg = cuda_core::simt::LaunchConfig { grid_dim: (blocks.min(65535) as u32, 1, 1), block_dim: (256, 1, 1), shared_mem_bytes: 4096 };
            module.analyze::<W>(pool.stream(), analyze_cfg, input.ptr(), rows as u64,
                block_rows, fields.ptr(), packing.len() as u32, headers.ptr(), frames.ptr().cast(), fences.ptr(),
                std::env::var_os("PACKED_LEAF_ALIGNED").is_some(), framed)?;
        }
        module.offsets::<W>(pool.stream(), &module.prepare_offsets::<W>(LaunchConfig1D::new(1, 256, 64))?, headers.ptr(), blocks as u64, total.ptr())?;
        let words = {
            crate::phases::admission_scope!(c"admit/seal-sizing");
            total.to_host()?[0] as usize
        };
        let payload = Buffer::uninitialized(pool.clone(), words)?;
        let run = Self { row_directory: Buffer::zeroed(pool.clone(), 0)?, pool, modules: modules.clone(), plain: None,
            payload, headers, frames, fences, fields: Some(Arc::new(fields)), packing: packing.clone(), rows, block_rows, variable: false };
        if blocks != 0 {
            module.pack::<W>(run.pool.stream(), &module.prepare_pack::<W>(cfg)?, input.ptr(), run.view(), run.payload.ptr())?;
        }
        Ok(run)
    }
    fn from_plain(batch: crate::SortedBatch<W>, packing: Packing, block_rows: u32, modules: Arc<Modules>) -> Result<Self> {
        let rows = batch.len();
        let pool = batch.buffer().pool().clone();
        Ok(Self { payload: Buffer::zeroed(pool.clone(), 0)?, headers: Buffer::zeroed(pool.clone(), 0)?,
            frames: Buffer::zeroed(pool.clone(), 0)?, fences: Buffer::zeroed(pool.clone(), 0)?,
            fields: None, row_directory: Buffer::zeroed(pool.clone(), 0)?,
            pool, modules, plain: Some(batch), packing, rows, block_rows, variable: false })
    }
    pub fn len(&self) -> usize { self.rows }
    pub fn is_empty(&self) -> bool { self.rows == 0 }
    pub fn block_rows(&self) -> u32 { self.block_rows }
    pub(crate) fn plain_batch(&self) -> Option<&crate::SortedBatch<W>> { self.plain.as_ref() }
    pub fn packing(&self) -> &Packing { &self.packing }
    pub fn pool(&self) -> &Arc<Pool> { &self.pool }
    pub fn payload_bytes(&self) -> usize { self.payload.allocated_bytes() + self.plain.as_ref().map_or(0, crate::SortedBatch::allocated_bytes) }
    pub fn header_bytes(&self) -> usize { self.headers.allocated_bytes() + self.frames.allocated_bytes() + self.fields.as_ref().map_or(0, |b| b.allocated_bytes()) + self.row_directory.allocated_bytes() }
    pub fn fence_bytes(&self) -> usize { self.fences.allocated_bytes() }
    pub fn allocated_bytes(&self) -> usize { self.payload_bytes() + self.header_bytes() + self.fence_bytes() }
    /// Metadata-only diagnostic, outside measured read/admission scopes.
    pub fn frame_width_histogram(&self) -> Result<[u64; 65]> {
        let mut counts = [0u64; 65];
        if self.plain.is_none() {
            let headers = self.headers.to_host()?;
            let frames = self.frames.to_host()?;
            for (block, header) in headers.iter().enumerate() {
                if header.encoding != 0 {
                    for frame in &frames[block * self.packing.len()..(block + 1) * self.packing.len()] {
                        counts[frame.0.bits as usize] += 1;
                    }
                }
            }
        }
        Ok(counts)
    }
    pub fn view(&self) -> PackedView<W> {
        PackedView { plain: self.plain.as_ref().map_or(std::ptr::null(), |b| b.buffer().ptr()), payload: self.payload.ptr(), headers: self.headers.ptr(), frames: self.frames.ptr().cast(),
            row_directory: if self.variable { self.row_directory.ptr() } else { std::ptr::null() },
            fences: self.fences.ptr(), fields: self.fields.as_ref().map_or(std::ptr::null(), |b| b.ptr()), rows: self.rows as u64, blocks: self.headers.len() as u64,
            block_rows: if self.variable { 0 } else { self.block_rows }, columns: self.packing.len() as u32 }
    }
    /// Explicit oracle/export boundary; never called by an operation kernel.
    // DECODE P: explicit packed-run export publishes reconstructed keys for host answers and exact test oracles outside timed operations.
    pub fn unpack(&self) -> Result<Buffer<W>> {
        let output = unsafe { Buffer::uninitialized(self.pool.clone(), self.rows)? };
        if self.rows != 0 {
            let module = &self.modules.codec;
            module.unpack::<W>(self.pool.stream(), &module.prepare_unpack::<W>(config(self.rows))?, self.view(), output.ptr())?;
        }
        Ok(output)
    }
    pub fn contains(&self, candidates: &Buffer<W>, rows: usize) -> Result<Buffer<u32>> {
        membership(&[self], candidates, rows, false)
    }
    pub fn absent(&self, candidates: &Buffer<W>, rows: usize) -> Result<Buffer<u32>> {
        membership(&[self], candidates, rows, true)
    }
    /// Batched inclusive key intervals; callers encode prefix minima/maxima.
    pub fn ranges(&self, low: &Buffer<W>, high: &Buffer<W>) -> Result<Buffer<[u64; 2]>> {
        if low.len() != high.len() || !self.pool.shares_memory(low.pool()) || !self.pool.shares_memory(high.pool()) {
            return Err("range input shape or pool mismatch".into());
        }
        let output = unsafe { Buffer::uninitialized(self.pool.clone(), low.len())? };
        if !low.is_empty() {
            let module = &self.modules.codec;
            module.ranges::<W>(self.pool.stream(), &module.prepare_ranges::<W>(config(low.len()))?,
                self.view(), low.ptr(), high.ptr(), low.len() as u64, output.ptr())?;
        }
        Ok(output)
    }
}

/// Membership flags against the union of immutable runs, without decoding them.
pub fn membership<W: Word>(runs: &[&PackedRun<W>], candidates: &Buffer<W>, rows: usize, absent: bool) -> Result<Buffer<u32>> {
    let modules = match runs.first() { Some(run) => run.modules.clone(), None => Modules::load(candidates.pool())? };
    membership_with_module(runs, candidates, rows, absent, &modules.codec)
}
fn membership_with_module<W: Word>(runs: &[&PackedRun<W>], candidates: &Buffer<W>, rows: usize, absent: bool, module: &kernels::LoadedModule) -> Result<Buffer<u32>> {
    let pool = candidates.pool();
    if rows > candidates.len() || runs.iter().any(|run| !pool.shares_memory(run.pool())) {
        return Err("membership input shape or pool mismatch".into());
    }
    let output = unsafe { Buffer::uninitialized(pool.clone(), rows)? };
    if rows != 0 {
        let views = {
            crate::phases::admission_scope!(c"admit/member-upload");
            Buffer::from_host_async(pool.clone(), &runs.iter().map(|run| run.view()).collect::<Vec<_>>())?
        };
        launch_membership(module, pool, views.ptr(), runs.len(), candidates, rows, absent, &output)?;
    }
    Ok(output)
}
fn launch_membership<W: Word>(module: &kernels::LoadedModule, pool: &Pool, views: *const PackedView<W>,
    count: usize, candidates: &Buffer<W>, rows: usize, absent: bool, output: &Buffer<u32>) -> Result<()> {
    module.membership::<W>(pool.stream(), &module.prepare_membership::<W>(config(rows))?,
        views, count as u32, candidates.ptr(), rows as u64, absent, output.ptr())?;
    Ok(())
}
fn config(work: usize) -> LaunchConfig1D { LaunchConfig1D::new(work.div_ceil(256).max(1).min(65535) as u32, 256, 0) }

pub mod device {
    use super::*;
    /// Fixed runs retain arithmetic addressing. A four-byte entry per 256 rows
    /// bounds the cumulative-header search for variable leaves.
    #[inline(always)]
    pub unsafe fn row_block<W: Word>(view: PackedView<W>, index: u64) -> u64 {
        if view.blocks == 1 { return 0; }
        if view.block_rows != 0 { return index / view.block_rows as u64; }
        let bucket = index / 256;
        let mut lo = *view.row_directory.add(bucket as usize) as u64;
        let mut hi = if bucket + 1 < view.rows.div_ceil(256) {
            *view.row_directory.add(bucket as usize + 1) as u64 + 1
        } else { view.blocks };
        while lo + 1 < hi {
            let mid = (lo + hi) / 2;
            if (*view.headers.add(mid as usize)).first <= index { lo = mid; }
            else { hi = mid; }
        }
        lo
    }
    /// Plain slot address, narrow frame page and local row, or a wide row's
    /// first word and relative end bit. No variant carries a descriptor.
    #[derive(Clone, Copy)]
    pub struct RowCursor {
        pub(crate) words: *const u64,
        pub(crate) frames: *const Frame,
        pub(crate) bit: u32,
    }
    impl RowCursor {
        #[inline(always)]
        pub(crate) fn plain<W>(slot: *const W) -> Self {
            Self { words: slot.cast(), frames: std::ptr::null(), bit: 0 }
        }
    }
    #[inline(never)]
    pub(crate) unsafe fn resolve_row<W: Word>(descriptor: *const PackedView<W>, position: u64) -> RowCursor {
        let view = &*descriptor;
        let fixed = view.block_rows == DEFAULT_BLOCK_ROWS;
        let block = if fixed { position / DEFAULT_BLOCK_ROWS as u64 } else { row_block(*view, position) };
        let header = &*view.headers.add(block as usize);
        let local = if fixed { position % DEFAULT_BLOCK_ROWS as u64 } else { position - header.first };
        if W::BITS <= 128 {
            // Keep the local bit calculation in u32 even for custom, very large leaves.
            let page = (header.offset & !super::FRAME_PAGE_MASK) + (local / 256) * 4 * header.bits as u64;
            return RowCursor { words: view.payload.wrapping_add(page as usize),
                frames: view.frames.add(block as usize * view.columns as usize), bit: (local % 256) as u32 };
        }
        let start = local * header.bits as u64;
        let words = view.payload.wrapping_add((header.offset + start / 64) as usize);
        if header.encoding == 0 {
            // Raw leaves start each word-aligned row in least-significant-lane order.
            return RowCursor { words, frames: std::ptr::null(), bit: 1 };
        }
        RowCursor { words, frames: view.frames.add(block as usize * view.columns as usize),
            bit: (start % 64) as u32 + header.bits }
    }
    #[inline(never)]
    pub(crate) unsafe fn field_code(row: RowCursor, column: u32) -> (u64, u64) {
        // Published frames are StoredFrame-aligned and devices are little endian.
        let f = *row.frames.add(column as usize).cast::<u128>();
        let width = (f >> 64) as u32;
        let end = (f >> 96) as u32;
        let bit = row.bit - end;
        (read_code(row.words, bit as u64, width), f as u64)
    }
    /// Base, width and the first row's bit address arrive in a single frame load.
    #[inline(always)]
    pub(crate) unsafe fn direct_code(row: RowCursor, column: u32) -> (u64, u64) {
        let f = *row.frames.add(column as usize).cast::<u128>();
        let shape = (f >> 64) as u32;
        let width = shape & 255;
        let bit = (shape >> 16) + row.bit * ((shape >> 8) & 255);
        let words = row.words.wrapping_add((f >> 96) as usize);
        let code = if width.wrapping_sub(1) < 32 {
            let offset = bit & 63;
            let word = words.add((bit >> 6) as usize);
            let mut value = (*word >> offset) as u32;
            if offset + width > 64 { value |= *word.add(1).cast::<u32>() << (64 - offset); }
            (value & (u32::MAX >> (32 - width))) as u64
        } else if width == 0 { 0 } else { wide_code(words, bit, width) };
        (code, f as u64)
    }
    #[inline(never)]
    unsafe fn wide_code(words: *const u64, bit: u32, width: u32) -> u64 {
        read_bits(words, bit as u64, width)
    }
    /// Plain-storage specialization: no packed metadata or decoder live state.
    #[inline(always)]
    pub unsafe fn plain_bound<W: Word>(view: PackedView<W>, key: W, upper: bool) -> u64 {
        search_plain::<W, false>(view, key, upper)
    }
    #[inline(always)]
    unsafe fn search_plain<W: Word, const MEMBER: bool>(view: PackedView<W>, key: W, upper: bool) -> u64 {
        let mut begin = 0;
        let mut end = view.rows;
        while begin < end {
            let mid = (begin + end) / 2;
            let order = (*view.plain.add(mid as usize)).compare(&key);
            if MEMBER && order.is_eq() { return 1; }
            if order.is_lt() || (upper && order.is_eq()) { begin = mid + 1; } else { end = mid; }
        }
        if MEMBER { 0 } else { begin }
    }
    #[inline(always)]
    pub unsafe fn read_range<W: Word, const PLAIN: bool>(view: PackedView<W>, low: W, high: W) -> (u64, u64) {
        if PLAIN { (plain_bound(view, low, false), plain_bound(view, high, true)) }
        else { range(view, low, high) }
    }
    #[inline(always)]
    pub unsafe fn read_bits(payload: *const u64, bit: u64, width: u32) -> u64 {
        if width == 0 { return 0; }
        let offset = (bit % 64) as u32;
        let mut value = *payload.add((bit / 64) as usize) >> offset;
        if offset != 0 && width > 64 - offset {
            value |= *payload.add((bit / 64 + 1) as usize) << (64 - offset);
        }
        value & (u64::MAX >> (64 - width))
    }
    /// Extract narrow fields with native 32-bit shifts and loads.
    #[inline(always)]
    pub unsafe fn read_code(payload: *const u64, bit: u64, width: u32) -> u64 {
        if width > 32 { return read_bits(payload, bit, width); }
        if width == 0 { return 0; }
        read_bits32(payload.cast::<u32>(), bit, width) as u64
    }
    /// Caller has established a nonzero width of at most 32 bits.
    #[inline(always)]
    pub unsafe fn read_bits32(payload: *const u32, bit: u64, width: u32) -> u32 {
        let offset = (bit % 32) as u32;
        let mut code = *payload.add((bit / 32) as usize) >> offset;
        if offset != 0 && width > 32 - offset {
            code |= *payload.add((bit / 32 + 1) as usize) << (32 - offset);
        }
        code & (u32::MAX >> (32 - width))
    }
    /// Borrowed encoded row. Consumers receive residuals and their frames, never
    /// a reconstructed key. Plain payload blocks use the identity (zero) frame.
    #[repr(C)]
    pub struct Codes<W: Word> {
        payload: *const u64,
        plain: *const W,
        fields: *const PackedField,
        frames: *const Frame,
        bit: u64,
        column: u32,
        columns: u32,
        packed: u32,
    }
    /// Iterate a metadata-bearing row, including identity-frame payload blocks.
    /// # Safety
    /// `view.plain` is null, `index` is in bounds, and the producing stream is ordered.
    #[inline(always)]
    pub unsafe fn codes<W: Word>(view: PackedView<W>, index: u64) -> Codes<W> {
        let block = row_block(view, index);
        let header = *view.headers.add(block as usize);
        let local = index - header.first;
        let packed = header.encoding;
        let bit = header.offset * 64 + if packed != 0 { (local + 1) * header.bits as u64 } else { local * W::BITS as u64 };
        Codes { payload: view.payload, plain: view.plain, fields: view.fields,
            frames: view.frames.add((block * view.columns as u64) as usize),
            bit, column: 0, columns: view.columns, packed }
    }
    impl<W: Word> Codes<W> {
        #[inline(always)]
        pub unsafe fn next(&mut self) -> Option<(u64, Frame, PackedField)> {
            if self.column == self.columns { return None; }
            let field = *self.fields.add(self.column as usize);
            if !self.plain.is_null() {
                self.column += 1;
                return Some(((*self.plain).extract(field.shift, field.bits),
                    Frame::new(0, field.bits), field));
            }
            let frame = if self.packed != 0 { *self.frames.add(self.column as usize) }
                else { Frame::new(0, field.bits) };
            let bit = if self.packed != 0 { self.bit -= frame.width() as u64; self.bit }
                else { self.bit + field.shift as u64 };
            self.column += 1;
            Some((read_code(self.payload, bit, frame.width()), frame, field))
        }
    }
    #[inline(always)]
    pub(super) unsafe fn source_codes<W: Word>(mut view: PackedView<W>, index: u64, fields: *const PackedField, columns: u32) -> Codes<W> {
        view.fields = fields;
        view.columns = columns;
        if view.plain.is_null() { return codes(view, index); }
        Codes { payload: view.payload, plain: view.plain.add(index as usize), fields,
            frames: view.frames, bit: 0, column: 0, columns, packed: 0 }
    }
    /// Compare fields in the lower of the two frames, with no global key.
    #[inline(always)]
    pub(super) unsafe fn compare_rows<W: Word>(a: PackedView<W>, ai: u64, b: PackedView<W>, bi: u64,
        fields: *const PackedField, columns: u32) -> core::cmp::Ordering {
        if !a.plain.is_null() && !b.plain.is_null() {
            return (*a.plain.add(ai as usize)).compare(&*b.plain.add(bi as usize));
        }
        let mut left = source_codes(a, ai, fields, columns);
        let mut right = source_codes(b, bi, fields, columns);
        while let Some((x, af, field)) = left.next() {
            let (y, bf, _) = right.next().unwrap();
            let order = if field.bits <= 32 {
                let common = (af.base as u32).min(bf.base as u32);
                ((x as u32) + (af.base as u32 - common)).cmp(&((y as u32) + (bf.base as u32 - common)))
            } else {
                let common = af.base.min(bf.base);
                (x + (af.base - common)).cmp(&(y + (bf.base - common)))
            };
            if !order.is_eq() { return order; }
        }
        core::cmp::Ordering::Equal
    }
    /// # Safety
    /// View is live, index is in bounds, and its producing stream is ordered before this read.
    #[inline(always)]
    // DECODE P: private row reconstruction serves only the explicit packed-run export kernel.
    pub(super) unsafe fn export_row<W: Word>(view: PackedView<W>, index: u64) -> W {
        if !view.plain.is_null() { return *view.plain.add(index as usize); }
        let block = row_block(view, index);
        let header = *view.headers.add(block as usize);
        let local = index - header.first;
        let mut word = W::zero();
        if header.encoding == 0 {
            let mut lane = 0;
            while lane < W::BITS / 64 {
                word.set_lane(lane, *view.payload.add((header.offset + local * (W::BITS / 64) as u64 + lane as u64) as usize));
                lane += 1;
            }
        } else {
            let mut bit = header.offset * 64 + (local + 1) * header.bits as u64;
            let mut col = 0;
            while col < view.columns {
                let frame = *view.frames.add((block * view.columns as u64 + col as u64) as usize);
                let field = *view.fields.add(col as usize);
                bit -= frame.width() as u64;
                let value = frame.base + read_bits(view.payload, bit, frame.width());
                word.insert(field.shift, field.bits, value);
                col += 1;
            }
        }
        word
    }
    #[inline(always)]
    pub unsafe fn bound<W: Word>(view: PackedView<W>, key: W, upper: bool) -> u64 {
        search::<W, false>(view, key, upper)
    }
    /// Membership shares probe encoding and row-code comparisons with bounds.
    #[inline(always)]
    pub unsafe fn contains<W: Word>(view: PackedView<W>, key: W) -> bool {
        search::<W, true>(view, key, false) != 0
    }
    #[inline(always)]
    unsafe fn search<W: Word, const MEMBER: bool>(view: PackedView<W>, key: W, upper: bool) -> u64 {
        if !view.plain.is_null() { return search_plain::<W, MEMBER>(view, key, upper); }
        if view.blocks == 0 { return 0; }
        let fence = fence_bound(view, key, upper || MEMBER, 0, view.blocks);
        search_leaf::<W, MEMBER>(view, key, upper, fence.saturating_sub(1))
    }
    #[inline(always)]
    unsafe fn fence_bound<W: Word>(view: PackedView<W>, key: W, upper: bool, mut lo: u64, mut hi: u64) -> u64 {
        while lo < hi {
            let mid = (lo + hi) / 2;
            let order = (*view.fences.add(mid as usize)).compare(&key);
            if order.is_lt() || (upper && order.is_eq()) { lo = mid + 1; } else { hi = mid; }
        }
        lo
    }
    #[inline(always)]
    unsafe fn search_leaf<W: Word, const MEMBER: bool>(view: PackedView<W>, key: W, upper: bool, block: u64) -> u64 {
        let header = *view.headers.add(block as usize);
        let mut begin = 0u32;
        let mut end = header.rows;
        if header.encoding != 0 {
            if header.bits <= 64 {
                // Translate the probe once. Concatenated column residuals
                // preserve lexicographic order and need only one extraction.
                let used = if view.columns == 0 { 0 } else {
                    let first = *view.fields;
                    first.shift + first.bits
                };
                let mut lane = 0;
                while lane < W::BITS / 64 {
                    let bits = used.saturating_sub(lane * 64).min(64);
                    if bits < 64 && key.lane(lane) >> bits != 0 { return if MEMBER { 0 } else { header.first + end as u64 }; }
                    lane += 1;
                }
                let mut probe = 0u64;
                let mut remaining = header.bits;
                let mut inclusive = upper;
                let mut col = 0;
                while col < view.columns {
                    let frame = *view.frames.add((block * view.columns as u64 + col as u64) as usize);
                    let field = *view.fields.add(col as usize);
                    let value = key.extract(field.shift, field.bits);
                    if value < frame.base {
                        // The probe precedes every suffix with this prefix.
                        inclusive = false;
                        break;
                    }
                    let residual = value - frame.base;
                    if frame.width() < 64 && residual >> frame.width() != 0 {
                        // The probe follows every suffix with this prefix.
                        if remaining != 0 { probe |= u64::MAX >> (64 - remaining); }
                        inclusive = true;
                        break;
                    }
                    remaining -= frame.width();
                    if frame.width() != 0 { probe |= residual << remaining; }
                    col += 1;
                }
                if MEMBER && col != view.columns { return 0; }
                while begin < end {
                    let mid = begin + (end - begin) / 2;
                    let residual = read_bits(view.payload, header.offset * 64 + mid as u64 * header.bits as u64, header.bits);
                    if MEMBER && residual == probe { return 1; }
                    if residual < probe || (inclusive && residual == probe) { begin = mid + 1; } else { end = mid; }
                }
                return if MEMBER { 0 } else { header.first + begin as u64 };
            }
            return wide_residual_bound::<W, MEMBER>(view, key, upper, block, header);
        }
        // The fence selected this identity-frame leaf, so every midpoint
        // addresses its payload directly; no row directory search is needed.
        let payload = view.payload.add(header.offset as usize);
        while begin < end {
            let mid = begin + (end - begin) / 2;
            let order = plain_compare(payload, mid, key);
            if MEMBER && order.is_eq() { return 1; }
            if order.is_lt() || (upper && order.is_eq()) { begin = mid + 1; } else { end = mid; }
        }
        if MEMBER { 0 } else { header.first + begin as u64 }
    }
    /// Plain payload blocks are already in the zero frame; compare lanes in place.
    #[inline(always)]
    unsafe fn plain_compare<W: Word>(payload: *const u64, local: u32, key: W) -> core::cmp::Ordering {
        let offset = local as u64 * (W::BITS / 64) as u64;
        let mut lane = W::BITS / 64;
        while lane != 0 {
            lane -= 1;
            let order = (*payload.add((offset + lane as u64) as usize)).cmp(&key.lane(lane));
            if !order.is_eq() { return order; }
        }
        core::cmp::Ordering::Equal
    }
    #[inline(always)]
    unsafe fn wide_residual_bound<W: Word, const MEMBER: bool>(view: PackedView<W>, key: W, mut inclusive: bool, block: u64, header: Header) -> u64 {
        let mut begin = 0u32;
        let mut end = header.rows;
        let used = if view.columns == 0 { 0 } else { let first = *view.fields; first.shift + first.bits };
        let mut lane = 0;
        while lane < W::BITS / 64 {
            let bits = used.saturating_sub(lane * 64).min(64);
            if bits < 64 && key.lane(lane) >> bits != 0 { return if MEMBER { 0 } else { header.first + end as u64 }; }
            lane += 1;
        }
        let mut probe = W::zero();
        let mut remaining = header.bits;
        let mut col = 0;
        while col < view.columns {
            let frame = *view.frames.add((block * view.columns as u64 + col as u64) as usize);
            let field = *view.fields.add(col as usize);
            let value = key.extract(field.shift, field.bits);
            if value < frame.base { inclusive = false; break; }
            let residual = value - frame.base;
            if frame.width() < 64 && residual >> frame.width() != 0 {
                let mut lane = 0;
                while lane < W::BITS / 64 {
                    let bits = remaining.saturating_sub(lane * 64).min(64);
                    if bits != 0 { probe.set_lane(lane, probe.lane(lane) | (u64::MAX >> (64 - bits))); }
                    lane += 1;
                }
                inclusive = true;
                break;
            }
            remaining -= frame.width();
            if frame.width() != 0 { probe.insert(remaining, frame.width(), residual); }
            col += 1;
        }
        if MEMBER && col != view.columns { return 0; }
        while begin < end {
            let mid = begin + (end - begin) / 2;
            let bit = header.offset * 64 + mid as u64 * header.bits as u64;
            let order = if W::BITS == 128 {
                // Two independent code loads avoid a high-lane dependency chain.
                // The wide path guarantees 64 < header.bits <= W::BITS.
                let low = read_bits(view.payload, bit, 64);
                let high = read_bits(view.payload, bit + 64, header.bits - 64);
                let same_high = high == probe.lane(1);
                if (high < probe.lane(1)) | (same_high & (low < probe.lane(0))) { core::cmp::Ordering::Less }
                else if same_high & (low == probe.lane(0)) { core::cmp::Ordering::Equal }
                else { core::cmp::Ordering::Greater }
            } else {
                let mut order = core::cmp::Ordering::Equal;
                let mut lane = header.bits.div_ceil(64);
                while lane != 0 {
                    lane -= 1;
                    let code = read_bits(view.payload, bit + lane as u64 * 64, (header.bits - lane * 64).min(64));
                    order = code.cmp(&probe.lane(lane));
                    if !order.is_eq() { break; }
                }
                order
            };
            if MEMBER && order.is_eq() { return 1; }
            if order.is_lt() || (inclusive && order.is_eq()) { begin = mid + 1; } else { end = mid; }
        }
        if MEMBER { 0 } else { header.first + begin as u64 }
    }
    /// Search with an enumerated source row, retaining its codes and frame.
    /// Both views must have the same column schema; the source is packed.
    /// No global probe key is constructed between source and destination.
    #[inline(always)]
    pub unsafe fn code_bound<W: Word>(target: PackedView<W>, source: PackedView<W>, index: u64, upper: bool) -> u64 {
        let fields = source.fields;
        let columns = source.columns;
        let mut lo = 0;
        let mut hi = if target.plain.is_null() { target.blocks } else { 0 };
        while lo < hi {
            let mid = (lo + hi) / 2;
            let fence = *target.fences.add(mid as usize);
            let identity = PackedView { plain: &fence, rows: 1, fields, columns, ..target };
            let order = compare_rows(identity, 0, source, index, fields, columns);
            if order.is_lt() || (upper && order.is_eq()) { lo = mid + 1; } else { hi = mid; }
        }
        let block = lo.saturating_sub(1);
        let first = if target.plain.is_null() && target.blocks != 0 { (*target.headers.add(block as usize)).first } else { 0 };
        let mut begin = first;
        let mut end = if target.plain.is_null() && target.blocks != 0 { first + (*target.headers.add(block as usize)).rows as u64 } else { target.rows };
        if target.plain.is_null() && begin < end {
            let h = *target.headers.add(block as usize);
            if h.encoding != 0 {
                let mut source_codes = codes(source, index);
                let mut probe = W::zero();
                let mut remaining = h.bits;
                let mut inclusive = upper;
                let mut col = 0;
                while let Some((code, frame, _)) = source_codes.next() {
                    let target_frame = *target.frames.add((block * columns as u64 + col as u64) as usize);
                    let residual = if frame.base >= target_frame.base {
                        code + (frame.base - target_frame.base)
                    } else {
                        let delta = target_frame.base - frame.base;
                        if code < delta { inclusive = false; break; }
                        code - delta
                    };
                    if target_frame.width() < 64 && residual >> target_frame.width() != 0 {
                        let mut lane = 0;
                        while lane * 64 < remaining {
                            let width = (remaining - lane * 64).min(64);
                            probe.set_lane(lane, probe.lane(lane) | (u64::MAX >> (64 - width)));
                            lane += 1;
                        }
                        inclusive = true;
                        break;
                    }
                    remaining -= target_frame.width();
                    if target_frame.width() != 0 { probe.insert(remaining, target_frame.width(), residual); }
                    col += 1;
                }
                while begin < end {
                    let mid = (begin + end) / 2;
                    let bit = h.offset * 64 + (mid - first) * h.bits as u64;
                    let mut lane = h.bits.div_ceil(64);
                    let mut order = core::cmp::Ordering::Equal;
                    while lane != 0 {
                        lane -= 1;
                        order = read_bits(target.payload, bit + lane as u64 * 64, (h.bits - lane * 64).min(64)).cmp(&probe.lane(lane));
                        if !order.is_eq() { break; }
                    }
                    if order.is_lt() || (inclusive && order.is_eq()) { begin = mid + 1; } else { end = mid; }
                }
                return begin;
            }
        }
        while begin < end {
            let mid = (begin + end) / 2;
            let order = compare_rows(target, mid, source, index, fields, columns);
            if order.is_lt() || (upper && order.is_eq()) { begin = mid + 1; } else { end = mid; }
        }
        begin
    }
    pub unsafe fn lower_bound<W: Word>(view: PackedView<W>, key: W) -> u64 { bound(view, key, false) }
    pub unsafe fn upper_bound<W: Word>(view: PackedView<W>, key: W) -> u64 { bound(view, key, true) }
    /// Inclusive encoded prefix interval, matching the radix run range contract.
    pub unsafe fn range<W: Word>(view: PackedView<W>, low: W, high: W) -> (u64, u64) {
        if !view.plain.is_null() { return (plain_bound(view, low, false), plain_bound(view, high, true)); }
        if view.blocks == 0 { return (0, 0); }
        if W::BITS > 64 {
            // Reuse each multi-lane fence load until the endpoint searches
            // diverge. After the split their remaining intervals are disjoint.
            let mut lo = 0;
            let mut hi = view.blocks;
            let (left, right) = loop {
                if lo == hi { break (lo, lo); }
                let mid = (lo + hi) / 2;
                let fence = *view.fences.add(mid as usize);
                let l = fence.compare(&low).is_lt();
                let r = !fence.compare(&high).is_gt();
                if l != r {
                    let left = fence_bound(view, low, false, if l { mid + 1 } else { lo }, if l { hi } else { mid });
                    let right = fence_bound(view, high, true, if r { mid + 1 } else { lo }, if r { hi } else { mid });
                    break (left, right);
                }
                if l { lo = mid + 1; } else { hi = mid; }
            };
            return (search_leaf::<W, false>(view, low, false, left.saturating_sub(1)),
                    search_leaf::<W, false>(view, high, true, right.saturating_sub(1)));
        }
        let left = fence_bound(view, low, false, 0, view.blocks);
        let a = search_leaf::<W, false>(view, low, false, left.saturating_sub(1));
        // Short ranges usually end in the same or the next leaf. The fence
        // immediately after the lower leaf gives an exact test, even for ties.
        let right = if left < view.blocks && !(*view.fences.add(left as usize)).compare(&high).is_gt() {
            let next = left + 1;
            if next < view.blocks && !(*view.fences.add(next as usize)).compare(&high).is_gt() {
                fence_bound(view, high, true, next + 1, view.blocks)
            } else { next }
        } else if low.compare(&high).is_gt() {
            fence_bound(view, high, true, 0, left)
        } else { left };
        (a, search_leaf::<W, false>(view, high, true, right.saturating_sub(1)))
    }
}

#[cuda_host::cuda_module]
mod kernels {
    use super::{device, Frame, Header, PackedView};
    use crate::{PackedField, Word};
    use cuda_device::{kernel, launch_bounds, launch_contract, thread, DynamicSharedArray};
    #[kernel]
    #[launch_bounds(32)]
    #[launch_contract(domain = 1, block = (32, 1, 1))]
    pub fn common_origins<W: Word>(view: PackedView<W>, origins: *mut u64) {
        let col = thread::blockIdx_x();
        let lane = thread::threadIdx_x();
        let mut block = lane as u64;
        let mut origin = u64::MAX;
        while block < view.blocks {
            let header = unsafe { *view.headers.add(block as usize) };
            if header.encoding == 0 { origin = 0; break; }
            let frame = unsafe { *view.frames.add((block * view.columns as u64 + col as u64) as usize) };
            origin = origin.min(frame.base);
            block += 32;
        }
        let mut step = 16;
        while step != 0 {
            origin = origin.min(cuda_device::warp::shuffle_down_u64(origin, step));
            step /= 2;
        }
        if lane == 0 { unsafe { *origins.add(col as usize) = origin; } }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn permute_codes<W: Word>(view: PackedView<W>, origins: *const u64, destinations: *const u32,
        fields: *const PackedField, words: *mut W) {
        let mut row = thread::index_1d().get() as u64;
        while row < view.rows {
            unsafe {
                let mut codes = device::codes(view, row);
                let mut word = W::zero();
                let mut src = 0;
                while let Some((code, frame, _)) = codes.next() {
                    let field = *fields.add(*destinations.add(src) as usize);
                    word.insert(field.shift, field.bits, code + (frame.base - *origins.add(src)));
                    src += 1;
                }
                *words.add(row as usize) = word;
            }
            row += thread::gridDim_x() as u64 * 256;
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn shift_frames<W: Word>(view: PackedView<W>, origins: *const u64, destinations: *const u32,
        frames: *mut Frame, fences: *mut W) {
        let mut block = thread::index_1d().get() as u64;
        while block < view.blocks {
            unsafe {
                let relative = *fences.add(block as usize);
                let mut fence = W::zero();
                let mut src = 0;
                while src < view.columns as usize {
                    let destination = *destinations.add(src) as usize;
                    let origin = *origins.add(src);
                    let field = *view.fields.add(destination);
                    (*frames.add(block as usize * view.columns as usize + destination)).base += origin;
                    // Fences are metadata in the run-wide comparison frame.
                    fence.insert(field.shift, field.bits, relative.extract(field.shift, field.bits) + origin);
                    src += 1;
                }
                *fences.add(block as usize) = fence;
            }
            block += thread::gridDim_x() as u64 * 256;
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    pub fn analyze<W: Word>(input: *const W, rows: u64, block_rows: u32, fields: *const PackedField,
        columns: u32, headers: *mut Header, frames: *mut Frame, fences: *mut W, aligned: bool, framed: bool) {
        let scratch = DynamicSharedArray::<u64>::get();
        let tid = thread::threadIdx_x() as u64;
        let mut block = thread::blockIdx_x() as u64;
        while block < rows.div_ceil(block_rows as u64) {
            unsafe {
                let start = block * block_rows as u64;
                let count = (rows - start).min(block_rows as u64);
                if tid == 0 { *fences.add(block as usize) = *input.add(start as usize); }
                let mut total_bits = 0;
                let mut col = 0;
                while col < columns {
                    let field = *fields.add(col as usize);
                    let mut min = u64::MAX;
                    let mut max = 0;
                    let mut i = tid;
                    while i < count {
                        let value = (*input.add((start + i) as usize)).extract(field.shift, field.bits);
                        min = min.min(value); max = max.max(value); i += 256;
                    }
                    *scratch.add(tid as usize) = min;
                    *scratch.add(tid as usize + 256) = max;
                    thread::sync_threads();
                    let mut stride = 128;
                    while stride != 0 {
                        if tid < stride {
                            *scratch.add(tid as usize) = (*scratch.add(tid as usize)).min(*scratch.add((tid + stride) as usize));
                            *scratch.add(tid as usize + 256) = (*scratch.add(tid as usize + 256)).max(*scratch.add((tid + stride) as usize + 256));
                        }
                        thread::sync_threads();
                        stride /= 2;
                    }
                    if tid == 0 {
                        let low = *scratch;
                        let high = *scratch.add(256);
                        let bits = if aligned { 64 - (low ^ high).leading_zeros() } else { 64 - (high - low).leading_zeros() };
                        let base = if aligned { if bits == 64 { 0 } else { (low >> bits) << bits } } else { low };
                        *frames.add((block * columns as u64 + col as u64) as usize) = Frame::new(base, bits).after(total_bits);
                        total_bits += bits;
                    }
                    thread::sync_threads();
                    col += 1;
                }
                if tid == 0 {
                    let packed = (count * total_bits as u64).div_ceil(64);
                    let plain = count * (W::BITS / 64) as u64;
                    *headers.add(block as usize) = Header { offset: 0, first: start, rows: count as u32,
                        bits: if framed || packed < plain { total_bits } else { W::BITS }, encoding: (framed || packed < plain) as u32, reserved: 0 };
                }
            }
            block += thread::gridDim_x() as u64;
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1), dynamic_shared_range = (64, 64))]
    pub fn offsets<W: Word>(headers: *mut Header, blocks: u64, total: *mut u64) {
        let scratch = DynamicSharedArray::<u64>::get();
        let tid = thread::threadIdx_x();
        let lane = tid & 31;
        let warp = tid >> 5;
        let mut base = 0u64;
        let mut carry = 0u64;
        while base < blocks {
            let index = base + tid as u64;
            let value = if index < blocks { unsafe { (*headers.add(index as usize)).words() } } else { 0 };
            let mut inclusive = value;
            let mut distance = 1;
            while distance < 32 {
                let previous = cuda_device::warp::shuffle_up_u64(inclusive, distance);
                if lane >= distance { inclusive += previous; }
                distance *= 2;
            }
            if lane == 31 { unsafe { *scratch.add(warp as usize) = inclusive; } }
            thread::sync_threads();
            if warp == 0 {
                let mut sum = if lane < 8 { unsafe { *scratch.add(lane as usize) } } else { 0 };
                distance = 1;
                while distance < 8 {
                    let previous = cuda_device::warp::shuffle_up_u64(sum, distance);
                    if lane >= distance { sum += previous; }
                    distance *= 2;
                }
                if lane < 8 { unsafe { *scratch.add(lane as usize) = sum; } }
            }
            thread::sync_threads();
            let before = if warp == 0 { 0 } else { unsafe { *scratch.add(warp as usize - 1) } };
            if index < blocks { unsafe { (*headers.add(index as usize)).offset = carry + before + inclusive - value; } }
            carry += unsafe { *scratch.add(7) };
            thread::sync_threads();
            base += 256;
        }
        if tid == 0 { unsafe { *total = carry; } }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn pack<W: Word>(input: *const W, view: PackedView<W>, output: *mut u64) {
        let mut block = thread::blockIdx_x() as u64;
        let stride = thread::gridDim_x() as u64;
        while block < view.blocks {
            unsafe {
                let header = *view.headers.add(block as usize);
                let mut word = thread::threadIdx_x() as u64;
                let input = input.add(header.first as usize);
                while word < header.words() {
                    *output.add((header.offset + word) as usize) = super::merge::merge_kernels::payload_word(view, block, input, word);
                    word += 256;
                }
                if W::BITS <= 128 {
                    // Finish every payload read of these frames before updating them.
                    thread::sync_threads();
                    let mut col = thread::threadIdx_x() as usize;
                    while col < view.columns as usize {
                        let frame = view.frames.cast_mut().add(block as usize * view.columns as usize + col);
                        *frame = (*frame).published::<W>(header, view.fields, col);
                        col += 256;
                    }
                }
            }
            block += stride;
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn unpack<W: Word>(view: PackedView<W>, output: *mut W) {
        let mut i = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * 256;
        while i < view.rows { unsafe { *output.add(i as usize) = device::export_row(view, i); } i += stride; }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn membership<W: Word>(views: *const PackedView<W>, count: u32, input: *const W, rows: u64, absent: bool, output: *mut u32) {
        let mut i = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * 256;
        while i < rows {
            unsafe {
                let key = *input.add(i as usize);
                let mut found = false;
                let mut run = 0;
                while run < count && !found {
                    let view = *views.add(run as usize);
                    found = device::contains(view, key);
                    run += 1;
                }
                *output.add(i as usize) = (found != absent) as u32;
            }
            i += stride;
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn ranges<W: Word>(view: PackedView<W>, low: *const W, high: *const W, rows: u64, output: *mut [u64; 2]) {
        let mut i = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * 256;
        while i < rows {
            unsafe {
                let (a, b) = device::range(view, *low.add(i as usize), *high.add(i as usize));
                *output.add(i as usize) = [a, b];
            }
            i += stride;
        }
    }
}

