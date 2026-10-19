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

/// Decode a root scan vector into this lane's four striped registers.
#[device]
#[inline(always)]
pub unsafe fn column_tile(table: TableView, column: usize, vector: u64, lane: u32) -> [i64; 4] {
    unsafe { gpu_device_columns::decode_tile(table.columns.add(column).read(), vector, lane) }
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
