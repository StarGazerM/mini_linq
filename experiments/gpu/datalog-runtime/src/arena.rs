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

    /// Reserve row capacity using one u64 lane through 64 bits and two lanes
    /// through 128 bits. The emitter initializes its live prefix before finish.
    /// Two-lane words store their low lane first, independently of alignment.
    pub fn reserve(&mut self, packing: Packing, capacity: usize) -> Result<CandidateLease> {
        let lanes = lane_count(&packing, capacity)?;
        let available = self.slots.iter().enumerate()
            .filter(|(_, slot)| Arc::strong_count(slot) == 1);
        let fitting = available.clone().filter(|(_, slot)| slot.len() >= lanes)
            .min_by_key(|(_, slot)| slot.len()).map(|(index, _)| index);
        let index = match fitting {
            Some(index) => index,
            None => {
                let replace = available.min_by_key(|(_, slot)| slot.len()).map(|(index, _)| index);
                // SAFETY: this unpublished storage is handed only to a writer;
                // finish requires stream-ordered initialization of the prefix.
                let buffer = Arc::new(unsafe { Buffer::uninitialized(self.pool.clone(), lanes)? });
                match replace {
                    Some(index) => { self.slots[index] = buffer; index }
                    None => { self.slots.push(buffer); self.slots.len() - 1 }
                }
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

    /// Publish an immutable live prefix and retain the terminal's input owners.
    /// Only the scalar length supplied by the terminal is inspected on the host.
    ///
    /// # Safety
    /// Exactly `live_len` packed words must have been initialized on the pool
    /// stream, with this lease's packing and low-u64-first lane order. No alias
    /// may write that prefix after publication. All work using these buffers
    /// and the supplied input owners must use the same pool stream.
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
    pub fn packing(&self) -> &Packing { &self.storage.packing }
    pub fn pool(&self) -> &Arc<Pool> { self.storage.buffer.pool() }
    pub fn buffer(&self) -> &Buffer<u64> { &self.storage.buffer }
    pub fn owner(&self) -> Arc<dyn Any> { self.storage.clone() }
}

/// Concatenate candidate bags on the pool stream without downloading rows.
///
/// `W` must be the packing's native word width: `u64` through 64 bits, otherwise
/// `u128`. Batch order and duplicates are preserved; admission remains separate.
/// The returned allocation is initialized by stream-ordered device copies.
pub fn concatenate_batches<W: Word>(
    pool: Arc<Pool>,
    packing: &Packing,
    batches: &[CandidateBatch],
) -> Result<Buffer<W>> {
    if W::BITS != packing.word_bits() {
        return Err("candidate concatenation word width differs from packing".into());
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
            // SAFETY: exact packing/word widths make the low-u64-first lanes
            // byte-identical to W on CUDA. Both buffers share this pool stream;
            // slices are initialized and disjoint, and pool frees/reuse follow
            // the copy in stream order even when callers drop the input batch.
            unsafe {
                cuda_core::memory::memcpy_dtod_async(
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
        if usize::BITS > 32 {
            assert!(lane_count(&Packing::new(&[8]).unwrap(), u32::MAX as usize + 1).is_err());
        }
    }
}
