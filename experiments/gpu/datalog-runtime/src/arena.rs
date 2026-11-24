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
pub(crate) struct HoleCounts {
    last: usize,
    high: usize,
}
impl HoleCounts {
    pub(crate) fn hint(&self, row_bytes: usize) -> usize {
        let budget_rows = SPECULATIVE_SLOT_BYTES / row_bytes;
        let slack = (self.last / 2).min(GROWTH_SLACK_BYTES / row_bytes);
        (self.last + slack).max(self.high.saturating_mul(2).min(budget_rows))
    }
    pub(crate) fn record(&mut self, rows: usize) {
        self.last = rows;
        self.high = self.high.max(rows);
    }
}

/// Grow a validated head count by 50%, but spend at most 64 MiB on speculative
/// rows per head. Don't let slack alone turn a reusable slot into a transient one.
fn head_capacity_hint(rows: usize, packing: &Packing) -> usize {
    const MAX_SLACK_BYTES: usize = 64 << 20;
    let row_bytes = (packing.word_bits() / 8) as usize;
    let retained = RETAINED_SLOT_BYTES / row_bytes;
    let extra = rows.div_ceil(2).min(MAX_SLACK_BYTES / row_bytes);
    let limit = if rows <= retained { retained } else { u32::MAX as usize };
    rows.saturating_add(extra).min(limit)
}

/// One publication point's interleaved `[cursor, failure]` pairs. Each head
/// appends on this pool's stream; a single download checks every head before
/// any prefix is published. Dispatch-local scratch is not shared here.
pub struct HeadCursors {
    words: Arc<Buffer<u32>>,
}
impl HeadCursors {
    pub fn new(pool: Arc<Pool>, heads: usize) -> Result<Self> {
        let words = heads.checked_mul(2).ok_or("head cursor group too large")?;
        Ok(Self { words: Arc::new(Buffer::zeroed(pool, words)?) })
    }

    /// Download all pairs in slot order, recording one metadata readback.
    pub fn read(&self) -> Result<Vec<u32>> {
        let words = self.words.to_host()?;
        if !words.is_empty() {
            crate::record_readback();
        }
        Ok(words)
    }
}

struct HeadAttempt {
    words: Arc<Buffer<u32>>,
    slot: usize,
    phase: usize,
    lease: CandidateLease,
    owners: Vec<Arc<dyn Any + Send + Sync>>,
}
impl HeadAttempt {
    fn cursor(&self) -> *mut u32 {
        self.words.ptr().wrapping_add(2 * self.slot)
    }

    fn reset(&self) -> Result<()> {
        // SAFETY: this attempt owns the pair, after all its appends on this stream.
        unsafe {
            cuda_core::simt::memory::memset_d8_async(
                self.cursor() as u64, 0, size_of::<[u32; 2]>(), self.words.pool().stream().cu_stream(),
            )?;
        }
        Ok(())
    }
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
    head: Option<HeadAttempt>,
    head_hints: [usize; 2],
    mixed_head: [bool; 2],
}
impl CandidateArena {
    pub fn new(pool: Arc<Pool>) -> Self {
        Self {
            pool,
            slots: Vec::new(),
            previous_counts: Vec::new(),
            hole: 0,
            resident: None,
            head: None,
            head_hints: [0; 2],
            mixed_head: [false; 2],
        }
    }
    pub fn pool(&self) -> &Arc<Pool> {
        &self.pool
    }
    pub(super) fn has_head_target(&self) -> bool {
        self.head.is_some()
    }
    /// Start the sole active head attempt on this arena. Use a distinct group
    /// slot for each head; finish or retry its attempt before reusing the slot.
    /// Phase 0 (seed) and 1 (step) have independent hints with bounded slack.
    pub fn begin_head(
        &mut self, group: &HeadCursors, slot: usize, packing: Packing, phase: usize,
    ) -> Result<()> {
        if self.head.is_some() {
            return Err("candidate head attempt already active".into());
        }
        if !Arc::ptr_eq(group.words.pool(), &self.pool) || slot >= group.words.len() / 2 {
            return Err("candidate head cursor pool or slot differs".into());
        }
        let &hint = self.head_hints.get(phase).ok_or("invalid candidate head phase")?;
        let lease = self.reserve_ordered(packing, hint, self.mixed_head[phase])?;
        self.head = Some(HeadAttempt {
            words: group.words.clone(), slot, phase, lease, owners: Vec::new(),
        });
        Ok(())
    }

    /// Output lanes, exact logical row capacity, and `[cursor, failure]` pair.
    /// Dispatches may only write complete reservations within the capacity, on
    /// the arena stream, before the next finish or retry; a probe may have zero
    /// capacity. All writers use the same checked native packing.
    pub fn head_target(&self, packing: &Packing) -> Result<(*mut u64, usize, *mut u32)> {
        let head = self.head.as_ref().ok_or("no active candidate head")?;
        if head.lease.packing() != packing {
            return Err("candidate head packing differs from dispatch".into());
        }
        Ok((head.lease.buffer().ptr(), head.lease.capacity(), head.cursor()))
    }

    /// Keep this dispatch's input dependencies alive through publication.
    pub fn retain_head_owners(&mut self, owners: Vec<Arc<dyn Any + Send + Sync>>) {
        let held = &mut self.head.as_mut().expect("no active candidate head").owners;
        if held.is_empty() {
            *held = owners;
        } else {
            held.extend(owners);
        }
    }

    pub fn head_capacity(&self) -> usize {
        self.head.as_ref().expect("no active candidate head").lease.capacity()
    }

    /// Retry only this head on unchanged inputs. Reserve the observed count
    /// plus bounded headroom, reusing the slot's full capacity if larger.
    pub fn retry_head(&mut self, required: usize) -> Result<()> {
        if required > u32::MAX as usize {
            return Err("candidate row capacity exceeds u32".into());
        }
        let mut head = self.head.take().ok_or("no active candidate head")?;
        head.reset()?;
        let CandidateLease { buffer, packing, .. } = head.lease;
        drop(buffer);
        head.owners.clear();
        let hint = head_capacity_hint(required, &packing);
        head.lease = self.reserve_ordered(packing, hint, self.mixed_head[head.phase])?;
        self.head = Some(head);
        Ok(())
    }

    /// Publish one prefix for every append to this head and remember its count.
    ///
    /// # Safety
    /// The checked group result must have zero failure and `rows` initialized
    /// native packed rows for this slot. All writers must be finished in stream
    /// order and may never write this prefix again. See [`CandidateLease::finish`].
    pub unsafe fn finish_head(&mut self, rows: usize) -> Result<CandidateBatch> {
        let head = self.head.take().ok_or("no active candidate head")?;
        head.reset()?;
        let hint = head_capacity_hint(rows, head.lease.packing());
        // SAFETY: the group check and stream-ordered writers establish the premise.
        let batch = unsafe { head.lease.finish(rows, head.owners)? };
        self.head_hints[head.phase] = hint;
        Ok(batch)
    }

    /// Diagnostic only: read one cumulative cursor, never candidate rows.
    pub fn trace_head_count(&self) -> Result<usize> {
        let head = self.head.as_ref().ok_or("no active candidate head")?;
        let mut count = 0u32;
        // SAFETY: the pair is initialized and alive; the download completes
        // before the destination goes out of scope.
        unsafe {
            cuda_core::simt::memory::memcpy_dtoh_async(
                &mut count as *mut u32, head.cursor() as u64, size_of::<u32>(),
                self.pool.stream().cu_stream(),
            )?;
        }
        self.pool.stream().synchronize()?;
        crate::record_readback();
        Ok(count as usize)
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
        counts.hint(word_bytes)
    }
    /// Remember the selected hole's fully validated count, including an empty result.
    pub fn record_count(&mut self, rows: usize) {
        // A detached publication contributes no rows to the shared cursor.
        // Its shrinking hint must not evict expensive sibling appends from
        // already-retained capacity. This policy never grows the request.
        if let Some(head) = &self.head {
            self.mixed_head[head.phase] = true;
        }
        let hole = self.hole;
        let counts = &mut self.hole(hole).previous_counts[hole];
        counts.record(rows);
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
        self.head_hints = [0; 2];
        self.mixed_head = [false; 2];
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
        self.reserve_ordered(packing, capacity, false)
    }

    fn reserve_ordered(&mut self, packing: Packing, capacity: usize, largest: bool) -> Result<CandidateLease> {
        let lanes = lane_count(&packing, capacity)?;
        let fitting = self
            .slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| Arc::strong_count(slot) == 1 && slot.len() >= lanes)
            .min_by_key(|(_, slot)| if largest { !slot.len() } else { slot.len() })
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
impl AsRef<Buffer<u64>> for BatchStorage {
    fn as_ref(&self) -> &Buffer<u64> {
        &self.buffer
    }
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
/// The returned allocation is independent of the batches and initialized on
/// `pool`'s stream. Batches on other streams must share its allocator.
pub fn concatenate_batches<W: Word>(
    pool: Arc<Pool>,
    packing: &Packing,
    batches: &[CandidateBatch],
) -> Result<Buffer<W>> {
    gather_ranges(pool, packing, batches.iter().map(|batch| (batch, 0, batch.len())), false)
}

/// Gather one sort portion. Admission can borrow a single initialized range;
/// public concatenation always returns an independent allocation.
pub(crate) fn gather_ranges<'a, W: Word>(
    pool: Arc<Pool>,
    packing: &Packing,
    ranges: impl Iterator<Item = (&'a CandidateBatch, usize, usize)> + Clone,
    allow_view: bool,
) -> Result<Buffer<W>> {
    if W::BITS != packing.word_bits() {
        return Err("candidate concatenation word width differs from packing".into());
    }
    if W::BITS == 128 && W::MS_FIRST {
        return Err("candidate concatenation requires native u128 lane order".into());
    }
    let mut rows = 0usize;
    let mut single = None;
    for (batch, first, count) in ranges.clone() {
        if !pool.shares_memory(batch.pool()) {
            return Err("candidate concatenation pool differs from batch".into());
        }
        if batch.packing() != packing {
            return Err("candidate concatenation packing differs from batch".into());
        }
        if first.checked_add(count).is_none_or(|end| end > batch.len()) {
            return Err("candidate concatenation range exceeds its batch".into());
        }
        if count != 0 {
            single = Some((batch, first, count));
        }
        rows = rows
            .checked_add(count)
            .filter(|&rows| rows <= u32::MAX as usize)
            .ok_or("candidate concatenation exceeds u32 positions")?;
    }
    let bytes_per_word = std::mem::size_of::<W>();
    rows.checked_mul(bytes_per_word)
        .ok_or("candidate concatenation byte count overflow")?;
    if let Some((batch, first, count)) = single.filter(|&(batch, _, count)| {
        // A suballocator need only align a lane allocation as u64. If this
        // particular slot cannot be viewed as a native W, gather as usual.
        allow_view && count == rows && Arc::ptr_eq(pool.stream(), batch.pool().stream())
            && batch.buffer().cu_deviceptr().is_multiple_of(align_of::<W>() as u64)
    }) {
        // SAFETY: CandidateBatch publishes immutable checked native lanes.
        // The typed view retains both the lease and all input dependencies.
        return unsafe { Buffer::shared_cast_slice(batch.storage.clone(), first, count) };
    }
    crate::stats::trace_step(&pool, "admit/concatenate", rows, bytes_per_word);
    // SAFETY: the disjoint copies below initialize exactly the allocated prefix
    // before any subsequent operation on the same stream can consume it.
    let output = unsafe { Buffer::<W>::uninitialized(pool.clone(), rows)? };
    let mut offset = 0usize;
    let mut handoff = None;
    for (batch, first, count) in ranges {
        let bytes = count * bytes_per_word;
        if bytes != 0 {
            // Other allocator lanes can be producing this batch and will
            // reclaim its storage on their own stream. Order both edges.
            let event = if Arc::ptr_eq(pool.stream(), batch.pool().stream()) {
                None
            } else {
                if handoff.is_none() {
                    handoff = Some(pool.context().new_event(None)?);
                }
                let event = handoff.as_ref().unwrap();
                event.record(batch.pool().stream())?;
                pool.stream().wait(event)?;
                Some(event)
            };
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
            if let Some(event) = event {
                if let Err(error) = event.record(pool.stream()).and_then(|()| batch.pool().stream().wait(event)) {
                    // A submitted copy may still read the source. On a failed
                    // handoff, wait here or keep the source if CUDA is lost.
                    if let Err(sync) = pool.stream().synchronize() {
                        pool.context().record_err::<()>(Err(sync));
                        std::mem::forget(batch.owner());
                    }
                    return Err(error.into());
                }
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
        let view = gather_ranges::<u64>(pool.clone(), &packing, std::iter::once((&batch, 1, 2)), true)?;
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

    #[test]
    fn per_hole_hints_preserve_high_water_and_bound_growth_slack() -> Result<()> {
        let mut arena = CandidateArena::new(Pool::new(0, 0, 0)?);
        let packing = Packing::new(&[64; 3])?;
        let row_bytes = size_of::<[u64; 3]>();
        let spike = 1 << 26;
        arena.hole(0).record_count(spike);
        assert_eq!(arena.capacity_hint(&packing), spike + GROWTH_SLACK_BYTES / row_bytes);
        arena.record_count(1);
        assert_eq!(arena.capacity_hint(&packing), SPECULATIVE_SLOT_BYTES / row_bytes);
        arena.hole(1).record_count(3);
        assert_eq!(arena.capacity_hint(&packing), 6);
        assert_eq!(arena.hole(0).previous_count(), 1);
        Ok(())
    }

    #[test]
    fn head_hint_bounds_slack_in_native_bytes() -> Result<()> {
        let narrow = Packing::new(&[32])?;
        let wide = Packing::new(&[64; 3])?;
        assert_eq!(head_capacity_hint(0, &narrow), 0);
        assert_eq!(head_capacity_hint(3, &narrow), 5);
        let rows = 1 << 26;
        assert_eq!(head_capacity_hint(rows, &narrow) - rows, (64 << 20) / 8);
        assert_eq!(head_capacity_hint(rows, &wide) - rows, (64 << 20) / 24);
        let retained = RETAINED_SLOT_BYTES / 8;
        assert_eq!(head_capacity_hint(retained - 1, &narrow), retained);
        assert_eq!(head_capacity_hint(u32::MAX as usize, &narrow), u32::MAX as usize);
        Ok(())
    }

    #[test]
    fn grouped_heads_retry_and_publish_exact_prefixes() -> Result<()> {
        let pool = Pool::new(0, 0, 0)?;
        let packing = Packing::new(&[64])?;
        let group = HeadCursors::new(pool.clone(), 2)?;
        let mut a = CandidateArena::new(pool.clone());
        let mut b = CandidateArena::new(pool);
        let retained = a.reserve(packing.clone(), 19)?.capacity();
        a.begin_head(&group, 0, packing.clone(), 0)?;
        b.begin_head(&group, 1, packing.clone(), 0)?;
        let (probe, cap, cursor) = a.head_target(&packing)?;
        assert_eq!(cap, retained); // capacity already paid for, even on a cold head
        assert_eq!(b.head_target(&packing)?.2, cursor.wrapping_add(2));
        assert!(a.head_target(&Packing::new(&[32])?).is_err());
        // A retry resets both words for A without touching B's independent pair.
        unsafe { group.words.write_async(&[3, 1, 1, 0])? };
        let baseline = crate::readback_count();
        assert_eq!(group.read()?, [3, 1, 1, 0]);
        assert_eq!(crate::readback_count(), baseline + 1);
        assert_eq!(a.trace_head_count()?, 3);
        assert_eq!(crate::readback_count(), baseline + 2);
        a.retry_head(3)?;
        assert_eq!(a.head_target(&packing)?, (probe, retained, cursor));
        assert_eq!(group.read()?, [0, 0, 1, 0]);
        b.retry_head(1)?;
        let input = Arc::new(7);
        let weak = Arc::downgrade(&input);
        a.retain_head_owners(vec![input]);
        unsafe {
            a.head.as_ref().unwrap().lease.buffer().write_async(&[41, 42, 43])?;
            b.head.as_ref().unwrap().lease.buffer().write_async(&[99])?;
            group.words.write_async(&[3, 0, 1, 0])?;
        }
        let result = group.read()?;
        // SAFETY: each lease initialized exactly the prefix reported above.
        let full = unsafe { a.finish_head(result[0] as usize)? };
        let peer = unsafe { b.finish_head(result[2] as usize)? };
        assert!(a.head_target(&packing).is_err());
        assert_eq!(full.buffer().to_host()?, [41, 42, 43]);
        assert_eq!(peer.buffer().to_host()?, [99]);
        assert!(weak.upgrade().is_some());
        drop(full);
        assert!(weak.upgrade().is_none());
        assert_eq!(group.read()?, [0; 4]);
        a.release_unused(); // Test phase hints without masking them with recycled space.
        a.begin_head(&group, 0, packing.clone(), 1)?;
        assert_eq!(a.head_hints[1], 0);
        // SAFETY: no appends in this step; its zero hint is independent of seed.
        assert!(unsafe { a.finish_head(0)? }.buffer().is_empty());
        a.begin_head(&group, 0, packing, 0)?;
        assert_eq!(a.head_hints[0], 5);
        assert!(a.head_capacity() >= 5);
        // SAFETY: an empty prefix requires no initialized rows.
        unsafe { a.finish_head(0)? };
        Ok(())
    }

    #[test]
    fn admission_range_borrows_native_words_and_retains_inputs() -> Result<()> {
        let pool = Pool::new(0, 0, 0)?;
        let packing = Packing::new(&[64, 64])?;
        let mut arena = CandidateArena::new(pool.clone());
        let lease = arena.reserve(packing.clone(), 3)?;
        let ptr = lease.buffer().cu_deviceptr();
        let input = Arc::new(11);
        let weak = Arc::downgrade(&input);
        // SAFETY: each u128 has both native lanes initialized, low first.
        let batch = unsafe {
            lease.buffer().write_async(&[1, 2, 3, 4, 5, 6])?;
            lease.finish(3, vec![input])?
        };
        let range = std::iter::once((&batch, 1, 2));
        assert!(gather_ranges::<[u64; 2]>(pool.clone(), &packing, range.clone(), true).is_err());
        assert!(gather_ranges::<u128>(pool.clone(), &Packing::new(&[63, 64])?, range.clone(), true).is_err());
        assert!(gather_ranges::<u128>(pool.clone(), &packing, std::iter::once((&batch, 2, 2)), true).is_err());
        let lane = pool.lane()?;
        let copy = gather_ranges::<u128>(lane.clone(), &packing, range.clone(), true)?;
        assert!(Arc::ptr_eq(copy.pool(), &lane));
        assert_ne!(copy.cu_deviceptr(), ptr + 16);
        let view = gather_ranges::<u128>(pool, &packing, range, true)?;
        assert_eq!(view.cu_deviceptr(), ptr + 16);
        // SAFETY: both levels borrow immutable initialized native words.
        let tail = unsafe { view.shared_subrange(1, 1)? }.unwrap();
        assert_eq!(tail.cu_deviceptr(), ptr + 32);
        assert_eq!(tail.allocated_bytes(), batch.allocated_bytes());
        drop(batch);
        let expected = [(4u128 << 64) | 3, (6u128 << 64) | 5];
        assert_eq!(copy.to_host()?, expected);
        assert_eq!(view.to_host()?, expected);
        assert!(weak.upgrade().is_some());
        assert_ne!(arena.reserve(packing.clone(), 3)?.buffer().cu_deviceptr(), ptr);
        drop(view);
        assert!(weak.upgrade().is_some());
        assert_eq!(tail.to_host()?, [expected[1]]);
        drop(tail);
        assert!(weak.upgrade().is_none());
        assert_eq!(arena.reserve(packing, 3)?.buffer().cu_deviceptr(), ptr);
        Ok(())
    }

    #[test]
    fn admission_range_preserves_wide_native_lane_order() -> Result<()> {
        let pool = Pool::new(0, 0, 0)?;
        let packing = Packing::new(&[64; 3])?;
        let mut arena = CandidateArena::new(pool.clone());
        let lease = arena.reserve(packing.clone(), 2)?;
        // SAFETY: both complete rows are initialized in MS-first order.
        let batch = unsafe {
            lease.buffer().write_async(&[1, 2, 3, 4, 5, 6])?;
            lease.finish(2, vec![])?
        };
        let range = std::iter::once((&batch, 1, 1));
        let view = gather_ranges::<[u64; 3]>(pool, &packing, range, true)?;
        assert_eq!(view.cu_deviceptr(), batch.buffer().cu_deviceptr() + 24);
        drop(batch);
        assert_eq!(view.to_host()?, [[4, 5, 6]]);
        Ok(())
    }
}
