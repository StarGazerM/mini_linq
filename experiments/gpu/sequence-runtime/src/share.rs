//! Formal rule GPU-ExpansionShare: a pool of published expansions.
//!
//! The pool's metadata starts with two words (the number of published slots
//! and a pad), then one four-word header per slot: the claim word (`u64`,
//! next batch in the low half, end batch in the high half), the slot state and
//! the number of blocks copying its body. Functions take the pool's first word.
//! The body (`stride` words per slot) lives in a buffer retained per memory
//! pool; its layout belongs to the emitting kernel.
//!
//! Every batch is claimed exactly once by a `fetch_add` on the claim word: the
//! owner takes one batch at a time, an idle block half of what is left. An
//! idle block counts itself busy and as a reader before claiming, copies the
//! body, then stops reading. The owner frees the slot once its own claim fails
//! and no reader remains, so a body is never overwritten while being copied.
//! Publication starts once some block waits with nothing to steal (hungry);
//! idle blocks probe the slots in parallel, one slot per thread. A block is
//! idle for good when no slot has a batch left and no block is busy with a
//! root tile or a claimed range: only busy blocks publish.

use crate::{Buffer, Pool, Result};
use cuda_device::atomic::{AtomicOrdering, DeviceAtomicU32, DeviceAtomicU64};
use cuda_device::device;
use std::{cell::RefCell, sync::Arc};

/// Metadata words per slot header.
pub const SLOT_WORDS: usize = 4;
/// Pool header words before the first slot: the number of published slots and
/// a pad that keeps every claim word `u64`-aligned.
pub const POOL_WORDS: usize = 2;
const FREE: u32 = 0;
const OWNED: u32 = 1;

#[device]
#[inline(always)]
unsafe fn claim_word<'a>(pool: *mut u32, slot: u32) -> &'a DeviceAtomicU64 {
    unsafe { DeviceAtomicU64::from_ptr(pool.add(POOL_WORDS + slot as usize * SLOT_WORDS) as *mut u64) }
}

#[device]
#[inline(always)]
unsafe fn state<'a>(pool: *mut u32, slot: u32) -> &'a DeviceAtomicU32 {
    unsafe { DeviceAtomicU32::from_ptr(pool.add(POOL_WORDS + slot as usize * SLOT_WORDS + 2)) }
}

#[device]
#[inline(always)]
unsafe fn readers<'a>(pool: *mut u32, slot: u32) -> &'a DeviceAtomicU32 {
    unsafe { DeviceAtomicU32::from_ptr(pool.add(POOL_WORDS + slot as usize * SLOT_WORDS + 3)) }
}

#[device]
#[inline(always)]
unsafe fn published<'a>(pool: *mut u32) -> &'a DeviceAtomicU32 {
    unsafe { DeviceAtomicU32::from_ptr(pool) }
}

/// Take a free slot for publication, scanning from `start`; `u32::MAX` when
/// every slot is owned.
///
/// # Safety
/// `headers` addresses `slots * SLOT_WORDS` aligned metadata words, zero on
/// launch entry, used only through this module during the launch.
#[device]
#[inline(always)]
pub unsafe fn acquire(headers: *mut u32, slots: u32, start: u32) -> u32 {
    let mut visited = 0u32;
    while visited < slots {
        let slot = (start.wrapping_add(visited)) % slots;
        let state = unsafe { state(headers, slot) };
        if state.load(AtomicOrdering::Relaxed) == FREE
            && state
                .compare_exchange(FREE, OWNED, AtomicOrdering::Acquire, AtomicOrdering::Relaxed)
                .is_ok()
        {
            return slot;
        }
        visited += 1;
    }
    u32::MAX
}

/// Make batches `first..end` of an owned slot claimable. The body must be
/// written and fenced by every writer before this call.
///
/// # Safety
/// As for [`acquire`]; `slot` is owned by the calling block.
#[device]
#[inline(always)]
pub unsafe fn publish(headers: *mut u32, slot: u32, first: u32, end: u32) {
    unsafe { claim_word(headers, slot) }
        .store(((end as u64) << 32) | first as u64, AtomicOrdering::Release);
    unsafe { published(headers) }.fetch_add(1, AtomicOrdering::Release);
}

/// Claim one batch of a slot: its index, or `u32::MAX` once all are claimed.
///
/// # Safety
/// As for [`acquire`].
#[device]
#[inline(always)]
pub unsafe fn claim(headers: *mut u32, slot: u32) -> u32 {
    let old = unsafe { claim_word(headers, slot) }.fetch_add(1, AtomicOrdering::AcqRel);
    let (next, end) = (old as u32, (old >> 32) as u32);
    if next < end { next } else { u32::MAX }
}

/// The owner's claims are exhausted: wait until no block copies the body, then
/// free the slot for another publication.
///
/// # Safety
/// As for [`publish`]; called once per publication, after `claim` failed.
#[device]
#[inline(always)]
pub unsafe fn release(headers: *mut u32, slot: u32) {
    let readers = unsafe { readers(headers, slot) };
    while readers.load(AtomicOrdering::Acquire) != 0 {
        unsafe { cuda_device::barrier::nanosleep(64) };
    }
    unsafe { state(headers, slot) }.store(FREE, AtomicOrdering::Release);
    unsafe { published(headers) }.fetch_sub(1, AtomicOrdering::Relaxed);
}

/// Whether `slot` has a published batch left: one thread's probe of a block
/// that scans the pool in parallel. A stale answer only costs a failed steal.
///
/// # Safety
/// As for [`acquire`].
#[device]
#[inline(always)]
pub unsafe fn claimable(headers: *mut u32, slot: u32) -> bool {
    if unsafe { published(headers) }.load(AtomicOrdering::Relaxed) == 0 {
        return false;
    }
    if unsafe { state(headers, slot) }.load(AtomicOrdering::Relaxed) != OWNED {
        return false;
    }
    let word = unsafe { claim_word(headers, slot) }.load(AtomicOrdering::Relaxed);
    (word as u32) < (word >> 32) as u32
}

/// An idle block's claim of half the batches left in `slot` (at least one):
/// `(first, count)`, with `count == 0` when none is left. A successful claim
/// leaves the caller busy and reading the slot. Taking half keeps the number
/// of steals per expansion logarithmic, so each copy of the state pays for
/// real work.
///
/// # Safety
/// As for [`acquire`]; `busy` is the launch's busy word.
#[device]
#[inline(always)]
pub unsafe fn steal(headers: *mut u32, slot: u32, busy: *mut u32) -> (u32, u32) {
    let busy = unsafe { DeviceAtomicU32::from_ptr(busy) };
    let claims = unsafe { claim_word(headers, slot) };
    let word = claims.load(AtomicOrdering::Relaxed);
    let (next, end) = (word as u32, (word >> 32) as u32);
    if next >= end {
        return (0, 0);
    }
    busy.fetch_add(1, AtomicOrdering::Relaxed);
    let readers = unsafe { readers(headers, slot) };
    readers.fetch_add(1, AtomicOrdering::Relaxed);
    let take = ((end - next) / 2).max(1);
    let old = claims.fetch_add(take as u64, AtomicOrdering::AcqRel);
    let (first, end) = (old as u32, (old >> 32) as u32);
    if first < end {
        return (first, take.min(end - first));
    }
    readers.fetch_sub(1, AtomicOrdering::Release);
    busy.fetch_sub(1, AtomicOrdering::Release);
    (0, 0)
}

/// The stealing block finished copying the body of `slot`.
///
/// # Safety
/// As for [`steal`], once per successful steal, after every copying thread's
/// loads (block barrier).
#[device]
#[inline(always)]
pub unsafe fn copied(headers: *mut u32, slot: u32) {
    unsafe { readers(headers, slot) }.fetch_sub(1, AtomicOrdering::Release);
}

/// Whether an idle block may stop: nothing is busy, or the launch failed.
///
/// # Safety
/// `busy` and `failure` address the launch's busy and failure words.
#[device]
#[inline(always)]
pub unsafe fn finished(busy: *mut u32, failure: *mut u32) -> bool {
    unsafe {
        DeviceAtomicU32::from_ptr(busy).load(AtomicOrdering::Acquire) == 0
            || DeviceAtomicU32::from_ptr(failure).load(AtomicOrdering::Relaxed) != 0
    }
}

/// A block starts (`1`) or ends (`u32::MAX`, wrapping) a root tile or batch.
///
/// # Safety
/// `busy` addresses the launch's busy word.
#[device]
#[inline(always)]
pub unsafe fn busy(busy: *mut u32, delta: u32) {
    unsafe { DeviceAtomicU32::from_ptr(busy) }.fetch_add(delta, AtomicOrdering::AcqRel);
}

/// A block that found nothing to steal becomes hungry (`1`); one that then
/// claims a batch stops being hungry (`u32::MAX`, wrapping).
///
/// # Safety
/// `hungry` addresses the launch's hungry word.
#[device]
#[inline(always)]
pub unsafe fn hungry(hungry: *mut u32, delta: u32) {
    unsafe { DeviceAtomicU32::from_ptr(hungry) }.fetch_add(delta, AtomicOrdering::Relaxed);
}

/// Whether publishing pays: more blocks wait with nothing to steal than there
/// are published slots. Until then every block has work or a slot to steal
/// from, and another publication would only add copies, claims and slot scans
/// to a launch that is already balanced.
///
/// # Safety
/// `hungry` addresses the launch's hungry word; `headers` is the pool.
#[device]
#[inline(always)]
pub unsafe fn wanted(hungry: *mut u32, headers: *mut u32) -> bool {
    let hungry = unsafe { DeviceAtomicU32::from_ptr(hungry) }.load(AtomicOrdering::Relaxed);
    hungry != 0 && hungry > unsafe { published(headers) }.load(AtomicOrdering::Relaxed)
}

/// Reserve one immutable continuation for a later launch. Saturation leaves
/// the remaining work with the caller. `used` starts at zero for the pair.
#[device]
#[inline(always)]
pub unsafe fn frontier_slot(used: *mut u32, slots: u32) -> u32 {
    let used = unsafe { DeviceAtomicU32::from_ptr(used) };
    let mut old = used.load(AtomicOrdering::Relaxed);
    while old < slots {
        match used.compare_exchange(old, old + 1, AtomicOrdering::Relaxed, AtomicOrdering::Relaxed) {
            Ok(_) => return old,
            Err(actual) => old = actual,
        }
    }
    u32::MAX - 1
}

/// Return the descriptor containing weighted batch `work`; only called for
/// `work < prefix[slots]`. The first grid has completed all descriptor writes.
#[device]
#[inline(always)]
pub unsafe fn frontier_owner(prefix: *const u64, mut slots: u32, work: u64) -> u32 {
    let mut lo = 0u32;
    while lo < slots {
        let m = lo + (slots - lo) / 2;
        if unsafe { *prefix.add(m as usize + 1) } <= work { lo = m + 1; } else { slots = m; }
    }
    lo
}

thread_local! {
    static BODIES: RefCell<Option<(Arc<Pool>, Buffer<u32>)>> = const { RefCell::new(None) };
}

/// Slot bodies of at least `words` words for kernels on `pool`'s stream. The
/// buffer is retained per thread for the most recent pool and grows by
/// replacement; the old buffer is released on its stream, after every kernel
/// that used it. Bodies need no initialization: a slot's claim word, in the
/// launch's zeroed metadata, is published only after its body is written.
pub fn bodies(pool: &Arc<Pool>, words: usize) -> Result<*mut u32> {
    crate::register_query_cleanup(release_query_cache);
    BODIES.with(|cache| {
        let mut cache = cache.borrow_mut();
        let reusable = cache
            .as_ref()
            .is_some_and(|(owner, buffer)| Arc::ptr_eq(owner, pool) && buffer.len() >= words);
        if !reusable {
            drop(cache.take());
            let buffer = unsafe { Buffer::<u32>::uninitialized(pool.clone(), words.max(1))? };
            *cache = Some((pool.clone(), buffer));
        }
        Ok(cache.as_ref().expect("retained bodies").1.ptr())
    })
}

fn release_query_cache() {
    BODIES.with(|cache| cache.borrow_mut().take());
}
