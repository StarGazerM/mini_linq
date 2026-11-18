use super::*;
use super::merge::MergeEvents;
use std::alloc::Layout;

const LEAF_CAP: u64 = 512;
const WRITE_THREADS: u32 = 128;
const SPLICE_LANES: u32 = 16;
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(super) struct Leaf {
    begin: u64,
    end: u64,
    offset: u64,
    block: u64,
    rows: u64,
    bits: u32,
    encoding: u32,
    keep: u32,
    split_rows: u32,
    extra_rows: u32,
}
unsafe impl DeviceCopy for Leaf {}

fn section<T>(layout: &mut Layout, count: usize) -> Result<usize> {
    let (next, offset) = layout.extend(Layout::array::<T>(count)?)?;
    *layout = next;
    Ok(offset)
}

unsafe fn download<T: DeviceCopy>(pool: &Pool, ptr: *const T, host: &mut [T]) -> Result<()> {
    if !host.is_empty() {
        cuda_core::simt::memory::memcpy_dtoh_async(host.as_mut_ptr().cast::<u8>(), ptr as u64,
            std::mem::size_of_val(host), pool.stream().cu_stream())?;
        pool.stream().synchronize()?;
    }
    Ok(())
}

impl<W: Word> PackedRun<W> {
    pub(super) unsafe fn merge_leaves(&self, other: &Self, events: Option<&MergeEvents>, policy: MergeFrames) -> Result<Self> {
        crate::phases::admission_scope!(c"admit/leaf-merge");
        let (owner, other) = if self.rows >= other.rows { (self, other) } else { (other, self) };
        let pool = owner.pool.clone();
        let columns = owner.packing.len();
        let blocks = if owner.plain.is_some() { owner.rows.div_ceil(owner.block_rows as usize) } else { owner.headers.len() };
        // Fixed/fixed with at most a splice team of incoming rows per
        // owner on average amortizes the lane planner at a smaller grid.
        // Skew changes work, never the bounds or contents of an output.
        let sparse_fixed = blocks > 2_048 && owner.plain.is_none() && other.plain.is_none()
            && !owner.variable && !other.variable && other.rows.div_ceil(blocks) <= SPLICE_LANES as usize;
        let tiled = !sparse_fixed && (1..=128 * 128).contains(&blocks);
        let mut source = owner.view();
        let mut schema = owner.fields.clone();
        if let Some(plain) = &owner.plain {
            // The sparse writer and the explicit copy-bandwidth experiment
            // splice *encoded* owner words. Only those paths need a payload.
            if !tiled || (matches!(policy, MergeFrames::LeafPaths) && std::env::var_os("PACKED_COPY_BOUND").is_some()) {
                let sealed = Self::seal_with_modules(plain.buffer(), owner.rows, &owner.packing, owner.block_rows, owner.modules.clone())?;
                return sealed.merge_leaves(other, events, policy);
            }
        }
        // All private arrays have the same call/stream lifetime. Retire one
        // aligned pool allocation, not an event for each of its typed regions.
        let tile_count = blocks.div_ceil(if tiled { 4 } else { 32 });
        let fuse = tiled && tile_count <= 128;
        let groups = if tiled { other.rows / 32 } else { 0 };
        let bootstrap = if owner.plain.is_some() { blocks } else { 0 };
        let mut layout = Layout::new::<()>();
        let plan_at = section::<Leaf>(&mut layout, blocks)?;
        let frame_at = section::<Frame>(&mut layout, blocks * columns)?;
        let tile_at = section::<[u64; 2]>(&mut layout, tile_count)?;
        let exception_at = section::<[u32; 2]>(&mut layout, if tiled { 0 } else { tile_count })?;
        let cut_at = section::<u64>(&mut layout, if tiled { blocks + 1 } else { 0 })?;
        let summary_at = section::<[u64; 2]>(&mut layout, groups * columns)?;
        let total_at = section::<u64>(&mut layout, 3 + usize::from(fuse))?;
        let header_at = section::<Header>(&mut layout, bootstrap)?;
        let source_frame_at = section::<Frame>(&mut layout, bootstrap * columns)?;
        let fence_at = section::<W>(&mut layout, bootstrap)?;
        // u128 covers the alignment of every sealed Word and metadata type.
        debug_assert!(layout.align() <= align_of::<u128>());
        let workspace = Buffer::<u128>::uninitialized(pool.clone(), layout.size().div_ceil(size_of::<u128>()))?;
        let base = workspace.ptr().cast::<u8>();
        let plans = base.add(plan_at).cast::<Leaf>();
        let frames = base.add(frame_at).cast::<Frame>();
        let tiles = base.add(tile_at).cast::<[u64; 2]>();
        let exceptions = base.add(exception_at).cast::<[u32; 2]>();
        let cuts = base.add(cut_at).cast::<u64>();
        let summaries = base.add(summary_at).cast::<[u64; 2]>();
        let total = base.add(total_at).cast::<u64>();
        let retired = if fuse { total.add(3).cast::<u32>() } else { std::ptr::null_mut() };
        if let Some(plain) = &owner.plain {
            schema = match &other.fields {
                Some(fields) => Some(fields.clone()),
                None => Some(Arc::new(Buffer::from_host_async(pool.clone(), &owner.packing.packed_fields().collect::<Vec<_>>())?)),
            };
            let headers = base.add(header_at).cast::<Header>();
            let old_frames = base.add(source_frame_at).cast::<Frame>();
            let fences = base.add(fence_at).cast::<W>();
            source = PackedView { headers, frames: old_frames, fences,
                fields: schema.as_ref().unwrap().ptr(), blocks: blocks as u64, ..source };
            let cfg = cuda_core::simt::LaunchConfig { grid_dim: (blocks as u32, 1, 1), block_dim: (256, 1, 1), shared_mem_bytes: 4096 };
            owner.modules.codec.analyze::<W>(pool.stream(), cfg, plain.buffer().ptr(), owner.rows as u64,
                owner.block_rows, source.fields, columns as u32, headers, old_frames, fences,
                std::env::var_os("PACKED_LEAF_ALIGNED").is_some(), false)?;
        }
        let module = &owner.modules.leaf;
        let plan_cfg = cuda_core::simt::LaunchConfig { grid_dim: (blocks.div_ceil(128).clamp(1, 65535) as u32, 1, 1),
            block_dim: (128, 1, 1), shared_mem_bytes: 0 };
        let aligned = std::env::var_os("PACKED_LEAF_ALIGNED").is_some();
        let frame_policy = match policy { MergeFrames::ExactCodes => 1, MergeFrames::ZeroPlain => 2, _ => 0 };
        let sizing = if std::env::var_os("PACKED_LEAF_SIZING").is_some() {
            let timing = Some(cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT);
            Some([pool.context().new_event(timing)?, pool.context().new_event(timing)?, pool.context().new_event(timing)?])
        } else { None };
        if let Some(e) = events { e.0[0].record(pool.stream())?; }
        // Fence partitioning is part of the layout kernel.
        if let Some(e) = events { e.0[1].record(pool.stream())?; }
        if tiled {
            let cut_ctas = (blocks + 1).div_ceil(128) as u32;
            let cfg = cuda_core::simt::LaunchConfig { grid_dim: (cut_ctas + groups.div_ceil(4) as u32, 1, 1), ..plan_cfg };
            module.plan_bounds::<W>(pool.stream(), cfg, source, other.view(), cuts, summaries, cut_ctas, retired, frame_policy)?;
            module.plan_team::<W>(pool.stream(), cuda_core::simt::LaunchConfig { grid_dim: (tile_count as u32, 1, 1), shared_mem_bytes: 104, ..plan_cfg },
                source, other.view(), cuts, summaries, plans, frames, tiles, retired, total, aligned, frame_policy)?;
        } else {
            module.plan::<W>(pool.stream(), plan_cfg, owner.view(), other.view(), plans, frames, tiles,
                exceptions, aligned, frame_policy)?;
        }
        if !fuse {
            if let Some(e) = &sizing { e[0].record(pool.stream())?; }
            module.tile_offsets(pool.stream(), &module.prepare_tile_offsets(LaunchConfig1D::new(1, 256, 128))?,
                tiles, tile_count as u64, exceptions, if tiled { 0 } else { blocks as u64 }, total)?;
        }
        if let Some(e) = &sizing { e[1].record(pool.stream())?; }
        if let Some(e) = events { e.0[2].record(pool.stream())?; }
        let started = sizing.as_ref().map(|_| std::time::Instant::now());
        let mut totals = [0u64; 3];
        {
            crate::phases::admission_scope!(c"admit/merge-sizing");
            download(&pool, total, &mut totals)?;
        }
        let fold_splices = tiled && totals[2] & 3 == 3 && (totals[2] >> 32) * 2 < blocks as u64;
        let writers = totals[2] as u32 & if fold_splices { 6 } else { 7 };
        if let (Some(e), Some(started)) = (&sizing, started) {
            let blocked = started.elapsed();
            e[2].record(pool.stream())?;
            pool.stream().synchronize()?;
            let scan_ns = if fuse { 0 } else { (e[0].elapsed_ms(&e[1])? as f64 * 1e6) as u64 };
            eprintln!("LEAF_SIZING rows={} words={} blocks={} launches={} fused_scan={} scan_ns={} readback_ns={} host_blocked_ns={}",
                owner.rows + other.rows, totals[0], totals[1], 1 + u32::from(!fuse) + u32::from(tiled) + writers.count_ones(), fuse,
                scan_ns, (e[1].elapsed_ms(&e[2])? as f64 * 1e6) as u64,
                blocked.as_nanos());
        }
        let output_blocks = totals[1] as usize;
        if output_blocks > u32::MAX as usize { return Err("leaf row directory exceeds u32 block addresses".into()); }
        let result = Self { pool: pool.clone(), modules: owner.modules.clone(), plain: None,
            payload: Buffer::uninitialized(pool.clone(), totals[0] as usize)?,
            headers: Buffer::uninitialized(pool.clone(), output_blocks)?,
            row_directory: Buffer::uninitialized(pool.clone(), (owner.rows + other.rows).div_ceil(256))?,
            frames: Buffer::uninitialized(pool.clone(), output_blocks * columns)?,
            fences: Buffer::uninitialized(pool.clone(), output_blocks)?,
            fields: schema,
            packing: owner.packing.clone(), rows: owner.rows + other.rows, block_rows: owner.block_rows, variable: true };
        let cfg = cuda_core::simt::LaunchConfig { grid_dim: (blocks.div_ceil((WRITE_THREADS / SPLICE_LANES) as usize).clamp(1, 65535) as u32, 1, 1),
            block_dim: (WRITE_THREADS, 1, 1), shared_mem_bytes: 0 };
        let launch = |path| -> Result<()> {
            if writers & 1 != 0 {
                let max_rows = if owner.variable { LEAF_CAP as u32 } else { owner.block_rows.min(LEAF_CAP as u32) };
                let stride = max_rows.div_ceil(2) + 2; // <=32 bits per owner row plus two guard words
                let shared = (WRITE_THREADS / SPLICE_LANES) * stride * 8;
                module.write::<W>(pool.stream(), cuda_core::simt::LaunchConfig { shared_mem_bytes: shared, ..cfg },
                    source, other.view(), plans, tiles, result.view(), stride, path, tiled)?;
            }
            if tiled {
                let leaf_cfg = cuda_core::simt::LaunchConfig { grid_dim: (output_blocks.clamp(1, 65535) as u32, 1, 1), ..cfg };
                if writers & 2 != 0 {
                    module.write_leaf_compact::<W>(pool.stream(), cuda_core::simt::LaunchConfig { shared_mem_bytes: LEAF_CAP as u32 * 16, ..leaf_cfg },
                        source, other.view(), plans, tiles, frames, result.view(), path, fold_splices)?;
                }
                if writers & 4 != 0 {
                    module.write_leaf_wide::<W>(pool.stream(), cuda_core::simt::LaunchConfig { shared_mem_bytes: LEAF_CAP as u32 * 8, ..leaf_cfg },
                        source, other.view(), plans, tiles, frames, result.view(), path)?;
                }
                return Ok(());
            }
            let general = cuda_core::simt::LaunchConfig { grid_dim: (blocks.div_ceil(8).clamp(1, 65535) as u32, 1, 1), ..cfg };
            if writers & 2 != 0 {
                let compact_cfg = cuda_core::simt::LaunchConfig { shared_mem_bytes: LEAF_CAP as u32 * 16, ..general };
                if output_blocks == blocks {
                    module.write_unsplit::<W>(pool.stream(), compact_cfg, owner.view(), other.view(), plans,
                        tiles, exceptions, frames, result.view(), path)?;
                } else {
                    module.write_compact::<W>(pool.stream(), compact_cfg, owner.view(), other.view(), plans,
                        tiles, exceptions, frames, result.view(), path)?;
                }
            }
            if writers & 4 != 0 {
                module.write_wide::<W>(pool.stream(), cuda_core::simt::LaunchConfig { shared_mem_bytes: LEAF_CAP as u32 * 8, ..general },
                    owner.view(), other.view(), plans, tiles, exceptions, frames, result.view(), path)?;
            }
            Ok(())
        };
        if let Some(e) = events { e.0[3].record(pool.stream())?; }
        launch(0)?;
        if let Some(e) = events { e.0[4].record(pool.stream())?; }
        if matches!(policy, MergeFrames::LeafPaths) {
            let timing = Some(cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT);
            let begin = pool.context().new_event(timing)?;
            let end = pool.context().new_event(timing)?;
            for path in 1..=3 {
                begin.record(pool.stream())?;
                launch(path)?;
                end.record(pool.stream())?;
                pool.stream().synchronize()?;
                eprintln!("LEAF_PATH_TIME rows={} path={} gpu_ns={}", result.rows, path,
                    (begin.elapsed_ms(&end)? as f64 * 1e6) as u64);
            }
        }
        if matches!(policy, MergeFrames::LeafPaths) {
            let mut host_plans = vec![Leaf::default(); blocks];
            let mut bases = vec![[0u64; 2]; tile_count];
            download(&pool, plans, &mut host_plans)?;
            download(&pool, tiles, &mut bases)?;
            let mut plans = host_plans;
            for (i, p) in plans.iter_mut().enumerate() {
                let tile = i / if tiled { 4 } else { 32 };
                p.offset += bases[tile][0];
                p.block += bases[tile][1];
            }
            let mut copied = 0u64;
            let mut encoded = 0u64;
            let mut reframed = 0u64;
            let mut untouched = 0u64;
            let mut widen = 0u64;
            let mut path_rows = [0u64; 3];
            let mut path_bytes = [0u64; 3];
            for p in &plans {
                let incoming = p.end - p.begin;
                let path = if incoming == 0 && p.keep != 0 { 0 } else if p.keep != 0 { 1 } else { 2 };
                path_rows[path] += p.rows;
                let leaves = p.rows.div_ceil(LEAF_CAP);
                path_bytes[path] += ((leaves - p.extra_rows as u64) * (p.split_rows as u64 * p.bits as u64).div_ceil(64)
                    + p.extra_rows as u64 * ((p.split_rows as u64 + 1) * p.bits as u64).div_ceil(64)) * 8;
                encoded += incoming;
                if p.keep != 0 { copied += p.rows - incoming; }
                else { reframed += p.rows - incoming; widen += 1; }
                untouched += (incoming == 0) as u64;
            }
            eprintln!("LEAF_PATHS rows={} blocks={} untouched={} widened={} copied={} encoded={} reframed={} output_blocks={} payload_bytes={} copy_rows={} fit_rows={} wide_rows={} copy_output_bytes={} fit_output_bytes={} wide_output_bytes={}",
                result.rows, blocks, untouched, widen, copied, encoded, reframed, output_blocks, result.payload_bytes(),
                path_rows[0], path_rows[1], path_rows[2], path_bytes[0], path_bytes[1], path_bytes[2]);
            if std::env::var_os("PACKED_COPY_BOUND").is_some() {
                let source_headers = owner.headers.to_host()?;
                let mut offsets = Vec::with_capacity(blocks);
                let mut words = 0u64;
                for (block, (p, h)) in plans.iter().zip(&source_headers).enumerate() {
                    offsets.push(words);
                    let next = plans.get(block + 1).map_or(totals[0], |next| next.offset);
                    words += (next - p.offset).max(h.words() + 1);
                }
                let destinations = Buffer::from_host(pool.clone(), &offsets)?;
                let scratch = Buffer::<u64>::zeroed(pool.clone(), words as usize)?;
                let eviction = Buffer::<u64>::uninitialized(pool.clone(), 32 * 1024 * 1024)?;
                module.evict(pool.stream(), &module.prepare_evict(config(eviction.len()))?,
                    eviction.ptr(), eviction.len() as u64)?;
                pool.stream().synchronize()?;
                let timing = Some(cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT);
                let begin = pool.context().new_event(timing)?;
                let end = pool.context().new_event(timing)?;
                begin.record(pool.stream())?;
                module.copy_owner_bound(pool.stream(), &module.prepare_copy_owner_bound(
                    LaunchConfig1D::new(blocks.max(1).min(65535) as u32, 64, 0))?,
                    owner.headers.ptr(), owner.payload.ptr(), destinations.ptr(), scratch.ptr(), blocks as u64, 13)?;
                end.record(pool.stream())?;
                pool.stream().synchronize()?;
                let ns = (begin.elapsed_ms(&end)? as f64 * 1e6) as u64;
                let output_bytes = source_headers.iter().filter(|h| h.words() != 0).map(|h| (h.words() + 1) * 8).sum::<u64>();
                eprintln!("LEAF_COPY_BOUND rows={} owner_rows={} input_bytes={} output_bytes={} gpu_ns={} gb_s={}",
                    result.rows, owner.rows, owner.payload_bytes(), output_bytes, ns,
                    (owner.payload_bytes() as u64 + output_bytes) as f64 / ns as f64);
                let source = owner.payload.to_host()?;
                let actual = scratch.to_host()?;
                for (block, h) in source_headers.iter().enumerate() {
                    if h.words() == 0 { continue; }
                    for word in 0..=h.words() {
                        let low = if word < h.words() { source[(h.offset + word) as usize] << 13 } else { 0 };
                        let high = if word != 0 { source[(h.offset + word - 1) as usize] >> 51 } else { 0 };
                        if actual[(offsets[block] + word) as usize] != low | high {
                            return Err("word-shift copy bound oracle mismatch".into());
                        }
                    }
                }
            }
        }
        Ok(result)
    }
}

#[cuda_host::cuda_module]
pub(super) mod leaf_kernels {
    use super::{Leaf, LEAF_CAP, SPLICE_LANES, WRITE_THREADS};
    use super::super::{device, Frame, Header, PackedView};
    use super::super::merge::merge_kernels::write_fence;
    use crate::{PackedField, Word};
    use cuda_device::{kernel, launch_bounds, launch_contract, thread, DynamicSharedArray};

    unsafe fn envelope<W: Word>(view: PackedView<W>, begin: u64, end: u64, step: u64, field: PackedField, column: u32, policy: u32) -> (u64, u64) {
        let mut low = u64::MAX;
        let mut high = 0;
        if !view.plain.is_null() {
            if policy == 2 && begin < end {
                return (0, if field.bits == 0 { 0 } else { u64::MAX >> (64 - field.bits) });
            }
            let mut i = begin;
            while i < end {
                let value = (*view.plain.add(i as usize)).extract(field.shift, field.bits);
                low = low.min(value); high = high.max(value); i += step;
            }
        } else if begin < end {
            let mut block = device::row_block(view, begin);
            let last = device::row_block(view, end - 1);
            while block <= last {
                let h = *view.headers.add(block as usize);
                let f = if h.encoding == 0 { Frame::new(0, field.bits) }
                    else { *view.frames.add((block * view.columns as u64 + column as u64) as usize) };
                if policy != 1 {
                    let mask = if f.width() == 0 { 0 } else { u64::MAX >> (64 - f.width()) };
                    let maximum = if field.bits == 0 { 0 } else { u64::MAX >> (64 - field.bits) };
                    low = low.min(f.base); high = high.max(f.base.saturating_add(mask).min(maximum));
                } else {
                    let mut shift = if h.encoding == 0 { field.shift } else { h.bits };
                    if h.encoding != 0 {
                        let mut col = 0;
                        while col <= column { shift -= (*view.frames.add((block * view.columns as u64 + col as u64) as usize)).width(); col += 1; }
                    }
                    let stride = if h.encoding == 0 { W::BITS } else { h.bits };
                    let mut i = begin.max(h.first);
                    let stop = end.min(h.first + h.rows as u64);
                    let mut min = u64::MAX;
                    let mut max = 0;
                    while i < stop {
                        let code = device::read_code(view.payload, h.offset * 64 + (i - h.first) * stride as u64 + shift as u64, f.width());
                        min = min.min(code); max = max.max(code); i += 1;
                    }
                    // Only extrema cross into metadata; no row value is materialized.
                    low = low.min(f.base + min); high = high.max(f.base + max);
                }
                block += 1;
            }
        }
        (low, high)
    }
    /// Return one owner's unchanged/split geometry and final frames. The old
    /// envelope is retained unless an actual incoming value escapes it.
    unsafe fn plan_owner<W: Word>(owner: PackedView<W>, other: PackedView<W>, block: u64,
        begin: u64, end: u64, frames: *mut Frame, aligned: bool, policy: u32) -> Leaf {
        let h = *owner.headers.add(block as usize);
        let mut bits = 0;
        let mut keep = true;
        let mut col = 0;
        while col < owner.columns {
            let field = *owner.fields.add(col as usize);
            let frame_ptr = (block * owner.columns as u64 + col as u64) as usize;
            let old = if h.encoding == 0 && begin < end { Frame::new(0, field.bits) }
                else { *owner.frames.add(frame_ptr) };
            let mask = if old.width() == 0 { 0 } else { u64::MAX >> (64 - old.width()) };
            let (mut low, mut high) = envelope(other, begin, end, 1, field, col, policy);
            if policy == 0 && other.plain.is_null() && begin < end
                && (low < old.base || high.saturating_sub(old.base) > mask) {
                (low, high) = envelope(other, begin, end, 1, field, col, 1);
            }
            let frame = widened(old, field, low, high, aligned);
            *frames.add(frame_ptr) = frame.after(bits);
            bits += frame.width();
            keep &= frame.base == old.base && frame.width() == old.width();
            col += 1;
        }
        finish_plan::<W>(h, begin, end, bits, keep)
    }
    fn widened(old: Frame, field: PackedField, low: u64, high: u64, aligned: bool) -> Frame {
        let mask = if old.width() == 0 { 0 } else { u64::MAX >> (64 - old.width()) };
        if low >= old.base && high.saturating_sub(old.base) <= mask { return old; }
        let maximum = if field.bits == 0 { 0 } else { u64::MAX >> (64 - field.bits) };
        let low = low.min(old.base);
        let high = high.max(old.base.saturating_add(mask).min(maximum));
        let width = if aligned { 64 - (low ^ high).leading_zeros() } else { 64 - (high - low).leading_zeros() };
        let base = if aligned { if width == 64 { 0 } else { (low >> width) << width } } else { low };
        Frame::new(base, width)
    }
    fn finish_plan<W: Word>(h: Header, begin: u64, end: u64, bits: u32, keep: bool) -> Leaf {
        let encoding = if begin == end { h.encoding } else { (bits < W::BITS) as u32 };
        let rows = h.rows as u64 + (end - begin);
        let leaves = rows.div_ceil(LEAF_CAP);
        let split_rows = rows / leaves;
        Leaf { begin, end, rows, encoding, bits: if encoding == 0 { W::BITS } else { bits },
            keep: (encoding == h.encoding && (encoding == 0 || keep)) as u32,
            split_rows: split_rows as u32, extra_rows: (rows - split_rows * leaves) as u32, offset: 0, block: 0 }
    }
    fn geometry(p: Leaf) -> [u64; 2] {
        let leaves = p.rows.div_ceil(LEAF_CAP);
        let words = (leaves - p.extra_rows as u64) * (p.split_rows * p.bits).div_ceil(64) as u64
            + p.extra_rows as u64 * ((p.split_rows + 1) * p.bits).div_ceil(64) as u64;
        [words, leaves]
    }
    fn residual_range(code: u64, from: Frame, field: PackedField, policy: u32) -> [u64; 2] {
        if policy == 2 {
            let mask = if from.width() == 0 { 0 } else { u64::MAX >> (64 - from.width()) };
            let maximum = if field.bits == 0 { 0 } else { u64::MAX >> (64 - field.bits) };
            [0, mask.min(maximum - from.base)]
        } else { [code, code] }
    }
    // Distances below/above an inherited base, without rebasing a row
    // to the run-wide identity frame. The old base itself is retained.
    fn displacement(base: u64, from: Frame, codes: [u64; 2]) -> [u64; 2] {
        if from.base >= base { [0, codes[1] + (from.base - base)] } else {
            let shift = base - from.base;
            [shift.saturating_sub(codes[0]), codes[1].saturating_sub(shift)]
        }
    }
    fn warp_range(mut low: u64, mut high: u64) -> [u64; 2] {
        let mut delta = 16;
        while delta != 0 {
            low = low.min(cuda_device::warp::shuffle_down_u64(low, delta));
            high = high.max(cuda_device::warp::shuffle_down_u64(high, delta));
            delta /= 2;
        }
        [low, high]
    }
    // One fence search per owner and one pair of extrema per full incoming
    // warp and column. Both sets of jobs are independent, even under skew.
    #[kernel]
    #[launch_bounds(128, 8)]
    pub fn plan_bounds<W: Word>(owner: PackedView<W>, other: PackedView<W>, cuts: *mut u64,
        summaries: *mut [u64; 2], cut_ctas: u32, retired: *mut u32, policy: u32) {
        let cta = thread::blockIdx_x();
        let tid = thread::threadIdx_x();
        unsafe {
            if cta < cut_ctas {
                let block = cta as u64 * 128 + tid as u64;
                if block == 0 && !retired.is_null() { *retired = 0; }
                if block <= owner.blocks {
                    *cuts.add(block as usize) = if block == 0 { 0 } else if block == owner.blocks { other.rows }
                        else { device::lower_bound(other, *owner.fences.add(block as usize)) };
                }
                return;
            }
            let row = (cta - cut_ctas) as u64 * 128 + tid as u64;
            let group = row / 32;
            if group >= other.rows / 32 { return; }
            let mut cursor = device::source_codes(other, row, owner.fields, owner.columns);
            let mut col = 0;
            while let Some((code, from, field)) = cursor.next() {
                let [mut lo, mut hi] = residual_range(code, from, field, policy);
                let mut base = from.base;
                let mut d = 16;
                while d != 0 {
                    let b = cuda_device::warp::shuffle_down_u64(base, d);
                    let l = cuda_device::warp::shuffle_down_u64(lo, d);
                    let r = cuda_device::warp::shuffle_down_u64(hi, d);
                    let common = base.min(b);
                    lo = (lo + (base - common)).min(l + (b - common));
                    hi = (hi + (base - common)).max(r + (b - common));
                    base = common;
                    d /= 2;
                }
                if tid & 31 == 0 {
                    // Only the two reduced extrema cross into metadata.
                    *summaries.add((group * owner.columns as u64 + col) as usize) = [base + lo, base + hi];
                }
                col += 1;
            }
        }
    }
    unsafe fn team_plan<W: Word>(owner: PackedView<W>, other: PackedView<W>, cuts: *const u64, summaries: *const [u64; 2],
        plans: *mut Leaf, frames: *mut Frame, aligned: bool, policy: u32, block: u64, lane: u64) -> [u64; 3] {
        if block >= owner.blocks { return [0; 3]; }
        unsafe {
            let begin = *cuts.add(block as usize);
            let end = *cuts.add(block as usize + 1);
            let h = *owner.headers.add(block as usize);
            let groups_begin = begin.div_ceil(32);
            let groups_end = end / 32;
            let head = begin + lane;
            let tail = (groups_end * 32).max((groups_begin * 32).min(end)) + lane;
            let head_valid = head < end.min(groups_begin * 32);
            let tail_valid = tail < end;
            // Never construct a cursor to a sentinel row. The value from an
            // inactive lane is ignored and no row data is read there.
            let mut head_codes = if head_valid { Some(device::source_codes(other, head, owner.fields, owner.columns)) } else { None };
            let mut tail_codes = if tail_valid { Some(device::source_codes(other, tail, owner.fields, owner.columns)) } else { None };
            let mut bits = 0;
            let mut keep = true;
            let mut col = 0;
            while col < owner.columns {
                let field = *owner.fields.add(col as usize);
                let i = (block * owner.columns as u64 + col as u64) as usize;
                let old = if h.encoding == 0 && begin < end { Frame::new(0, field.bits) }
                    else { *owner.frames.add(i) };
                let mut below = 0;
                let mut above = 0;
                if policy == 2 && !other.plain.is_null() && begin < end {
                    below = old.base;
                    above = (if field.bits == 0 { 0 } else { u64::MAX >> (64 - field.bits) }) - old.base;
                } else {
                    if let Some(c) = &mut head_codes {
                        let (x, from, _) = c.next().unwrap();
                        [below, above] = displacement(old.base, from, residual_range(x, from, field, policy));
                    }
                    if let Some(c) = &mut tail_codes {
                        let (x, from, _) = c.next().unwrap();
                        let [l, r] = displacement(old.base, from, residual_range(x, from, field, policy));
                        below = below.max(l); above = above.max(r);
                    }
                    let mut g = groups_begin + lane;
                    while g < groups_end {
                        let [l, r] = *summaries.add((g * owner.columns as u64 + col as u64) as usize);
                        below = below.max(old.base.saturating_sub(l));
                        above = above.max(r.saturating_sub(old.base)); g += 32;
                    }
                    let [inv, max] = warp_range(!below, above);
                    below = !inv; above = max;
                }
                if lane == 0 {
                    let frame = widened(old, field, old.base - below, old.base + above, aligned);
                    *frames.add(i) = frame.after(bits);
                    bits += frame.width();
                    keep &= frame.width() == old.width() && frame.base == old.base;
                }
                col += 1;
            }
            if lane == 0 {
                let mut p = finish_plan::<W>(h, begin, end, bits, keep);
                if !owner.plain.is_null() { p.keep = 0; }
                *plans.add(block as usize) = p;
                let [words, leaves] = geometry(p);
                let class = if sparse(p) { 1 } else if p.encoding != 0 && p.bits <= 64 { 2 } else { 4 };
                return [words, leaves, class];
            }
        }
        [0; 3]
    }
    #[kernel]
    #[launch_bounds(128, 8)]
    pub fn plan_team<W: Word>(owner: PackedView<W>, other: PackedView<W>, cuts: *const u64, summaries: *const [u64; 2],
        plans: *mut Leaf, frames: *mut Frame, tiles: *mut [u64; 2], retired: *mut u32, total: *mut u64, aligned: bool, policy: u32) {
        let tid = thread::threadIdx_x();
        let warp = (tid / 32) as usize;
        let tile = thread::blockIdx_x() as usize;
        let shared = DynamicSharedArray::<u64>::get().cast::<[u64; 3]>();
        unsafe {
            let counts = team_plan(owner, other, cuts, summaries, plans, frames, aligned, policy,
                (tile * 4 + warp) as u64, (tid & 31) as u64);
            if tid & 31 == 0 { *shared.add(warp) = counts; }
            thread::sync_threads();
            if tid < 4 {
                let mut words = 0;
                let mut leaves = 0;
                let mut flags = 0;
                let mut splices = 0;
                let mut i = 0;
                while i < tid as usize {
                    let [w, n, class] = *shared.add(i);
                    words += w; leaves += n; flags |= class; splices += (class == 1) as u64; i += 1;
                }
                let block = tile * 4 + tid as usize;
                if block < owner.blocks as usize {
                    let p = plans.add(block);
                    (*p).offset = words; (*p).block = leaves;
                }
                if tid == 3 {
                    let [w, n, class] = *shared.add(3);
                    // A tile contains at most four u32-sized owner runs plus
                    // the incoming intervals. Small-grid inputs fit well
                    // within 2^48 512-row splits; the upper bits are tags.
                    *tiles.add(tile) = [words + w, (leaves + n) | ((flags | class) << 48)
                        | ((splices + (class == 1) as u64) << 56)];
                    if !retired.is_null() {
                        let previous = cuda_device::atomic::DeviceAtomicU32::from_ptr(retired)
                            .fetch_add(1, cuda_device::atomic::AtomicOrdering::AcqRel);
                        *shared.add(4).cast::<u64>() = (previous + 1 == thread::gridDim_x()) as u64;
                    }
                }
            }
            if !retired.is_null() {
                thread::sync_threads();
                if *shared.add(4).cast::<u64>() != 0 {
                    // Each tile has one publishing lane, and the acq/rel
                    // chain ensures the final CTA sees all of their counts.
                    scan_tiles::<128>(tiles, thread::gridDim_x() as u64, tiles.cast(), 0, total);
                }
            }
        }
    }
    #[inline(always)]
    fn sparse(p: Leaf) -> bool {
        p.keep != 0 && p.encoding != 0 && p.bits <= 32 && p.rows <= LEAF_CAP && p.end - p.begin <= SPLICE_LANES as u64
    }
    #[kernel]
    #[launch_bounds(128, 8)]
    pub fn plan<W: Word>(owner: PackedView<W>, other: PackedView<W>, plans: *mut Leaf,
        frames: *mut Frame, tiles: *mut [u64; 2], exceptions: *mut [u32; 2], aligned: bool, policy: u32) {
        let lane = thread::threadIdx_x() & 31;
        let mut first = thread::index_1d().get() as u64 - lane as u64;
        while first < owner.blocks {
            let block = first + lane as u64;
            unsafe {
                let begin = if block == 0 { 0 } else if block < owner.blocks {
                    device::lower_bound(other, *owner.fences.add(block as usize))
                } else { other.rows };
                let mut end = cuda_device::warp::shuffle_down_u64(begin, 1);
                if lane == 31 {
                    end = if block + 1 < owner.blocks { device::lower_bound(other, *owner.fences.add(block as usize + 1)) }
                        else { other.rows };
                }
                let mut p = if block < owner.blocks { plan_owner(owner, other, block, begin, end, frames, aligned, policy) }
                    else { Leaf::default() };
                let [words, leaves] = geometry(p);
                let mut sum_words = words;
                let mut sum_blocks = leaves;
                let mut d = 1;
                while d < 32 {
                    let a = cuda_device::warp::shuffle_up_u64(sum_words, d);
                    let b = cuda_device::warp::shuffle_up_u64(sum_blocks, d);
                    if lane >= d { sum_words += a; sum_blocks += b; }
                    d *= 2;
                }
                if block < owner.blocks {
                    p.offset = sum_words - words;
                    p.block = sum_blocks - leaves;
                    *plans.add(block as usize) = p;
                }
                let general = block < owner.blocks && !sparse(p);
                let compact = p.bits <= 64 && p.encoding != 0;
                let x = cuda_device::warp::ballot(general && compact);
                let y = cuda_device::warp::ballot(general && !compact);
                if lane == 31 {
                    *tiles.add((first / 32) as usize) = [sum_words, sum_blocks];
                    *exceptions.add((first / 32) as usize) = [x, y];
                }
            }
            first += thread::gridDim_x() as u64 * 128;
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1), dynamic_shared_range = (128, 128))]
    pub fn tile_offsets(tiles: *mut [u64; 2], blocks: u64, exceptions: *const [u32; 2], owners: u64, total: *mut u64) {
        scan_tiles::<256>(tiles, blocks, exceptions, owners, total);
    }
    #[inline(always)]
    fn scan_tiles<const THREADS: u32>(tiles: *mut [u64; 2], blocks: u64, exceptions: *const [u32; 2], owners: u64, total: *mut u64) {
        let warps = THREADS / 32;
        let scratch = DynamicSharedArray::<u64>::get();
        let tid = thread::threadIdx_x();
        let lane = tid & 31;
        let warp = tid >> 5;
        let mut base = 0u64;
        let mut words_carry = 0u64;
        let mut blocks_carry = 0u64;
        let mut classes = 0;
        let mut splices = 0u32;
        while base < blocks {
            let index = base + tid as u64;
            let [words, leaves] = if index < blocks { unsafe { *tiles.add(index as usize) } } else { [0, 0] };
            if owners == 0 { classes |= ((leaves >> 48) & 7) as u32; splices += (leaves >> 56) as u32; }
            let leaves = if owners == 0 { leaves & ((1u64 << 48) - 1) } else { leaves };
            let mut sum_words = words;
            let mut sum_blocks = leaves;
            let mut distance = 1;
            while distance < 32 {
                let a = cuda_device::warp::shuffle_up_u64(sum_words, distance);
                let b = cuda_device::warp::shuffle_up_u64(sum_blocks, distance);
                if lane >= distance { sum_words += a; sum_blocks += b; }
                distance *= 2;
            }
            if lane == 31 { unsafe {
                *scratch.add(warp as usize) = sum_words;
                *scratch.add((warp + warps) as usize) = sum_blocks;
            } }
            thread::sync_threads();
            if warp == 0 {
                let mut a = if lane < warps { unsafe { *scratch.add(lane as usize) } } else { 0 };
                let mut b = if lane < warps { unsafe { *scratch.add((lane + warps) as usize) } } else { 0 };
                distance = 1;
                while distance < warps {
                    let x = cuda_device::warp::shuffle_up_u64(a, distance);
                    let y = cuda_device::warp::shuffle_up_u64(b, distance);
                    if lane >= distance { a += x; b += y; }
                    distance *= 2;
                }
                if lane < warps { unsafe { *scratch.add(lane as usize) = a; *scratch.add((lane + warps) as usize) = b; } }
            }
            thread::sync_threads();
            unsafe {
                if index < blocks {
                    if owners != 0 {
                        let [compact, wide] = *exceptions.add(index as usize);
                        let valid = (owners - index * 32).min(32) as u32;
                        classes |= ((compact | wide).count_ones() < valid) as u32
                            | (((compact != 0) as u32) << 1) | (((wide != 0) as u32) << 2);
                    }
                    let earlier = if warp == 0 { [0, 0] } else {
                        [*scratch.add(warp as usize - 1), *scratch.add((warp + warps - 1) as usize)]
                    };
                    *tiles.add(index as usize) = [words_carry + earlier[0] + sum_words - words,
                        blocks_carry + earlier[1] + sum_blocks - leaves];
                }
                words_carry += *scratch.add((warps - 1) as usize);
                blocks_carry += *scratch.add((2 * warps - 1) as usize);
            }
            thread::sync_threads();
            base += THREADS as u64;
        }
        let sparse = cuda_device::warp::ballot(classes & 1 != 0);
        let compact = cuda_device::warp::ballot(classes & 2 != 0);
        let wide = cuda_device::warp::ballot(classes & 4 != 0);
        if owners == 0 {
            let mut d = 16;
            while d != 0 { splices += cuda_device::warp::shuffle_down_u64(splices as u64, d) as u32; d /= 2; }
        }
        if lane == 0 { unsafe { *scratch.add(warp as usize) =
            (sparse != 0) as u64 | (((compact != 0) as u64) << 1) | (((wide != 0) as u64) << 2) | ((splices as u64) << 32); } }
        thread::sync_threads();
        if tid == 0 { unsafe {
            *total = words_carry;
            *total.add(1) = blocks_carry;
            let mut flags = 0;
            let mut i = 0;
            while i < warps as usize {
                let v = *scratch.add(i);
                flags = (flags | (v & 7)) + (v & !7);
                i += 1;
            }
            *total.add(2) = flags;
        } }
    }
    #[inline(always)]
    unsafe fn absolute_plan(plans: *const Leaf, tiles: *const [u64; 2], block: u64) -> Leaf {
        let mut p = *plans.add(block as usize);
        let base = *tiles.add((block / 32) as usize);
        p.offset += base[0];
        p.block += base[1];
        p
    }
    /// Rebase only the requested packed fragment; no row key is reconstructed.
    #[inline(never)]
    unsafe fn translated_fragment<W: Word>(source: PackedView<W>, index: u64, schema: PackedView<W>,
        frames: *const Frame, encoding: u32, bits: u32, start: u32, width: u32) -> u64 {
        let mut cursor = device::source_codes(source, index, schema.fields, schema.columns);
        let mut remaining = bits;
        let mut col = 0;
        let mut result = 0;
        while let Some((code, frame, field)) = cursor.next() {
            let target = if encoding == 0 { Frame::new(0, field.bits) }
                else { *frames.add(col) };
            let shift = if encoding == 0 { field.shift } else { remaining -= target.width(); remaining };
            let lo = start.max(shift);
            let hi = (start + width).min(shift + target.width());
            if lo < hi {
                let rebased = code.wrapping_add(frame.base.wrapping_sub(target.base));
                let mask = u64::MAX >> (64 - (hi - lo));
                result |= ((rebased >> (lo - shift)) & mask) << (lo - start);
            }
            col += 1;
        }
        result
    }
    fn split_header(p: Leaf, oh: Header, split: u64) -> Header {
        let extra = split.min(p.extra_rows as u64);
        let start = split * p.split_rows as u64 + extra;
        let short = (p.split_rows * p.bits).div_ceil(64) as u64;
        let long = ((p.split_rows + 1) * p.bits).div_ceil(64) as u64;
        Header { first: oh.first + p.begin + start, offset: p.offset + split * short + extra * (long - short),
            rows: p.split_rows + (split < p.extra_rows as u64) as u32, bits: p.bits, encoding: p.encoding, reserved: 0 }
    }
    /// Owner rows in the first k merged rows. Strict comparison keeps
    /// incoming-before-owner tie order. Only interior split boundaries need it.
    #[inline(never)]
    unsafe fn cut<W: Word, const TEAM: bool>(a: PackedView<W>, ai: u64, an: u64, b: PackedView<W>, bi: u64, bn: u64, k: u64) -> u64 {
        let mut lo = k.saturating_sub(bn);
        let mut hi = k.min(an);
        while lo < hi {
            if TEAM {
                let step = (hi - lo).div_ceil(32);
                let m = lo + (thread::threadIdx_x() & 31) as u64 * step;
                let stop = m >= hi || !device::compare_rows(a, ai + m, b, bi + k - m - 1, a.fields, a.columns).is_lt();
                let first = cuda_device::warp::ballot(stop).trailing_zeros() as u64;
                hi = hi.min(lo + first * step);
                if first != 0 { lo += (first - 1) * step + 1; }
                continue;
            }
            let m = (lo + hi) / 2;
            if device::compare_rows(a, ai + m, b, bi + k - m - 1, a.fields, a.columns).is_lt() {
                lo = m + 1;
            } else { hi = m; }
        }
        lo
    }
    /// Concatenate residuals in the destination frame, not the run-wide key.
    #[inline(never)]
    unsafe fn compact_code<W: Word, const NARROW: bool>(source: PackedView<W>, idx: u64, schema: PackedView<W>, target: *const Frame, bits: u32) -> u64 {
        let mut cursor = device::source_codes(source, idx, schema.fields, schema.columns);
        let mut code = 0;
        let mut remaining = bits;
        let mut col = 0;
        while let Some((x, from, _)) = cursor.next() {
            let to = *target.add(col);
            remaining -= to.width();
            if to.width() != 0 {
                let value = x.wrapping_add(from.base.wrapping_sub(to.base));
                code |= if NARROW { ((value as u32) << remaining) as u64 } else { value << remaining };
            }
            col += 1;
        }
        code
    }
    #[inline(always)]
    unsafe fn less<W: Word, const COMPACT: bool>(input: *const u64, an: u32, i: u32, j: u32,
        a: PackedView<W>, ai: u64, b: PackedView<W>, bi: u64) -> bool {
        if COMPACT { *input.add(i as usize) < *input.add((an + j) as usize) }
        else { device::compare_rows(a, ai + i as u64, b, bi + j as u64, a.fields, a.columns).is_lt() }
    }
    /// Each lane merges a disjoint contiguous stripe. Compact tiles carry
    /// common-frame codes; wide tiles carry bounded source indices.
    unsafe fn merge_tile<W: Word, const COMPACT: bool>(input: *const u64, merged: *mut u64, lane: u32,
        a: PackedView<W>, ai: u64, an: u32, b: PackedView<W>, bi: u64, bn: u32) {
        let n = an + bn;
        let step = n.div_ceil(WRITE_THREADS);
        let mut k = lane * step;
        let end = (k + step).min(n);
        if k >= end { return; }
        let mut lo = k.saturating_sub(bn);
        let mut hi = k.min(an);
        while lo < hi {
            let m = (lo + hi) / 2;
            if less::<W, COMPACT>(input, an, m, k - m - 1, a, ai, b, bi) { lo = m + 1; } else { hi = m; }
        }
        let mut i = lo;
        let mut j = k - i;
        while k < end {
            let owner = i < an && (j == bn || less::<W, COMPACT>(input, an, i, j, a, ai, b, bi));
            let v = if owner {
                let v = if COMPACT { *input.add(i as usize) } else { i as u64 };
                i += 1; v
            } else {
                let v = if COMPACT { *input.add((an + j) as usize) } else { (j | (1 << 31)) as u64 };
                j += 1; v
            };
            *merged.add(k as usize) = v;
            k += 1;
        }
    }
    #[cuda_device::device]
    unsafe fn direct_leaf<W: Word, const COMPACT: bool, const ONE: bool, const UNSPLIT: bool>(owner: PackedView<W>, other: PackedView<W>,
        block: u64, p: Leaf, start_split: u64, frames: *const Frame, out: PackedView<W>, path_filter: u32) {
        let lane = thread::threadIdx_x();
        let input = DynamicSharedArray::<u64>::get();
        let merged = if COMPACT { input.add(LEAF_CAP as usize) } else { input };
        let path = if p.begin == p.end && p.keep != 0 { 1 } else if p.keep != 0 { 2 } else { 3 };
        if path_filter != 0 && path_filter != path { return; }
        let oh = *owner.headers.add(block as usize);
        let local = PackedView { headers: owner.headers.add(block as usize),
            frames: owner.frames.add((block * owner.columns as u64) as usize),
            fences: owner.fences.add(block as usize), blocks: 1, block_rows: u32::MAX, ..owner };
        let target = frames.add((block * owner.columns as u64) as usize);
        let mut ai = oh.first;
        let mut bi = p.begin;
        let splits = if UNSPLIT { 1 } else { p.rows.div_ceil(LEAF_CAP) };
        let mut split = start_split;
        if ONE && split != 0 {
            let k = split * p.split_rows as u64 + split.min(p.extra_rows as u64);
            let a = cut::<W, ONE>(local, ai, oh.rows as u64, other, bi, p.end - bi, k);
            ai += a; bi += k - a;
        }
        while split < splits {
            let dest = p.block + split;
            let h = split_header(p, oh, split);
            let left = oh.first + oh.rows as u64 - ai;
            let an = if UNSPLIT || split + 1 == splits { left } else {
                cut::<W, ONE>(local, ai, left, other, bi, p.end - bi, h.rows as u64)
            } as u32;
            let bn = h.rows - an;
            let copy = bn == 0 && p.keep != 0;
            if !copy {
                if COMPACT {
                    let mut r = lane;
                    while r < an {
                        *input.add(r as usize) = if p.keep != 0 {
                            device::read_code(owner.payload.add(oh.offset as usize),
                                (ai - oh.first + r as u64) * h.bits as u64, h.bits)
                        } else { compact_code::<W, false>(local, ai + r as u64, owner, target, h.bits) };
                        r += WRITE_THREADS;
                    }
                    r = lane;
                    while r < bn {
                        *input.add((an + r) as usize) = compact_code::<W, false>(other, bi + r as u64, owner, target, h.bits);
                        r += WRITE_THREADS;
                    }
                    thread::sync_threads();
                }
                merge_tile::<W, COMPACT>(input, merged, lane, local, ai, an, other, bi, bn);
                thread::sync_threads();
            }
            if lane == 0 {
                *(out.headers as *mut Header).add(dest as usize) = h;
                let first_other = !copy && if COMPACT {
                    an == 0 || (bn != 0 && *input.add(an as usize) <= *input)
                } else { *merged >> 31 != 0 };
                let (source, index) = if first_other { (other, bi) } else { (local, ai) };
                write_fence(source, index, out, dest);
            }
            let bucket = h.first.div_ceil(256) + lane as u64;
            if bucket * 256 < h.first + h.rows as u64 {
                *(out.row_directory as *mut u32).add(bucket as usize) = dest as u32;
            }
            let mut col = lane as u64;
            while col < out.columns as u64 {
                *(out.frames as *mut Frame).add((dest * out.columns as u64 + col) as usize) =
                    (*target.add(col as usize)).published::<W>(h, out.fields, col as usize);
                col += WRITE_THREADS as u64;
            }
            let bits = h.rows * h.bits;
            let mut wi = lane;
            while wi < bits.div_ceil(64) {
                let begin = wi * 64;
                let end = (begin + 64).min(bits);
                let word = if copy {
                    device::read_bits(owner.payload.add(oh.offset as usize), (ai - oh.first) * h.bits as u64 + begin as u64, end - begin)
                } else if COMPACT {
                    let mut r = begin / h.bits;
                    let skip = begin - r * h.bits;
                    let mut w = *merged.add(r as usize) >> skip;
                    let mut shift = h.bits - skip;
                    r += 1;
                    while shift < 64 && r < h.rows {
                        w |= *merged.add(r as usize) << shift;
                        r += 1; shift += h.bits;
                    }
                    w
                } else {
                    let mut bit = begin;
                    let mut w = 0;
                    while bit < end {
                        let r = bit / h.bits;
                        let skip = bit - r * h.bits;
                        let width = (h.bits - skip).min(end - bit);
                        let source = *merged.add(r as usize) as u32;
                        let fragment = if source >> 31 == 0 && p.keep != 0 {
                            device::read_bits(owner.payload.add(oh.offset as usize),
                                (ai - oh.first + source as u64) * h.bits as u64 + skip as u64, width)
                        } else {
                            let (view, idx) = if source >> 31 != 0 { (other, bi + (source & 0x7fffffff) as u64) }
                                else { (local, ai + source as u64) };
                            translated_fragment(view, idx, owner, target, h.encoding, h.bits, skip, width)
                        };
                        w |= fragment << (bit - begin);
                        bit += width;
                    }
                    w
                };
                *(out.payload as *mut u64).add((h.offset + wi as u64) as usize) = word;
                wi += WRITE_THREADS;
            }
            thread::sync_threads();
            if ONE || UNSPLIT { return; }
            ai += an as u64; bi += bn as u64;
            split += 1;
        }
    }
    #[cuda_device::device]
    unsafe fn direct<W: Word, const COMPACT: bool, const UNSPLIT: bool>(owner: PackedView<W>, other: PackedView<W>, plans: *const Leaf,
        tiles: *const [u64; 2], exceptions: *const [u32; 2], frames: *const Frame, out: PackedView<W>, path_filter: u32) {
        let mut chunk = thread::blockIdx_x() as u64;
        while chunk * 8 < owner.blocks {
            let tile = chunk / 4;
            let mut mask = (*exceptions.add(tile as usize))[if COMPACT { 0 } else { 1 }] & (0xff << (8 * (chunk & 3)));
            while mask != 0 {
                let block = tile * 32 + mask.trailing_zeros() as u64;
                mask &= mask - 1;
                let p = absolute_plan(plans, tiles, block);
                direct_leaf::<W, COMPACT, false, UNSPLIT>(owner, other, block, p, 0, frames, out, path_filter);
            }
            chunk += thread::gridDim_x() as u64;
        }
    }
    unsafe fn write_by_leaf<W: Word, const COMPACT: bool>(owner: PackedView<W>, other: PackedView<W>, plans: *const Leaf,
        bases: *const [u64; 2], frames: *const Frame, out: PackedView<W>, path_filter: u32, fold_splices: bool) {
        let mut dest = thread::blockIdx_x() as u64;
        while dest < out.blocks {
            unsafe {
                // First split is addressed by owner; only additional splits
                // search the exclusive prefix of extra splits (base - owner).
                let (block, split) = if dest < owner.blocks { (dest, 0) } else {
                    let extra = dest - owner.blocks;
                    let mut lo = 0;
                    let mut hi = owner.blocks;
                    while lo + 1 < hi {
                        let mid = (lo + hi) / 2;
                        let start = (*bases.add((mid / 4) as usize))[1] + (*plans.add(mid as usize)).block;
                        if start - mid <= extra { lo = mid; } else { hi = mid; }
                    }
                    let start = (*bases.add((lo / 4) as usize))[1] + (*plans.add(lo as usize)).block;
                    (lo, 1 + extra - (start - lo))
                };
                let mut p = *plans.add(block as usize);
                if (fold_splices || !sparse(p)) && (p.bits <= 64 && p.encoding != 0) == COMPACT {
                    let [words, leaves] = *bases.add((block / 4) as usize);
                    p.offset += words; p.block += leaves;
                    direct_leaf::<W, COMPACT, true, false>(owner, other, block, p, split, frames, out, path_filter);
                }
            }
            dest += thread::gridDim_x() as u64;
        }
    }
    #[kernel]
    #[launch_bounds(128, 8)]
    pub fn write_leaf_compact<W: Word>(owner: PackedView<W>, other: PackedView<W>, plans: *const Leaf,
        bases: *const [u64; 2], frames: *const Frame, out: PackedView<W>, path_filter: u32, fold_splices: bool) {
        unsafe { write_by_leaf::<W, true>(owner, other, plans, bases, frames, out, path_filter, fold_splices); }
    }
    #[kernel]
    #[launch_bounds(128, 8)]
    pub fn write_leaf_wide<W: Word>(owner: PackedView<W>, other: PackedView<W>, plans: *const Leaf,
        bases: *const [u64; 2], frames: *const Frame, out: PackedView<W>, path_filter: u32) {
        unsafe { write_by_leaf::<W, false>(owner, other, plans, bases, frames, out, path_filter, false); }
    }
    #[kernel]
    #[launch_bounds(128, 8)]
    pub fn write<W: Word>(owner: PackedView<W>, other: PackedView<W>, plans: *const Leaf,
        tiles: *const [u64; 2], out: PackedView<W>, stride: u32, path_filter: u32, tiled: bool) {
        let lane = thread::threadIdx_x() & (SPLICE_LANES - 1);
        let half = thread::threadIdx_x() & SPLICE_LANES;
        let mask = 0xffff << half;
        let mut block = (thread::index_1d().get() / SPLICE_LANES as usize) as u64;
        while block < owner.blocks {
            unsafe {
                let mut p = *plans.add(block as usize);
                let [words, leaves] = *tiles.add((block / if tiled { 4 } else { 32 }) as usize);
                p.offset += words; p.block += leaves;
                let n = (p.end - p.begin) as u32;
                if sparse(p) && (path_filter == 0 || path_filter == if n == 0 { 1 } else { 2 }) {
                    let oh = *owner.headers.add(block as usize);
                    let h = Header { first: oh.first + p.begin, offset: p.offset, rows: p.rows as u32,
                        bits: oh.bits, encoding: oh.encoding, reserved: 0 };
                    let target = owner.frames.add((block * owner.columns as u64) as usize);
                    let mut col = lane;
                    while col < owner.columns {
                        let mut frame = *target.add(col as usize);
                        #[cfg(feature = "packed-radix")]
                        if W::BITS <= 128 && owner.plain.is_null() && oh.encoding != 0 {
                            // Relocate a published field; bootstrap frames still have plan prefixes.
                            frame.end = oh.bits - frame.shift as u32;
                        }
                        *(out.frames as *mut Frame).add((p.block * out.columns as u64 + col as u64) as usize) =
                            frame.published::<W>(h, out.fields, col as usize);
                        col += SPLICE_LANES;
                    }
                    let bucket = h.first.div_ceil(256) + lane as u64;
                    if bucket * 256 < h.first + h.rows as u64 { *(out.row_directory as *mut u32).add(bucket as usize) = p.block as u32; }
                    if lane == 0 {
                        *(out.headers as *mut Header).add(p.block as usize) = h;
                        let mut fence = *owner.fences.add(block as usize);
                        // Partition assigned only the first owner any rows below its fence.
                        if block == 0 && n != 0 {
                            let first = if other.plain.is_null() { *other.fences } else { *other.plain };
                            if first.compare(&fence).is_lt() { fence = first; }
                        }
                        *(out.fences as *mut W).add(p.block as usize) = fence;
                    }
                    let source = owner.payload.add(oh.offset as usize);
                    let output = (out.payload as *mut u64).add(h.offset as usize);
                    let src_bits = oh.rows * h.bits;
                    let src_words = src_bits.div_ceil(64);
                    let stage = DynamicSharedArray::<u64>::get().add(((thread::threadIdx_x() / SPLICE_LANES) * stride) as usize);
                    let mut wi = lane;
                    while wi < src_words {
                        let w = *source.add(wi as usize);
                        if n == 0 { *output.add(wi as usize) = w; } else { *stage.add(wi as usize) = w; }
                        wi += SPLICE_LANES;
                    }
                    if n != 0 && h.bits != 0 {
                        if lane < 2 { *stage.add((src_words + lane) as usize) = 0; }
                        cuda_device::warp::sync_mask(mask);
                        let mut pos = u32::MAX - 64;
                        let mut code = 0u32;
                        if lane < n {
                            code = compact_code::<W, true>(other, p.begin + lane as u64, owner, target, h.bits) as u32;
                            let mut lo = 0;
                            let mut hi = oh.rows;
                            while lo < hi {
                                let m = (lo + hi) / 2;
                                let v = device::read_bits32(stage.cast(), (m * h.bits) as u64, h.bits);
                                if v < code { lo = m + 1; } else { hi = m; }
                            }
                            pos = (lo + lane) * h.bits;
                        }
                        let valid = h.rows * h.bits;
                        let words = valid.div_ceil(64);
                        let mut base = 0;
                        while base < words {
                            let wi = base + lane;
                            // Even tail lanes must participate in all exchanges in their team.
                            let begin = wi.min(words - 1) * 64;
                            let end = (begin + 64).min(valid);
                            let mut i = 0u32;
                            let mut step = SPLICE_LANES / 2;
                            while step != 0 {
                                if cuda_device::warp::shuffle_sync(mask, pos + h.bits, half + i + step - 1) <= begin { i += step; }
                                step >>= 1;
                            }
                            if cuda_device::warp::shuffle_sync(mask, pos + h.bits, half + SPLICE_LANES - 1) <= begin { i = SPLICE_LANES; }
                            let mut x = cuda_device::warp::shuffle_sync(mask, pos, half + (i & (SPLICE_LANES - 1)));
                            if i >= n { x = u32::MAX - 64; }
                            let owner_bit = begin - i * h.bits - begin.saturating_sub(x);
                            let src = stage.add((owner_bit >> 6) as usize);
                            let shift = owner_bit & 63;
                            let mut word = *src >> shift;
                            if shift != 0 { word |= *src.add(1) << (64 - shift); }
                            loop {
                                let c = cuda_device::warp::shuffle_sync(mask, code, half + (i & (SPLICE_LANES - 1)));
                                if x < end {
                                    let left = x.saturating_sub(begin);
                                    let skip = begin.saturating_sub(x);
                                    let low = (1u64 << left) - 1;
                                    word = (word & low) | ((word & !low) << (h.bits - skip)) | (((c >> skip) as u64) << left);
                                }
                                i += 1;
                                x = cuda_device::warp::shuffle_sync(mask, pos, half + (i & (SPLICE_LANES - 1)));
                                if i >= n { x = u32::MAX - 64; }
                                if cuda_device::warp::ballot_sync(mask, x < end) == 0 { break; }
                            }
                            if wi < words { *output.add(wi as usize) = word & (u64::MAX >> (64 - (end - begin))); }
                            base += SPLICE_LANES;
                        }
                        cuda_device::warp::sync_mask(mask);
                    }
                }
            }
            block += (thread::gridDim_x() * (WRITE_THREADS / SPLICE_LANES)) as u64;
        }
    }
    #[kernel]
    #[launch_bounds(128, 8)]
    pub fn write_compact<W: Word>(owner: PackedView<W>, other: PackedView<W>, plans: *const Leaf,
        tiles: *const [u64; 2], exceptions: *const [u32; 2], frames: *const Frame, out: PackedView<W>, path_filter: u32) {
        unsafe { direct::<W, true, false>(owner, other, plans, tiles, exceptions, frames, out, path_filter); }
    }
    #[kernel]
    #[launch_bounds(128, 8)]
    pub fn write_unsplit<W: Word>(owner: PackedView<W>, other: PackedView<W>, plans: *const Leaf,
        tiles: *const [u64; 2], exceptions: *const [u32; 2], frames: *const Frame, out: PackedView<W>, path_filter: u32) {
        unsafe { direct::<W, true, true>(owner, other, plans, tiles, exceptions, frames, out, path_filter); }
    }
    #[kernel]
    #[launch_bounds(128, 8)]
    pub fn write_wide<W: Word>(owner: PackedView<W>, other: PackedView<W>, plans: *const Leaf,
        tiles: *const [u64; 2], exceptions: *const [u32; 2], frames: *const Frame, out: PackedView<W>, path_filter: u32) {
        unsafe { direct::<W, false, false>(owner, other, plans, tiles, exceptions, frames, out, path_filter); }
    }
    #[kernel]
    #[launch_bounds(64)]
    #[launch_contract(domain = 1, block = (64, 1, 1))]
    pub fn copy_owner_bound(headers: *const Header, source: *const u64, offsets: *const u64,
        output: *mut u64, blocks: u64, shift: u32) {
        let mut block = thread::blockIdx_x() as u64;
        while block < blocks {
            unsafe {
                let h = *headers.add(block as usize);
                let destination = *offsets.add(block as usize);
                let mut word = thread::threadIdx_x() as u64;
                while h.words() != 0 && word < h.words() + (shift != 0) as u64 {
                    let low = if word < h.words() { *source.add((h.offset + word) as usize) << shift } else { 0 };
                    let high = if shift != 0 && word != 0 { *source.add((h.offset + word - 1) as usize) >> (64 - shift) } else { 0 };
                    *output.add((destination + word) as usize) = low | high;
                    word += 64;
                }
            }
            block += thread::gridDim_x() as u64;
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn evict(output: *mut u64, rows: u64) {
        let mut i = thread::index_1d().get() as u64;
        while i < rows {
            unsafe { *output.add(i as usize) = i.wrapping_mul(0x9e3779b97f4a7c15); }
            i += thread::gridDim_x() as u64 * 256;
        }
    }
}
