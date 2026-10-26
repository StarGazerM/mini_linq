//! Caller-owned reusable candidate storage. Publication never downloads rows.
use crate::{Buffer, Packing, Pool, Result, Word};
use std::{any::Any, sync::Arc};

/// Reusable storage for generated Sequence terminals on one pool stream.
///
/// A lease exclusively reserves a slot. Published batches keep that slot and
/// their input owners alive; subsequent reservations cannot overwrite it until
/// every batch clone and owner token has been dropped. Dropped leases become
/// reusable, including failed capacity-probe attempts, without synchronization.
/// The pool orders both reuse and destruction after preceding device work.
pub struct CandidateArena {
    pool: Arc<Pool>,
    slots: Vec<Arc<Buffer<u64>>>,
}
impl CandidateArena {
    pub fn new(pool: Arc<Pool>) -> Self { Self { pool, slots: Vec::new() } }
    pub fn pool(&self) -> &Arc<Pool> { &self.pool }

    /// Device bytes retained by all slots, including spare capacity and slots
    /// still owned by published batches. Does not include their input owners.
    pub fn allocated_bytes(&self) -> usize {
        self.slots.iter().map(|slot| slot.allocated_bytes()).sum()
    }
    /// Total lane capacity, including in-use slots, expressed as rows of
    /// `packing`. Slots may have different packings; each slot rounds down.
    pub fn reserved_capacity(&self, packing: &Packing) -> usize {
        let lanes = (packing.word_bits() / 64) as usize;
        self.slots.iter().map(|slot| slot.len() / lanes).sum()
    }
    /// Number of backing buffers retained by this arena, including idle slots.
    pub fn retained_buffers(&self) -> usize { self.slots.len() }
    /// Backing bytes currently leased or retained by a published batch.
    /// These are capacities, not initialized live-prefix sizes.
    pub fn in_use_bytes(&self) -> usize {
        self.slots.iter().filter(|slot| Arc::strong_count(slot) > 1)
            .map(|slot| slot.allocated_bytes()).sum()
    }
    /// Release idle slots in stream order without disturbing leases or batches.
    pub fn release_unused(&mut self) {
        self.slots.retain(|slot| Arc::strong_count(slot) > 1);
    }
    /// Forget every slot. Outstanding leases and batches remain valid and keep
    /// their own storage alive; subsequent reservations allocate fresh slots.
    pub fn clear(&mut self) { self.slots.clear(); }

    /// Reserve the minimum complete u64 lanes for each row. Native u128 words
    /// use low-lane-first storage; words wider than 128 bits use MS-first arrays.
    /// The emitter initializes the live prefix before finish.
    /// `GPU_DATALOG_TRACE_MEMORY=1` logs requested and actual backing bytes on
    /// allocation/growth, including failed retries, without device readbacks.
    pub fn reserve(&mut self, packing: Packing, capacity: usize) -> Result<CandidateLease> {
        let lanes = lane_count(&packing, capacity)?;
        let fitting = self.slots.iter().enumerate()
            .filter(|(_, slot)| Arc::strong_count(slot) == 1 && slot.len() >= lanes)
            .min_by_key(|(_, slot)| slot.len()).map(|(index, _)| index);
        let index = match fitting {
            Some(index) => index,
            None => {
                let trace = std::env::var_os("GPU_DATALOG_TRACE_MEMORY").is_some_and(|value| value == "1");
                if trace {
                    eprintln!("[gpu-memory] arena-grow request_rows={capacity} word_bits={} request_bytes={} retained_bytes={} in_use_bytes={} buffers={}",
                        packing.word_bits(), lanes * std::mem::size_of::<u64>(),
                        self.allocated_bytes(), self.in_use_bytes(), self.retained_buffers());
                }
                // Every idle slot is too small. Release them before allocating
                // the replacement, rather than doubling the retry growth peak.
                // Pool deallocation/reallocation is ordered on the same stream.
                self.release_unused();
                // SAFETY: this unpublished storage is handed only to a writer;
                // finish requires stream-ordered initialization of the prefix.
                let buffer = Arc::new(unsafe { Buffer::uninitialized(self.pool.clone(), lanes)? });
                if trace {
                    eprintln!("[gpu-memory] arena-grown request_rows={capacity} slot_bytes={} retained_bytes={} buffers={}",
                        buffer.allocated_bytes(), self.allocated_bytes() + buffer.allocated_bytes(),
                        self.retained_buffers() + 1);
                }
                self.slots.push(buffer);
                self.slots.len() - 1
            }
        };
        Ok(CandidateLease { buffer: self.slots[index].clone(), packing, capacity })
    }
}

fn lane_count(packing: &Packing, rows: usize) -> Result<usize> {
    if rows > u32::MAX as usize {
        return Err("candidate row capacity exceeds u32".into());
    }
    rows.checked_mul((packing.word_bits() / 64) as usize)
        .ok_or_else(|| "candidate lane capacity overflow".into())
}

/// Exclusive unpublished storage for one terminal attempt. Dropping a lease
/// without finishing releases the slot; no candidate data is read back.
pub struct CandidateLease {
    buffer: Arc<Buffer<u64>>,
    packing: Packing,
    capacity: usize,
}
impl CandidateLease {
    /// Unpublished output storage; read only the prefix initialized by a kernel.
    pub fn buffer(&self) -> &Buffer<u64> { &self.buffer }
    pub fn capacity(&self) -> usize { self.capacity }
    pub fn packing(&self) -> &Packing { &self.packing }
    /// Full backing allocation, including capacity reused from a larger slot.
    pub fn allocated_bytes(&self) -> usize { self.buffer.allocated_bytes() }
    /// Actual slot capacity, which may exceed this attempt's requested capacity.
    pub fn reserved_capacity(&self) -> usize {
        self.buffer.len() / (self.packing.word_bits() / 64) as usize
    }

    /// Publish an immutable live prefix and retain the terminal's input owners.
    /// Only the scalar length supplied by the terminal is inspected on the host.
    ///
    /// # Safety
    /// Exactly `live_len` packed words must have been initialized on the pool
    /// stream, with this lease's packing and native lane order: low-first through
    /// 128 bits, most-significant-first above 128 bits. No alias may write that
    /// prefix after publication. All buffers and owners use the same pool stream.
    pub unsafe fn finish(self, live_len: usize, owners: Vec<Arc<dyn Any>>) -> Result<CandidateBatch> {
        if live_len > self.capacity {
            return Err("candidate live length exceeds reserved row capacity".into());
        }
        let lanes = lane_count(&self.packing, live_len)?;
        // SAFETY: the caller initialized this prefix and publishes no writers;
        // the shared slice retains the arena slot until every batch is dropped.
        let buffer = unsafe { Buffer::shared_slice(self.buffer, 0, lanes)? };
        Ok(CandidateBatch { storage: Arc::new(BatchStorage {
            buffer, packing: self.packing, rows: live_len, _owners: owners,
        }) })
    }
}

struct BatchStorage {
    buffer: Buffer<u64>,
    packing: Packing,
    rows: usize,
    _owners: Vec<Arc<dyn Any>>,
}

/// An immutable, not necessarily sorted or unique device candidate batch.
/// The buffer exposes exactly the initialized lane prefix, not spare capacity.
/// Cloning or taking an owner token retains both storage and input dependencies.
#[derive(Clone)]
pub struct CandidateBatch {
    storage: Arc<BatchStorage>,
}
impl CandidateBatch {
    pub fn len(&self) -> usize { self.storage.rows }
    pub fn is_empty(&self) -> bool { self.storage.rows == 0 }
    /// Initialized packed bytes; excludes the spare capacity retained by the slot.
    pub fn live_bytes(&self) -> usize { self.storage.buffer.bytes() }
    /// Full slot allocation, shared with the arena and any batch clones.
    /// Input dependency allocations are not included.
    pub fn allocated_bytes(&self) -> usize { self.storage.buffer.allocated_bytes() }
    /// Backing capacity expressed in rows of this batch's packing.
    pub fn reserved_capacity(&self) -> usize {
        self.allocated_bytes() / (self.storage.packing.word_bits() / 8) as usize
    }
    pub fn packing(&self) -> &Packing { &self.storage.packing }
    pub fn pool(&self) -> &Arc<Pool> { self.storage.buffer.pool() }
    pub fn buffer(&self) -> &Buffer<u64> { &self.storage.buffer }
    pub fn owner(&self) -> Arc<dyn Any> { self.storage.clone() }
}

/// Concatenate candidate bags on the pool stream without downloading rows.
///
/// `W` must be the packing's narrowest native representation: `u64` through
/// 64 bits, `u128` through 128 bits, then `[u64; L]` with MS-first lanes.
/// Batch order and duplicates are preserved; admission remains separate.
/// The returned allocation is initialized by stream-ordered device copies.
pub fn concatenate_batches<W: Word>(
    pool: Arc<Pool>,
    packing: &Packing,
    batches: &[CandidateBatch],
) -> Result<Buffer<W>> {
    if W::BITS != packing.word_bits() {
        return Err("candidate concatenation word width differs from packing".into());
    }
    if W::BITS == 128 && W::MS_FIRST {
        return Err("candidate concatenation requires native u128 lane order".into());
    }
    let mut rows = 0usize;
    for batch in batches {
        if !Arc::ptr_eq(&pool, batch.pool()) {
            return Err("candidate concatenation pool differs from batch".into());
        }
        if batch.packing() != packing {
            return Err("candidate concatenation packing differs from batch".into());
        }
        rows = rows.checked_add(batch.len()).filter(|&rows| rows <= u32::MAX as usize)
            .ok_or("candidate concatenation exceeds u32 positions")?;
    }
    let bytes_per_word = std::mem::size_of::<W>();
    rows.checked_mul(bytes_per_word).ok_or("candidate concatenation byte count overflow")?;
    // SAFETY: the disjoint copies below initialize exactly the allocated prefix
    // before any subsequent operation on the same stream can consume it.
    let output = unsafe { Buffer::<W>::uninitialized(pool.clone(), rows)? };
    let mut offset = 0usize;
    for batch in batches {
        let bytes = batch.len() * bytes_per_word;
        if bytes != 0 {
            // SAFETY: exact packing/word widths and lane order make the lanes
            // byte-identical to W on CUDA. Both buffers share this pool stream;
            // slices are initialized and disjoint, and pool frees/reuse follow
            // the copy in stream order even when callers drop the input batch.
            unsafe {
                cuda_core::simt::memory::memcpy_dtod_async(
                    output.cu_deviceptr() + offset as u64,
                    batch.buffer().cu_deviceptr(),
                    bytes,
                    pool.stream().cu_stream(),
                )?;
            }
        }
        offset += bytes;
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_capacity_uses_native_lanes() {
        assert_eq!(lane_count(&Packing::new(&[]).unwrap(), 7).unwrap(), 7);
        assert_eq!(lane_count(&Packing::new(&[64]).unwrap(), 7).unwrap(), 7);
        assert_eq!(lane_count(&Packing::new(&[64, 1]).unwrap(), 7).unwrap(), 14);
        assert_eq!(lane_count(&Packing::new(&[64, 64]).unwrap(), 0).unwrap(), 0);
        assert_eq!(lane_count(&Packing::new(&[64, 64, 1]).unwrap(), 7).unwrap(), 21);
        assert_eq!(lane_count(&Packing::new(&[64; 4]).unwrap(), 7).unwrap(), 28);
        if usize::BITS > 32 {
            assert!(lane_count(&Packing::new(&[8]).unwrap(), u32::MAX as usize + 1).is_err());
        }
    }
}
