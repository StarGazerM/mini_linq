//! Nonowning kernel ABI. Views must remain within their owning execution and
//! stream; generated kernels are responsible for column indices and row bounds.
use cuda_core::DeviceCopy;
use cuda_device::atomic::{AtomicOrdering, DeviceAtomicU32, DeviceAtomicU64};
use cuda_device::{device, thread};
use gpu_blocked_run::RankView;

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct TableView {
    pub columns: *const gpu_device_columns::ColumnView,
    pub len: u64,
    pub count: *const u64,
    pub error: *mut u32,
}
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct RunView {
    pub rank: RankView,
    pub columns: *const *const i64,
    pub len: u64,
    pub count: *const u32,
    pub bucket_mask: u64,
}

pub const HASHED_RUN_SEED: u64 = 0xcbf29ce484222325;

/// Shared build/probe mixer; equality is always checked separately.
#[device]
#[inline(always)]
pub fn hashed_run_step(hash: u64, value: i64) -> u64 {
    let mut value = value as u64;
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    (hash ^ value ^ (value >> 31)).wrapping_mul(0x100000001b3)
}
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct OutputView {
    pub columns: *const *mut i64,
    pub capacity: u64,
    pub count: *mut u64,
    pub error: *mut u32,
}
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct MembershipView {
    pub bits: *const u64,
    pub bounds: *const u64,
}
/// Immutable exact multiword keys. Storage is owned by the parent Execution.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct ExactMembershipView {
    pub packed: *const u64,
    pub slots: *const u64,
    pub keys: TableView,
    pub allocation: usize,
    pub width: usize,
}
unsafe impl DeviceCopy for ExactMembershipView {}

#[device]
#[inline(always)]
pub unsafe fn exact_contains(view: ExactMembershipView, keys: &[u64]) -> bool {
    if keys.len() != view.width {
        return false;
    }
    let needed = (unsafe { table_len(view.keys) } as usize)
        .saturating_mul(2)
        .max(2);
    let mut capacity = 2usize;
    while capacity < needed && capacity < view.allocation {
        capacity *= 2;
    }
    capacity = capacity.min(view.allocation);
    let words = view.width.max(1);
    let mut hash = 0x9e37_79b9_7f4a_7c15u64;
    let mut word = 0;
    while word < words {
        hash ^= if view.width == 0 { 0 } else { keys[word] };
        hash = (hash ^ (hash >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        hash = (hash ^ (hash >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        hash ^= hash >> 31;
        word += 1;
    }
    let mut slot = hash as usize & (capacity - 1);
    let mut probes = 0;
    while probes < capacity {
        let existing = unsafe { view.slots.add(slot).read() };
        if existing == 0 {
            return false;
        }
        let other = unsafe { view.packed.add((existing - 1) as usize * words) };
        let mut word = 0;
        let mut same = true;
        while word < words {
            let key = if view.width == 0 { 0 } else { keys[word] };
            if key != unsafe { other.add(word).read() } {
                same = false;
                break;
            }
            word += 1;
        }
        if same {
            return true;
        }
        slot = (slot + 1) & (capacity - 1);
        probes += 1;
    }
    false
}
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct FrontierView {
    pub pool: *mut u32,
    pub slots: u32,
    pub descriptor_words: u32,
    pub cursor: *mut u64,
    pub remaining: *mut u64,
}
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct ResumeView {
    pub state: *mut u64,
    pub control: *mut u32,
}
unsafe impl DeviceCopy for ResumeView {}

#[device]
#[inline(always)]
pub unsafe fn resume_requested(resume: ResumeView) -> bool {
    unsafe { DeviceAtomicU32::from_ptr(resume.control) }.load(AtomicOrdering::Relaxed) != 0
}
#[device]
#[inline(always)]
pub unsafe fn resume_suspend(resume: ResumeView) {
    unsafe { DeviceAtomicU32::from_ptr(resume.control) }.store(1, AtomicOrdering::Relaxed);
}
/// Failed reservations change no counters, so saved continuations resume
/// exactly once without replaying previously written rows.
#[device]
#[inline(always)]
pub unsafe fn resume_reserve(output: OutputView, rows: u64) -> u64 {
    let count = unsafe { DeviceAtomicU64::from_ptr(output.count) };
    let mut current = count.load(AtomicOrdering::Relaxed);
    loop {
        if current > output.capacity || rows > output.capacity - current {
            return u64::MAX;
        }
        match count.compare_exchange_relaxed(current, current + rows) {
            Ok(_) => return current,
            Err(next) => current = next,
        }
    }
}
// SAFETY: C-layout records of device pointers and scalars. Views own no storage.
unsafe impl DeviceCopy for TableView {}
unsafe impl DeviceCopy for RunView {}
unsafe impl DeviceCopy for OutputView {}
unsafe impl DeviceCopy for FrontierView {}
unsafe impl DeviceCopy for MembershipView {}

#[device]
#[inline(always)]
pub unsafe fn membership_contains(view: MembershipView, key: i64) -> bool {
    let min_key = unsafe { view.bounds.read() } as i64;
    let max_key = unsafe { view.bounds.add(1).read() } as i64;
    if key < min_key || key > max_key {
        return false;
    }
    let position = (key as u64).wrapping_sub(min_key as u64);
    unsafe { view.bits.add((position / 64) as usize).read() & (1u64 << (position % 64)) != 0 }
}

#[device]
#[inline(always)]
pub unsafe fn flag_error(output: OutputView, code: u32) {
    let _ = unsafe { DeviceAtomicU32::from_ptr(output.error) }.compare_exchange_relaxed(0, code);
}

#[device]
#[inline(always)]
pub unsafe fn column(table: TableView, column: usize, row: u64) -> i64 {
    unsafe { gpu_device_columns::decode(table.columns.add(column).read(), row) }
}

/// Convergent root scan stripe; all lanes participate before row filtering.
#[device]
#[inline(always)]
pub unsafe fn column_stripe(
    table: TableView,
    column: usize,
    vector: u64,
    lane: u32,
    stripe: usize,
) -> i64 {
    unsafe {
        gpu_device_columns::decode_stripe(table.columns.add(column).read(), vector, lane, stripe)
    }
}
/// Logical initialized row count. Host `len` is only an allocation bound.
#[device]
#[inline(always)]
pub unsafe fn table_len(table: TableView) -> u64 {
    if !table.error.is_null() && unsafe { table.error.read() } != 0 {
        return 0;
    }
    if table.count.is_null() {
        table.len
    } else {
        unsafe { table.count.read() }.min(table.len)
    }
}
#[device]
#[inline(always)]
pub unsafe fn run_len(run: RunView) -> u64 {
    (unsafe { run.count.read() }) as u64
}
#[device]
#[inline(always)]
pub unsafe fn run_column(run: RunView, column: usize, row: u32) -> i64 {
    unsafe { run.columns.add(column).read().add(row as usize).read() }
}
#[device]
#[inline(always)]
pub unsafe fn reserve(output: OutputView, rows: u64) -> u64 {
    let base =
        unsafe { DeviceAtomicU64::from_ptr(output.count) }.fetch_add(rows, AtomicOrdering::Relaxed);
    if rows > u64::MAX - base {
        unsafe {
            flag_error(output, 1);
        }
    }
    base
}
#[device]
#[inline(always)]
pub unsafe fn write_output(output: OutputView, column: usize, row: u64, value: i64) {
    if row < output.capacity {
        unsafe {
            output
                .columns
                .add(column)
                .read()
                .add(row as usize)
                .write(value);
        }
    }
}
#[device]
#[inline(always)]
pub unsafe fn emit<const N: usize>(output: OutputView, values: [i64; N]) {
    let row = unsafe { reserve(output, 1) };
    let mut index = 0;
    while index < N {
        unsafe {
            write_output(output, index, row, values[index]);
        }
        index += 1;
    }
}

/// Stable projection/filter placement, called by all 256 lanes exactly once
/// per source block. `scratch` supplies eight shared u32 words for tiles::scan.
/// The count pass returns MAX for every lane; scatter returns MAX for rejected
/// lanes and an initialized-prefix destination for each selected source row.
/// `row` is the source row (block * 256 + lane), and output.capacity is the
/// source cardinality. The compiler must not use this for expanding operators.
#[device]
#[inline(always)]
pub unsafe fn stable_select(
    output: OutputView,
    counts: *mut u32,
    scatter: bool,
    row: u64,
    valid: bool,
    scratch: *mut u32,
) -> u64 {
    let selected = valid && row < output.capacity;
    let (offset, total) = gpu_tiles::scan(selected as u32, scratch);
    let block = (row / 256) as usize;
    if !scatter {
        if thread::threadIdx_x() == 0 {
            unsafe {
                counts.add(block).write(total);
            }
        }
        return u64::MAX;
    }
    let base = unsafe { counts.add(block).read() } as u64;
    if thread::threadIdx_x() == 0 && row + 256 >= output.capacity {
        unsafe {
            output.count.write(base + total as u64);
        }
    }
    if selected {
        base + offset as u64
    } else {
        u64::MAX
    }
}

#[device]
#[inline(always)]
unsafe fn lock(slot: *mut u32, state: u32) -> bool {
    unsafe { DeviceAtomicU32::from_ptr(slot) }
        .compare_exchange(state, 1, AtomicOrdering::Acquire, AtomicOrdering::Relaxed)
        .is_ok()
}
#[device]
#[inline(always)]
unsafe fn unlock(slot: *mut u32, state: u32) {
    unsafe { DeviceAtomicU32::from_ptr(slot) }.store(state, AtomicOrdering::Release);
}

/// Cooperative 256-thread publication. Scratch is two disjoint shared u32s.
/// The slot is published only after every descriptor word is visible. A full
/// pool changes no counters, allowing the compiler's bounded inline traversal.
#[device]
pub unsafe fn frontier_publish(
    view: FrontierView,
    descriptor: *const u32,
    chunks: u32,
    scratch: *mut u32,
) -> bool {
    let tid = thread::threadIdx_x() as usize;
    if tid == 0 {
        unsafe {
            scratch.write(u32::MAX);
        }
        if chunks != 0 {
            let mut index = 0;
            while index < view.slots {
                let slot = unsafe {
                    view.pool
                        .add(index as usize * (view.descriptor_words as usize + 3))
                };
                if unsafe { lock(slot, 0) } {
                    unsafe {
                        scratch.write(index);
                    }
                    break;
                }
                index += 1;
            }
        }
    }
    thread::sync_threads();
    let index = unsafe { scratch.read() };
    thread::sync_threads();
    if index == u32::MAX {
        return chunks == 0;
    }
    let slot = unsafe {
        view.pool
            .add(index as usize * (view.descriptor_words as usize + 3))
    };
    let mut word = tid;
    while word < view.descriptor_words as usize {
        unsafe {
            slot.add(3 + word).write(descriptor.add(word).read());
        }
        word += 256;
    }
    thread::sync_threads();
    if tid == 0 {
        unsafe {
            slot.add(1).write(0);
            slot.add(2).write(chunks);
            DeviceAtomicU64::from_ptr(view.remaining)
                .fetch_add(chunks as u64, AtomicOrdering::Relaxed);
            unlock(slot, 2);
        }
    }
    thread::sync_threads();
    true
}

/// Cooperative claim, copying under the same lock that protects slot reuse.
/// No slot handle escapes: an exhausted slot may be reused immediately after
/// the descriptor copy barrier, without racing any outstanding consumer.
#[device]
pub unsafe fn frontier_claim(
    view: FrontierView,
    descriptor: *mut u32,
    scratch: *mut u32,
) -> Option<u32> {
    let tid = thread::threadIdx_x() as usize;
    if tid == 0 {
        unsafe {
            scratch.write(u32::MAX);
        }
        let mut index = 0;
        while index < view.slots {
            let slot = unsafe {
                view.pool
                    .add(index as usize * (view.descriptor_words as usize + 3))
            };
            if unsafe { lock(slot, 2) } {
                let chunk = unsafe { slot.add(1).read() };
                let end = unsafe { slot.add(2).read() };
                if chunk < end {
                    unsafe {
                        scratch.write(index);
                        scratch.add(1).write(chunk);
                        slot.add(1).write(chunk + 1);
                    }
                    break;
                }
                unsafe {
                    unlock(slot, 0);
                }
            }
            index += 1;
        }
    }
    thread::sync_threads();
    let index = unsafe { scratch.read() };
    thread::sync_threads();
    if index == u32::MAX {
        return None;
    }
    let slot = unsafe {
        view.pool
            .add(index as usize * (view.descriptor_words as usize + 3))
    };
    let chunk = unsafe { scratch.add(1).read() };
    let mut word = tid;
    while word < view.descriptor_words as usize {
        unsafe {
            descriptor.add(word).write(slot.add(3 + word).read());
        }
        word += 256;
    }
    thread::sync_threads();
    if tid == 0 {
        let exhausted = unsafe { slot.add(1).read() == slot.add(2).read() };
        unsafe {
            unlock(slot, if exhausted { 0 } else { 2 });
        }
    }
    thread::sync_threads();
    Some(chunk)
}
#[device]
#[inline(always)]
pub unsafe fn frontier_next_root(view: FrontierView) -> u64 {
    unsafe { DeviceAtomicU64::from_ptr(view.cursor) }.fetch_add(1, AtomicOrdering::Relaxed)
}
#[device]
#[inline(always)]
pub unsafe fn frontier_pending(view: FrontierView) -> u64 {
    unsafe { DeviceAtomicU64::from_ptr(view.remaining) }.load(AtomicOrdering::Acquire)
}
#[device]
#[inline(always)]
pub unsafe fn frontier_done(view: FrontierView) {
    unsafe { DeviceAtomicU64::from_ptr(view.remaining) }.fetch_sub(1, AtomicOrdering::Release);
}

/// Exact emitted-row set. Slots publish output row IDs after all columns exist.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct FusedSetView {
    pub slots: *mut u64,
    pub capacity: usize,
    #[cfg(feature = "dedup-diagnostics")]
    pub counters: *mut u64,
}
unsafe impl DeviceCopy for FusedSetView {}

/// Diagnostic-only striped counters: inserted, duplicate, busy, full,
/// total probes, maximum probes. These atomics never exist in normal builds.
#[cfg(feature = "dedup-diagnostics")]
#[device]
#[inline(always)]
unsafe fn fused_observe(set: FusedSetView, outcome: usize, probes: usize) {
    let counters = unsafe { set.counters.add(thread::blockIdx_x() as usize % 568 * 6) };
    unsafe { DeviceAtomicU64::from_ptr(counters.add(outcome)) }.fetch_add(1, AtomicOrdering::Relaxed);
    unsafe { DeviceAtomicU64::from_ptr(counters.add(4)) }.fetch_add(probes as u64, AtomicOrdering::Relaxed);
    unsafe { DeviceAtomicU64::from_ptr(counters.add(5)) }.fetch_max(probes as u64, AtomicOrdering::Relaxed);
}

/// 0 = emitted/already present, 1 = retry after reconvergence, 2 = grow.
/// Never wait for a busy owner in this call: it may be another SIMD lane.
#[device]
#[inline(always)]
pub unsafe fn fused_emit<const N: usize>(
    output: OutputView,
    set: FusedSetView,
    values: [i64; N],
) -> u32 {
    // Every lane named here executes the same finite insertion attempt.
    // Reconverge before any busy lane can retry: serial query cursors need
    // this even though cooperative emitters also have a later block scan.
    let participants = cuda_device::warp::active_mask();
    let result = unsafe { fused_try_emit(output, set, values) };
    cuda_device::warp::sync_mask(participants);
    result
}

#[device]
#[inline(always)]
unsafe fn fused_try_emit<const N: usize>(
    output: OutputView,
    set: FusedSetView,
    values: [i64; N],
) -> u32 {
    let mut hash = HASHED_RUN_SEED;
    let mut column = 0;
    while column < N {
        hash = hashed_run_step(hash, values[column]);
        column += 1;
    }
    let mut slot = hash as usize & (set.capacity - 1);
    let mut probes = 0;
    while probes < set.capacity {
        let entry = unsafe { DeviceAtomicU64::from_ptr(set.slots.add(slot)) };
        let mut existing = entry.load(AtomicOrdering::Acquire);
        if existing == 0 {
            match entry.compare_exchange(0, u64::MAX, AtomicOrdering::Acquire, AtomicOrdering::Acquire) {
                Ok(_) => {
                    // A ticket is one atomic operation, not a global CAS
                    // retry loop while holding this slot's publication lock.
                    // Out-of-range tickets publish nothing; suspension clamps
                    // the counter to the completed dense prefix before growth.
                    let row = unsafe { reserve(output, 1) };
                    if row >= output.capacity {
                        entry.store(0, AtomicOrdering::Release);
                        #[cfg(feature = "dedup-diagnostics")]
                        unsafe { fused_observe(set, 3, probes + 1); }
                        return 2;
                    }
                    let mut column = 0;
                    while column < N {
                        unsafe { write_output(output, column, row, values[column]); }
                        column += 1;
                    }
                    entry.store(row + 1, AtomicOrdering::Release);
                    #[cfg(feature = "dedup-diagnostics")]
                    unsafe { fused_observe(set, 0, probes + 1); }
                    return 0;
                }
                Err(value) => existing = value,
            }
        }
        if existing == u64::MAX || existing == 0 {
            #[cfg(feature = "dedup-diagnostics")]
            unsafe { fused_observe(set, 2, probes + 1); }
            return 1;
        }
        let mut same = true;
        let mut column = 0;
        while column < N {
            if unsafe { (*output.columns.add(column)).add((existing - 1) as usize).read() }
                != values[column]
            {
                same = false;
                break;
            }
            column += 1;
        }
        if same {
            #[cfg(feature = "dedup-diagnostics")]
            unsafe { fused_observe(set, 1, probes + 1); }
            return 0;
        }
        slot = (slot + 1) & (set.capacity - 1);
        probes += 1;
    }
    #[cfg(feature = "dedup-diagnostics")]
    unsafe { fused_observe(set, 3, probes); }
    2
}
