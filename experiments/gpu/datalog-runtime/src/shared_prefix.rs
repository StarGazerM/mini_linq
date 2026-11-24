//! Physical operators for RoundShared: exact, snapshot-keyed hash ranges and
//! bounded, replayable witness materialization. No host tuple materialization.
use crate::{Buffer, CandidateArena, CandidateBatch, Packing, Pool, RadixLayout, RadixLayoutView, Result, Word};
use crate::radix_run::device::{decode_key, run};
use cuda_device::{atomic::{AtomicOrdering::Relaxed, DeviceAtomicU32}, device, thread};
use std::sync::Arc;

#[repr(C)]
#[derive(Copy, Clone)]
pub struct Directory { pub heads: *mut u32, pub rows: *mut u32, pub mask: u32 }
unsafe impl cuda_core::DeviceCopy for Directory {}

// Hash one to three leading columns. The caller must compare *all* keys in a bucket.
#[device]
pub fn shared_hash(a: u32, b: u32, c: u32, n: u32) -> u32 {
    if n == 1 { a } else { (a.wrapping_mul(0x9e3779b1) ^ b.wrapping_mul(0x85ebca77) ^ c.wrapping_mul(0xc2b2ae3d)).wrapping_mul(0x27d4eb2d) }
}
#[device]
pub fn shared_tid() -> usize { (thread::blockIdx_x() * 256 + thread::threadIdx_x()) as usize }
#[device]
pub fn shared_stride() -> usize { thread::gridDim_x() as usize * 256 }

#[cuda_device::cuda_module]
mod shared_directory_kernels {
    use super::*;
    use cuda_device::{kernel, launch_bounds, launch_contract};
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain=1,block=(256,1,1))]
    pub fn round_shared_build<W: Word + Copy>(view: RadixLayoutView<W,u64>, width: u32, keys: u32, dir: Directory) { unsafe {
        let mut offset = 0;
        let mut r = 0;
        while r < view.keys.run_count {
            let n = run(view.keys,r).keys_len as usize;
            let mut i = shared_tid();
            while i < n {
                let row = dir.rows.add((offset+i)*(width as usize+1));
                let mut col = 0;
                while col < width {
                    *row.add(1+col as usize) = decode_key(view,r,i as u64,col) as u32;
                    col += 1;
                }
                let b = if keys >= 2 { *row.add(2) } else { 0 };
                let c = if keys >= 3 { *row.add(3) } else { 0 };
                let hash = shared_hash(*row.add(1),b,c,keys);
                *row = DeviceAtomicU32::from_ptr(dir.heads.add((hash & dir.mask) as usize)).swap((offset+i+1) as u32, Relaxed);
                i += shared_stride();
            }
            offset += n;
            r += 1;
        }
    }}
}

pub fn grid(n: usize) -> cuda_core::LaunchConfig1D {
    cuda_core::LaunchConfig1D::new(n.div_ceil(256).clamp(1,65535) as u32, 256, 0)
}

struct Index { snapshot: u64, keys: u32, heads: Buffer<u32>, rows: Buffer<u32> }
pub struct Directories(shared_directory_kernels::LoadedModule);
impl Directories {
    pub fn new(pool: &Pool) -> Result<Self> { Ok(Self(unsafe { shared_directory_kernels::load(pool.context())? })) }
}
/// One checked source/layout/key shape, scoped to the component and its stream.
#[derive(Default)]
pub struct IndexSlot(Option<Index>);
impl IndexSlot {
    pub fn get<K,W: Word + Copy + 'static>(&mut self, directories: &Directories, pool: &Arc<Pool>, src: &RadixLayout<K,(),W,u64,()>, keys: u32) -> Result<Directory> {
        let width = u32::try_from(src.packing().len())?;
        if keys == 0 || keys > width.min(3) { return Err("invalid shared directory key".into()); }
        if self.0.as_ref().is_none_or(|i| i.snapshot != src.snapshot_id() || i.keys != keys) {
            let n = u32::try_from(src.len()).map_err(|_| "shared directory position overflow")? as usize;
            let heads = Buffer::zeroed(pool.clone(), n.max(1).next_power_of_two())?;
            let rows = unsafe { Buffer::uninitialized(pool.clone(), n.checked_mul(width as usize+1).ok_or("shared directory size overflow")?)? };
            if n != 0 {
                let dir = Directory {heads: heads.ptr(), rows: rows.ptr(), mask: (heads.len()-1) as u32};
                let module = &directories.0;
                module.round_shared_build(pool.stream(), &module.prepare_round_shared_build::<W>(grid(n))?, src.view_on(pool)?, width, keys, dir)?;
            }
            self.0 = Some(Index {snapshot:src.snapshot_id(), keys, heads, rows});
        }
        let i = self.0.as_ref().unwrap();
        Ok(Directory {heads:i.heads.ptr(), rows:i.rows.ptr(), mask:(i.heads.len()-1) as u32})
    }
}

fn count(counter: &Buffer<u64>) -> Result<usize> {
    let n = counter.to_host()?[0];
    crate::record_readback();
    Ok(u32::try_from(n).map_err(|_| "shared candidate position overflow")? as usize)
}

pub struct Witnesses { rows: Buffer<u32>, pub len: usize }
impl Witnesses {
    /// Device pointer; only `len` witness tuples are initialized.
    pub fn ptr(&self) -> *const u32 { self.rows.ptr() }
}
/// The same byte-budgeted last/high-water policy as a candidate arena. A
/// large round followed by an empty one must not forget *all* useful history.
#[derive(Default)]
pub struct WitnessHint(crate::arena::HoleCounts);
/// # Safety
/// The launcher must use `pool`'s stream, count all witnesses, and initialize
/// every in-capacity witness (exactly `width` words) without writing past it.
pub unsafe fn materialize(pool: &Arc<Pool>, width: usize, hint: &mut WitnessHint, empty: bool, mut launch: impl FnMut(*mut u32, u64, *mut u64) -> Result<()>) -> Result<Witnesses> {
    let row_bytes = width.max(1).checked_mul(4).ok_or("shared witness width overflow")?;
    let mut capacity = if empty { 0 } else { hint.0.hint(row_bytes) };
    loop {
        let rows = unsafe { Buffer::uninitialized(pool.clone(), capacity.checked_mul(width).ok_or("shared witness size overflow")?)? };
        let n = if empty { 0 } else {
            let cursor = Buffer::zeroed(pool.clone(), 1)?;
            launch(rows.ptr(), capacity as u64, cursor.ptr())?;
            count(&cursor)?
        };
        if n <= capacity {
            // A miss only changes capacity, never visibility or prefix contents.
            hint.0.record(n);
            return Ok(Witnesses { rows, len: n });
        }
        capacity = n;
    }
}

/// A projection without joins has one output per witness; only a variable
/// cardinality suffix needs a cursor/readback. Short leases are never published.
/// # Safety
/// The launcher must write `packing` on the arena stream, respect `cap`,
/// and initialize the entire validated prefix: exactly `n` rows when direct,
/// otherwise as many as its complete cursor reports (up to capacity).
pub unsafe fn project(arena: &mut CandidateArena, packing: &Packing, n: usize, direct: bool, mut launch: impl FnMut(*mut u64, u64, *mut u64) -> Result<()>) -> Result<CandidateBatch> {
    let mut cap = if direct || n == 0 { n } else { arena.capacity_hint(packing) };
    loop {
        let lease = arena.reserve(packing.clone(), cap)?;
        let rows = if n == 0 { 0 } else if direct {
            launch(lease.buffer().ptr(), lease.capacity() as u64, std::ptr::null_mut())?;
            n
        } else {
            let cursor = Buffer::zeroed(arena.pool().clone(), 1)?;
            launch(lease.buffer().ptr(), lease.capacity() as u64, cursor.ptr())?;
            count(&cursor)?
        };
        if rows <= lease.capacity() {
            arena.record_count(rows);
            // Inputs and frees are ordered on the same pool stream, including
            // the known-cardinality case that needs no synchronization.
            return unsafe { lease.finish(rows, vec![]) };
        }
        cap = rows;
    }
}
