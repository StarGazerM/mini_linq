//! Untimed replay ablations: independently vary geometry and inherited frames.
use super::*;

impl<W: Word> PackedRun<W> {
    /// Re-encode a replay's original plain input, without exporting packed rows.
    /// `fixed=false, inherit=false` isolates variable geometry with exact frames;
    /// `fixed=true, inherit=true` unions inherited frames on fixed row boundaries.
    ///
    /// # Safety
    /// Input contains exactly this run's sorted unique keys in the same order.
    pub unsafe fn replay_reframe(&self, input: &Buffer<W>, fixed: bool, inherit: bool) -> Result<Self> {
        if self.plain.is_some() || input.len() < self.rows || !self.pool.shares_memory(input.pool()) {
            return Err("replay reframing requires a matching packed run and plain input".into());
        }
        let pool = self.pool.clone();
        let blocks = if fixed { self.rows.div_ceil(self.block_rows as usize) } else { self.headers.len() };
        let mut result = Self { pool: pool.clone(), modules: self.modules.clone(), plain: None,
            payload: Buffer::zeroed(pool.clone(), 0)?, headers: Buffer::uninitialized(pool.clone(), blocks)?,
            row_directory: Buffer::uninitialized(pool.clone(), if fixed { 0 } else { self.rows.div_ceil(256) })?,
            frames: Buffer::uninitialized(pool.clone(), blocks * self.packing.len())?,
            fences: Buffer::uninitialized(pool.clone(), blocks)?,
            fields: self.fields.clone(),
            packing: self.packing.clone(), rows: self.rows, block_rows: self.block_rows, variable: !fixed };
        let module = geometry_kernels::load(pool.context())?;
        let cfg = cuda_core::simt::LaunchConfig { grid_dim: (blocks.max(1).min(65535) as u32, 1, 1), block_dim: (256, 1, 1), shared_mem_bytes: 4096 };
        module.analyze::<W>(pool.stream(), cfg, input.ptr(), self.view(), result.view(), inherit,
            std::env::var_os("PACKED_LEAF_ALIGNED").is_some())?;
        let total = Buffer::uninitialized(pool.clone(), 1)?;
        let codec = &self.modules.codec;
        codec.offsets::<W>(pool.stream(), &codec.prepare_offsets::<W>(LaunchConfig1D::new(1, 256, 64))?,
            result.headers.ptr(), blocks as u64, total.ptr())?;
        result.payload = Buffer::uninitialized(pool.clone(), total.to_host()?[0] as usize)?;
        codec.pack::<W>(pool.stream(), &codec.prepare_pack::<W>(LaunchConfig1D::new(blocks.max(1).min(65535) as u32, 256, 0))?,
            input.ptr(), result.view(), result.payload.ptr())?;
        Ok(result)
    }
}

#[cuda_host::cuda_module]
mod geometry_kernels {
    use super::super::{device, Frame, Header, PackedView};
    use crate::Word;
    use cuda_device::{kernel, launch_bounds, thread, DynamicSharedArray};

    #[kernel]
    #[launch_bounds(256)]
    pub fn analyze<W: Word>(input: *const W, source: PackedView<W>, out: PackedView<W>, inherit: bool, aligned: bool) {
        let scratch = DynamicSharedArray::<u64>::get();
        let tid = thread::threadIdx_x() as u64;
        let mut block = thread::blockIdx_x() as u64;
        while block < out.blocks {
            unsafe {
                let first = if out.block_rows == 0 { (*source.headers.add(block as usize)).first }
                    else { block * out.block_rows as u64 };
                let count = if out.block_rows == 0 { (*source.headers.add(block as usize)).rows as u64 }
                    else { (out.rows - first).min(out.block_rows as u64) };
                if tid == 0 { *(out.fences as *mut W).add(block as usize) = *input.add(first as usize); }
                let mut total_bits = 0;
                let mut col = 0;
                while col < out.columns {
                    let field = *out.fields.add(col as usize);
                    let mut low = u64::MAX;
                    let mut high = 0;
                    if inherit {
                        if tid == 0 {
                            let mut b = device::row_block(source, first);
                            let last = device::row_block(source, first + count - 1);
                            while b <= last {
                                let h = *source.headers.add(b as usize);
                                let frame = if h.encoding == 0 { Frame::new(0, field.bits) }
                                    else { *source.frames.add((b * source.columns as u64 + col as u64) as usize) };
                                let mask = if frame.width() == 0 { 0 } else { u64::MAX >> (64 - frame.width()) };
                                let maximum = if field.bits == 0 { 0 } else { u64::MAX >> (64 - field.bits) };
                                low = low.min(frame.base);
                                high = high.max(frame.base.saturating_add(mask).min(maximum));
                                b += 1;
                            }
                        }
                    } else {
                        let mut i = tid;
                        while i < count {
                            let value = (*input.add((first + i) as usize)).extract(field.shift, field.bits);
                            low = low.min(value); high = high.max(value);
                            i += 256;
                        }
                        *scratch.add(tid as usize) = low;
                        *scratch.add(tid as usize + 256) = high;
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
                        low = *scratch; high = *scratch.add(256);
                    }
                    if tid == 0 {
                        let bits = if aligned { 64 - (low ^ high).leading_zeros() } else { 64 - (high - low).leading_zeros() };
                        let base = if aligned { if bits == 64 { 0 } else { (low >> bits) << bits } } else { low };
                        *(out.frames as *mut Frame).add((block * out.columns as u64 + col as u64) as usize) = Frame::new(base, bits).after(total_bits);
                        total_bits += bits;
                    }
                    thread::sync_threads();
                    col += 1;
                }
                if tid == 0 {
                    let packed = (count * total_bits as u64).div_ceil(64);
                    let plain = count * (W::BITS / 64) as u64;
                    *(out.headers as *mut Header).add(block as usize) = Header { first, rows: count as u32,
                        offset: 0, bits: if packed < plain { total_bits } else { W::BITS }, encoding: (packed < plain) as u32, reserved: 0 };
                }
                if out.block_rows == 0 {
                    let mut bucket = first.div_ceil(256) + tid;
                    while bucket * 256 < first + count {
                        *(out.row_directory as *mut u32).add(bucket as usize) = block as u32;
                        bucket += 256;
                    }
                }
            }
            block += thread::gridDim_x() as u64;
        }
    }
}
