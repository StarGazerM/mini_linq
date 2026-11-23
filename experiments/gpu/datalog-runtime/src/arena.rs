//! Caller-owned reusable candidate storage. Publication never downloads rows.
use crate::{Buffer, Packing, Pool, Result, Word};
use std::{any::Any, sync::Arc};

/// Largest slot an arena keeps for reuse. A larger slot belongs to its batch
/// alone and is freed once admission has consumed the rows: holding a
/// multi-gigabyte candidate bag through publication and into the next round
/// costs more than allocating it again. The hole's count hint is still kept.
const RETAINED_SLOT_BYTES: usize = 1 << 30;

// Cap speculative storage, not the live output: a large historical spike must
// not keep imposing that allocation on every hole in subsequent rounds.
const SPECULATIVE_SLOT_BYTES: usize = 8 << 20;

// Growth slack above the last count: half of it, at most this many bytes. A
// growing hole above the speculative cap otherwise fails whenever it grows
// (jython: 1.65 s of failed attempts); the slack follows the last result, so a
// past spike does not keep imposing it.
const GROWTH_SLACK_BYTES: usize = 256 << 20;

#[derive(Clone, Copy, Default)]
struct HoleCounts {
    last: usize,
    high: usize,
}

/// Reusable storage for generated Sequence terminals on one pool stream.
///
/// A lease exclusively reserves a slot. Published batches keep that slot and
/// their input owners alive; subsequent reservations cannot overwrite it until
/// every batch clone and owner token has been dropped. Dropped leases become
/// reusable, including failed capacity-probe attempts, without synchronization.
/// The pool orders both reuse and destruction after preceding device work.
/// Capacity hints are kept per Round hole (GPU-ReservePrevious): the printer
/// selects the recorded hole with [`Self::hole`] before invoking it, so holes
/// sharing a head relation's arena never overwrite each other's hint.
pub struct CandidateArena {
    pool: Arc<Pool>,
    slots: Vec<Arc<Buffer<u64>>>,
    previous_counts: Vec<HoleCounts>,
    hole: usize,
    resident: Option<Arc<Buffer<u32>>>,
}
impl CandidateArena {
    pub fn new(pool: Arc<Pool>) -> Self {
        Self {
            pool,
            slots: Vec::new(),
            previous_counts: Vec::new(),
            hole: 0,
            resident: None,
        }
    }
    pub fn pool(&self) -> &Arc<Pool> {
        &self.pool
    }
    /// Select the Round hole whose hint the next terminal reads and records.
    pub fn hole(&mut self, hole: usize) -> &mut Self {
        if self.previous_counts.len() <= hole {
            self.previous_counts.resize(hole + 1, HoleCounts::default());
        }
        self.hole = hole;
        self
    }
    /// The selected hole's last validated count (zero before its first run).
    pub fn previous_count(&self) -> usize {
        self.previous_counts.get(self.hole).map_or(0, |counts| counts.last)
    }
    /// Reserve for growth beyond the high-water mark only within a bounded
    /// byte budget, and for growth beyond the last count by at most half of it
    /// within a bounded slack. Always honor the previous result.
    pub fn capacity_hint(&self, packing: &Packing) -> usize {
        let Some(counts) = self.previous_counts.get(self.hole) else { return 0 };
        let word_bytes = packing.word_bits() as usize / 8;
        let budget_rows = SPECULATIVE_SLOT_BYTES / word_bytes;
        let slack = (counts.last / 2).min(GROWTH_SLACK_BYTES / word_bytes);
        (counts.last + slack).max(counts.high.saturating_mul(2).min(budget_rows))
    }
    /// Remember the selected hole's fully validated count, including an empty result.
    pub fn record_count(&mut self, rows: usize) {
        let hole = self.hole;
        let counts = &mut self.hole(hole).previous_counts[hole];
        counts.last = rows;
        counts.high = counts.high.max(rows);
    }
    /// GPU-ResidentMeta: at least `words` resident metadata words, zero on
    /// entry. Each dispatch that uses them must leave every word zero again in
    /// stream order (its last block re-zeroes them), so attempts neither
    /// allocate nor clear metadata. Only growth allocates and clears, once.
    pub fn resident_words(&mut self, words: usize) -> Result<Arc<Buffer<u32>>> {
        if let Some(resident) = self
            .resident
            .as_ref()
            .filter(|resident| resident.len() >= words)
        {
            return Ok(resident.clone());
        }
        let resident = Arc::new(Buffer::zeroed(
            self.pool.clone(),
            words.next_power_of_two(),
        )?);
        self.resident = Some(resident.clone());
        Ok(resident)
    }

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
    pub fn retained_buffers(&self) -> usize {
        self.slots.len()
    }
    /// Backing bytes currently leased or retained by a published batch.
    /// These are capacities, not initialized live-prefix sizes.
    pub fn in_use_bytes(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| Arc::strong_count(slot) > 1)
            .map(|slot| slot.allocated_bytes())
            .sum()
    }
    /// Release idle slots in stream order without disturbing leases or batches.
    pub fn release_unused(&mut self) {
        self.slots.retain(|slot| Arc::strong_count(slot) > 1);
    }
    /// Forget every slot. Outstanding leases and batches remain valid and keep
    /// their own storage alive; subsequent reservations allocate fresh slots.
    pub fn clear(&mut self) {
        self.slots.clear();
        self.previous_counts.clear();
    }

    /// Reserve the minimum complete u64 lanes for each row. Native u128 words
    /// use low-lane-first storage; words wider than 128 bits use MS-first arrays.
    /// The lease's capacity is the chosen slot's full row capacity (at least the
    /// request, at most `u32::MAX` rows), so an attempt may use spare retained
    /// storage instead of replaying (GPU-ReservePrevious).
    /// The emitter initializes the live prefix before finish.
    /// `GPU_DATALOG_TRACE_MEMORY=1` logs requested and actual backing bytes on
    /// allocation/growth, including failed retries, without device readbacks.
    pub fn reserve(&mut self, packing: Packing, capacity: usize) -> Result<CandidateLease> {
        let lanes = lane_count(&packing, capacity)?;
        let fitting = self
            .slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| Arc::strong_count(slot) == 1 && slot.len() >= lanes)
            .min_by_key(|(_, slot)| slot.len())
            .map(|(index, _)| index);
        let index = match fitting {
            Some(index) => index,
            None => {
                let trace =
                    std::env::var_os("GPU_DATALOG_TRACE_MEMORY").is_some_and(|value| value == "1");
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
                if buffer.allocated_bytes() > RETAINED_SLOT_BYTES {
                    let rows = buffer.len() / (packing.word_bits() / 64) as usize;
                    return Ok(CandidateLease {
                        buffer,
                        packing,
                        capacity: rows.min(u32::MAX as usize).max(capacity),
                    });
                }
                self.slots.push(buffer);
                self.slots.len() - 1
            }
        };
        let rows = self.slots[index].len() / (packing.word_bits() / 64) as usize;
        Ok(CandidateLease {
            buffer: self.slots[index].clone(),
            packing,
            capacity: rows.min(u32::MAX as usize).max(capacity),
        })
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
    pub fn buffer(&self) -> &Buffer<u64> {
        &self.buffer
    }
    pub fn capacity(&self) -> usize {
        self.capacity
    }
    pub fn packing(&self) -> &Packing {
        &self.packing
    }
    /// Full backing allocation, including capacity reused from a larger slot.
    pub fn allocated_bytes(&self) -> usize {
        self.buffer.allocated_bytes()
    }
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
    pub unsafe fn finish(
        self,
        live_len: usize,
        owners: Vec<Arc<dyn Any + Send + Sync>>,
    ) -> Result<CandidateBatch> {
        if live_len > self.capacity {
            return Err("candidate live length exceeds reserved row capacity".into());
        }
        let lanes = lane_count(&self.packing, live_len)?;
        let (live, held) = (lanes * size_of::<u64>(), self.buffer.allocated_bytes());
        // A slot above the retained size is this batch's alone, sized by the
        // hole's previous count. When this result is at most half of it and
        // the rest is more than a retained slot, the rows move to a slot of
        // their own size: the spare capacity would otherwise stay allocated
        // through admission, beside every other batch of the round.
        let buffer = if held > RETAINED_SLOT_BYTES
            && live <= held / 2
            && held - live > RETAINED_SLOT_BYTES
        {
            let pool = self.buffer.pool().clone();
            // SAFETY: the copy below initializes all of it on the stream that
            // wrote the rows, and the oversized slot is freed after the copy
            // on that same stream.
            let exact = unsafe { Buffer::<u64>::uninitialized(pool.clone(), lanes)? };
            if live != 0 {
                unsafe {
                    cuda_core::simt::memory::memcpy_dtod_async(
                        exact.cu_deviceptr(),
                        self.buffer.cu_deviceptr(),
                        live,
                        pool.stream().cu_stream(),
                    )?;
                }
            }
            exact
        } else {
            // SAFETY: the caller initialized this prefix and publishes no writers;
            // the shared slice retains the arena slot until every batch is dropped.
            unsafe { Buffer::shared_slice(self.buffer, 0, lanes)? }
        };
        Ok(CandidateBatch {
            storage: Arc::new(BatchStorage {
                buffer,
                packing: self.packing,
                rows: live_len,
                _owners: owners,
            }),
        })
    }
}

struct BatchStorage {
    buffer: Buffer<u64>,
    packing: Packing,
    rows: usize,
    _owners: Vec<Arc<dyn Any + Send + Sync>>,
}

/// An immutable, not necessarily sorted or unique device candidate batch.
/// The buffer exposes exactly the initialized lane prefix, not spare capacity.
/// Cloning or taking an owner token retains both storage and input dependencies.
#[derive(Clone)]
pub struct CandidateBatch {
    storage: Arc<BatchStorage>,
}
impl CandidateBatch {
    pub fn len(&self) -> usize {
        self.storage.rows
    }
    pub fn is_empty(&self) -> bool {
        self.storage.rows == 0
    }
    /// Initialized packed bytes; excludes the spare capacity retained by the slot.
    pub fn live_bytes(&self) -> usize {
        self.storage.buffer.bytes()
    }
    /// Full slot allocation, shared with the arena and any batch clones.
    /// Input dependency allocations are not included.
    pub fn allocated_bytes(&self) -> usize {
        self.storage.buffer.allocated_bytes()
    }
    /// Backing capacity expressed in rows of this batch's packing.
    pub fn reserved_capacity(&self) -> usize {
        self.allocated_bytes() / (self.storage.packing.word_bits() / 8) as usize
    }
    pub fn packing(&self) -> &Packing {
        &self.storage.packing
    }
    pub fn pool(&self) -> &Arc<Pool> {
        self.storage.buffer.pool()
    }
    pub fn buffer(&self) -> &Buffer<u64> {
        &self.storage.buffer
    }
    pub fn owner(&self) -> Arc<dyn Any + Send + Sync> {
        self.storage.clone()
    }
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
    let ranges = batches.iter().map(|batch| (batch, 0, batch.len())).collect::<Vec<_>>();
    concatenate(pool, packing, &ranges, false)
}

/// Read-only admission input over initialized candidate ranges. Unlike the
/// public concatenation, a single range may share its immutable storage.
pub(crate) fn concatenate_ranges<W: Word>(
    pool: Arc<Pool>,
    packing: &Packing,
    ranges: &[(&CandidateBatch, usize, usize)],
) -> Result<Buffer<W>> {
    concatenate(pool, packing, ranges, true)
}

fn concatenate<W: Word>(
    pool: Arc<Pool>,
    packing: &Packing,
    ranges: &[(&CandidateBatch, usize, usize)],
    allow_shared: bool,
) -> Result<Buffer<W>> {
    if W::BITS != packing.word_bits() {
        return Err("candidate concatenation word width differs from packing".into());
    }
    if W::BITS == 128 && W::MS_FIRST {
        return Err("candidate concatenation requires native u128 lane order".into());
    }
    let mut rows = 0usize;
    for &(batch, first, count) in ranges {
        if !pool.shares_memory(batch.pool()) {
            return Err("candidate concatenation pool differs from batch".into());
        }
        if batch.packing() != packing {
            return Err("candidate concatenation packing differs from batch".into());
        }
        if first.checked_add(count).is_none_or(|end| end > batch.len()) {
            return Err("candidate concatenation range exceeds its batch".into());
        }
        rows = rows
            .checked_add(count)
            .filter(|&rows| rows <= u32::MAX as usize)
            .ok_or("candidate concatenation exceeds u32 positions")?;
    }
    let bytes_per_word = std::mem::size_of::<W>();
    rows.checked_mul(bytes_per_word)
        .ok_or("candidate concatenation byte count overflow")?;
    crate::stats::trace_step(&pool, "admit/concatenate", rows, bytes_per_word);
    if let [(batch, first, count)] = ranges {
        if allow_shared && Arc::ptr_eq(&pool, batch.pool()) {
            // Only share identical native element types (the batch stores
            // u64 lanes), not a type-punned or differently aligned buffer.
            if let Some(buffer) = (batch.buffer() as &dyn Any).downcast_ref::<Buffer<W>>() {
                // SAFETY: finish published an immutable, initialized prefix;
                // the retained slot cannot be leased again while this view lives.
                if let Some(view) = unsafe { buffer.shared_subrange(*first, *count)? } {
                    return Ok(view);
                }
            }
        }
    }
    // SAFETY: the disjoint copies below initialize exactly the allocated prefix
    // before any subsequent operation on the same stream can consume it.
    let output = unsafe { Buffer::<W>::uninitialized(pool.clone(), rows)? };
    let mut offset = 0usize;
    for &(batch, first, count) in ranges {
        let bytes = count * bytes_per_word;
        if bytes != 0 {
            // SAFETY: exact packing/word widths and lane order make the lanes
            // byte-identical to W on CUDA. Both buffers are ordered on this
            // pool's stream; slices are initialized and disjoint.
            unsafe {
                cuda_core::simt::memory::memcpy_dtod_async(
                    output.cu_deviceptr() + offset as u64,
                    batch.buffer().cu_deviceptr() + (first * bytes_per_word) as u64,
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
    fn admission_subrange_retains_slot_but_public_concatenation_is_owned() -> Result<()> {
        let pool = Pool::new(0, 0, 0)?;
        let packing = Packing::new(&[64])?;
        let mut arena = CandidateArena::new(pool.clone());
        let lease = arena.reserve(packing.clone(), 8)?;
        let base = lease.buffer().cu_deviceptr();
        // SAFETY: the sole writer initializes four lanes on the pool stream.
        let batch = unsafe {
            lease.buffer().write_async(&[2, 3, 5, 7])?;
            lease.finish(4, vec![])?
        };
        let view = concatenate_ranges::<u64>(pool.clone(), &packing, &[(&batch, 1, 2)])?;
        assert_eq!(view.cu_deviceptr(), base + 8);
        assert!(unsafe { view.shared_subrange(2, 1) }.is_err());
        let owned = concatenate_batches::<u64>(pool, &packing, std::slice::from_ref(&batch))?;
        assert_ne!(owned.cu_deviceptr(), base);
        drop(batch);
        let next = arena.reserve(packing, 8)?;
        assert_ne!(next.buffer().cu_deviceptr(), base);
        drop(arena);
        assert_eq!(view.to_host()?, [3, 5]);
        assert_eq!(owned.to_host()?, [2, 3, 5, 7]);
        Ok(())
    }

    #[test]
    fn row_capacity_uses_native_lanes() {
        assert_eq!(lane_count(&Packing::new(&[]).unwrap(), 7).unwrap(), 7);
        assert_eq!(lane_count(&Packing::new(&[64]).unwrap(), 7).unwrap(), 7);
        assert_eq!(lane_count(&Packing::new(&[64, 1]).unwrap(), 7).unwrap(), 14);
        assert_eq!(lane_count(&Packing::new(&[64, 64]).unwrap(), 0).unwrap(), 0);
        assert_eq!(
            lane_count(&Packing::new(&[64, 64, 1]).unwrap(), 7).unwrap(),
            21
        );
        assert_eq!(lane_count(&Packing::new(&[64; 4]).unwrap(), 7).unwrap(), 28);
        if usize::BITS > 32 {
            assert!(lane_count(&Packing::new(&[8]).unwrap(), u32::MAX as usize + 1).is_err());
        }
    }
}
