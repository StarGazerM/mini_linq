//! Host publication of final answers.
//!
//! Rows come out in key order with nothing allocated per row. A relation is
//! held as sorted runs: its longest run leaves the device through two reusable
//! pinned chunks at the bus rate, and every host thread merges its part of a
//! chunk with the matching keys of the other runs straight into the one result
//! allocation while the next chunk is in flight. Only the shorter runs get a
//! host copy. Small relations skip the pinned staging and its allocation cost.
use crate::{
    pack::{PackedRow, Packing},
    Buffer, Pool, Result, Word,
};
use cuda_core::PinnedHostBuffer;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Condvar, Mutex,
};

/// Bytes of one pinned chunk. Two are allocated on the first staged download.
const CHUNK_BYTES: usize = 32 << 20;
/// A run of at least this many bytes is staged; allocating the pinned chunks
/// costs about as much as a pageable download of this size.
const STAGED_BYTES: usize = 4 * CHUNK_BYTES;
/// Host work on at least this many bytes is divided over all threads.
const PARALLEL_BYTES: usize = 8 << 20;

/// Pinned staging and the thread count for one program's final downloads.
/// Dropping it frees the pinned memory.
pub struct OutputStaging {
    pool: Arc<Pool>,
    chunks: Option<[PinnedHostBuffer<u8>; 2]>,
    threads: usize,
}

/// A piece of a sorted run: its first row, the key before it, and its keys.
type Sink<'a, W> = &'a (dyn Fn(usize, Option<W>, &[W]) + Sync);

impl OutputStaging {
    /// Allocates nothing until a run of [`STAGED_BYTES`] needs it.
    pub fn new(pool: &Arc<Pool>) -> Self {
        Self {
            pool: pool.clone(),
            chunks: None,
            threads: std::thread::available_parallelism().map_or(1, usize::from),
        }
    }

    /// Calls `sink` for consecutive disjoint non-empty pieces of the first
    /// `rows` keys of `source`, concurrently on this staging's threads. Each
    /// piece is only valid during its call.
    fn stream<W: Word>(
        &mut self,
        source: &Buffer<W>,
        rows: usize,
        sink: Sink<'_, W>,
    ) -> Result<()> {
        if self.chunks.is_none() {
            let context = self.pool.context();
            let chunk = || PinnedHostBuffer::<u8>::zeroed(context, CHUNK_BYTES);
            self.chunks = Some([chunk()?, chunk()?]);
        }
        // Pinned allocations are page aligned, so every chunk holds whole keys.
        let chunks = self
            .chunks
            .as_mut()
            .unwrap()
            .each_mut()
            .map(|chunk| chunk.as_mut_ptr().cast::<W>());
        let chunk_rows = CHUNK_BYTES / size_of::<W>();
        let stream = source.pool().stream();
        let copy = |index: usize| -> Result<()> {
            let first = index * chunk_rows;
            unsafe {
                cuda_core::simt::memory::memcpy_dtoh_async(
                    chunks[index % 2],
                    source.cu_deviceptr() + (first * size_of::<W>()) as u64,
                    chunk_rows.min(rows - first) * size_of::<W>(),
                    stream.cu_stream(),
                )?;
            }
            Ok(())
        };
        let team = Team::default();
        let workers = self.threads;
        std::thread::scope(|scope| {
            for worker in 0..workers {
                let team = &team;
                scope.spawn(move || team.work(worker, workers, sink));
            }
            let result = (|| {
                copy(0)?;
                let mut before = None;
                for index in 0..rows.div_ceil(chunk_rows) {
                    // Only this download is on the stream: chunk `index` is complete,
                    // and the other chunk is free to receive the next one meanwhile.
                    stream.synchronize()?;
                    let (first, keys) = (index * chunk_rows, chunks[index % 2]);
                    let len = chunk_rows.min(rows - first);
                    if first + chunk_rows < rows {
                        copy(index + 1)?;
                    }
                    if !team.run(Job {
                        keys: keys as usize,
                        len,
                        first,
                        before,
                        pending: workers,
                        ..Job::default()
                    }) {
                        // A sink panicked; the scope reports it once the workers stop.
                        break;
                    }
                    before = Some(unsafe { *keys.add(len - 1) });
                }
                // An abandoned transfer must not outlive its pinned destination.
                stream.synchronize()?;
                Ok(())
            })();
            team.quit();
            result
        })
    }
}

/// One chunk at a time, shared by scoped workers that stay alive across
/// chunks: worker `w` of `n` handles the `w`-th of `n` equal parts.
struct Team<W> {
    job: Mutex<Job<W>>,
    start: Condvar,
    done: Condvar,
}

struct Job<W> {
    generation: u64,
    keys: usize,
    len: usize,
    first: usize,
    before: Option<W>,
    pending: usize,
    failed: bool,
    quit: bool,
}

impl<W> Default for Job<W> {
    fn default() -> Self {
        Self {
            generation: 0,
            keys: 0,
            len: 0,
            first: 0,
            before: None,
            pending: 0,
            failed: false,
            quit: false,
        }
    }
}

impl<W> Default for Team<W> {
    fn default() -> Self {
        Self {
            job: Mutex::default(),
            start: Condvar::new(),
            done: Condvar::new(),
        }
    }
}

impl<W: Word> Team<W> {
    /// Runs one chunk on every worker; false when a worker panicked.
    fn run(&self, chunk: Job<W>) -> bool {
        let mut job = self.job.lock().unwrap();
        *job = Job {
            generation: job.generation + 1,
            failed: job.failed,
            ..chunk
        };
        self.start.notify_all();
        while job.pending != 0 {
            job = self.done.wait(job).unwrap();
        }
        !job.failed
    }

    fn quit(&self) {
        self.job.lock().unwrap().quit = true;
        self.start.notify_all();
    }

    fn work(&self, worker: usize, workers: usize, sink: Sink<'_, W>) {
        struct Finish<'a, W>(&'a Team<W>);
        impl<W> Drop for Finish<'_, W> {
            fn drop(&mut self) {
                let mut job = self.0.job.lock().unwrap();
                job.pending -= 1;
                job.failed |= std::thread::panicking();
                self.0.done.notify_one();
            }
        }
        let mut seen = 0;
        loop {
            let (keys, len, first, before) = {
                let mut job = self.job.lock().unwrap();
                while job.generation == seen && !job.quit {
                    job = self.start.wait(job).unwrap();
                }
                if job.generation == seen {
                    return;
                }
                seen = job.generation;
                (job.keys as *const W, job.len, job.first, job.before)
            };
            let _finish = Finish(self);
            let (low, high) = (worker * len / workers, (worker + 1) * len / workers);
            if low != high {
                // The chunk stays unchanged until every worker has finished it.
                let before = if low == 0 {
                    before
                } else {
                    Some(unsafe { *keys.add(low - 1) })
                };
                sink(first + low, before, unsafe {
                    std::slice::from_raw_parts(keys.add(low), high - low)
                });
            }
        }
    }
}

/// A host allocation written at disjoint positions from several threads.
struct Slots<T>(*mut T);
// SAFETY: every user writes only its own disjoint range of the allocation.
unsafe impl<T: Send> Sync for Slots<T> {}
impl<T> Slots<T> {
    fn at(&self, index: usize) -> *mut T {
        unsafe { self.0.add(index) }
    }
}

/// An untouched allocation for `len` values. Large ones ask for huge pages:
/// first touch from every thread is otherwise bound by page faults (measured
/// 11 against 19 GB/s on 24 threads).
fn allocate<T>(len: usize) -> Vec<T> {
    let values = Vec::<T>::with_capacity(len);
    let (start, bytes) = (values.as_ptr() as usize, len * size_of::<T>());
    if bytes >= CHUNK_BYTES {
        const PAGE: usize = 4096;
        let first = start.next_multiple_of(PAGE);
        // Advisory: a refusal leaves ordinary pages.
        unsafe {
            libc::madvise(
                first as *mut libc::c_void,
                (start + bytes - first) / PAGE * PAGE,
                libc::MADV_HUGEPAGE,
            )
        };
    }
    values
}

/// Decodes keys into result rows and validates them once per relation.
struct Decode<'a, W, R> {
    packing: &'a Packing,
    row: &'a R,
    stray: AtomicBool,
    marker: std::marker::PhantomData<fn(W)>,
}

impl<W: Word, T, R: Fn(PackedRow<'_, W>) -> T> Decode<'_, W, R> {
    /// Writes the rows of sorted `runs` from `output` on, in key order.
    ///
    /// # Safety
    /// `output` must be valid for as many writes as `runs` hold keys, and no
    /// one else may perform them.
    unsafe fn merge(&self, mut runs: Vec<&[W]>, mut output: *mut T) {
        let mut all = W::zero();
        let mut emit = |key: W| {
            for lane in 0..W::BITS / 64 {
                all.set_lane(lane, all.lane(lane) | key.lane(lane));
            }
            unsafe {
                output.write((self.row)(self.packing.row(key)));
                output = output.add(1);
            }
        };
        runs.retain(|run| !run.is_empty());
        while runs.len() > 1 {
            // The run with the smallest head supplies rows up to the next smallest head.
            let (mut least, mut bound) = if runs[1][0] < runs[0][0] {
                (1, runs[0][0])
            } else {
                (0, runs[1][0])
            };
            for index in 2..runs.len() {
                let head = runs[index][0];
                if head < runs[least][0] {
                    (least, bound) = (index, runs[least][0]);
                } else if head < bound {
                    bound = head;
                }
            }
            let run = runs[least];
            let mut taken = 0;
            while taken != run.len() && run[taken] <= bound {
                emit(run[taken]);
                taken += 1;
            }
            runs[least] = &run[taken..];
            if taken == run.len() {
                runs.swap_remove(least);
            }
        }
        for &key in runs.first().copied().unwrap_or_default() {
            emit(key);
        }
        if self.packing.stray_bits(&all) {
            self.stray.store(true, Ordering::Relaxed);
        }
    }
}

/// Decoded rows of sorted runs of `sizes` keys, in key order. `source(index)`
/// yields run `index`'s device keys; each run is requested once and released
/// before the next.
pub(crate) fn rows<W: Word, T: Send>(
    staging: &mut OutputStaging,
    packing: &Packing,
    sizes: &[usize],
    mut source: impl FnMut(usize) -> Result<Arc<Buffer<W>>>,
    row: &(impl Fn(PackedRow<'_, W>) -> T + Sync),
) -> Result<Vec<T>> {
    if packing.bits() > W::BITS {
        return Err("packing exceeds the selected word width".into());
    }
    let total = sizes.iter().sum::<usize>();
    let Some((longest, &rows)) = sizes
        .iter()
        .enumerate()
        .max_by_key(|&(_, &rows)| rows)
        .filter(|_| total != 0)
    else {
        return Ok(Vec::new());
    };
    let staged = |rows: usize| rows * size_of::<W>() >= STAGED_BYTES;
    // Host copies of every run but the longest.
    let mut others = Vec::with_capacity(sizes.len() - 1);
    for (index, &rows) in sizes.iter().enumerate() {
        if index == longest || rows == 0 {
            continue;
        }
        let keys = source(index)?;
        others.push(if staged(rows) {
            let mut host = allocate::<W>(rows);
            let slots = Slots(host.as_mut_ptr());
            staging.stream(&keys, rows, &|first, _, keys: &[W]| unsafe {
                std::ptr::copy_nonoverlapping(keys.as_ptr(), slots.at(first), keys.len());
            })?;
            unsafe { host.set_len(rows) };
            host
        } else {
            keys.to_host_prefix(rows)?
        });
    }
    let mut output = allocate::<T>(total);
    let slots = Slots(output.as_mut_ptr());
    let decode = Decode {
        packing,
        row,
        stray: AtomicBool::new(false),
        marker: std::marker::PhantomData,
    };
    // A piece of the longest run takes the other runs' keys above the key
    // before it, up to its own last key; the last piece takes all that remain.
    let piece = |first: usize, before: Option<W>, keys: &[W]| {
        let last = (first + keys.len() != rows).then(|| keys[keys.len() - 1]);
        let mut at = first;
        let mut runs = Vec::with_capacity(others.len() + 1);
        for run in &others {
            let low = before.map_or(0, |before| run.partition_point(|&key| key <= before));
            let high = last.map_or(run.len(), |last| run.partition_point(|&key| key <= last));
            at += low;
            runs.push(&run[low..high]);
        }
        runs.push(keys);
        unsafe { decode.merge(runs, slots.at(at)) };
    };
    let keys = source(longest)?;
    if staged(rows) {
        staging.stream(&keys, rows, &piece)?;
    } else {
        let host = keys.to_host_prefix(rows)?;
        let parts = if total * size_of::<W>() >= PARALLEL_BYTES {
            staging.threads.min(rows)
        } else {
            1
        };
        std::thread::scope(|scope| {
            for part in 0..parts {
                let (low, high) = (part * rows / parts, (part + 1) * rows / parts);
                let (piece, host) = (&piece, &host);
                let work = move || {
                    piece(
                        low,
                        low.checked_sub(1).map(|before| host[before]),
                        &host[low..high],
                    )
                };
                if parts == 1 {
                    work()
                } else {
                    scope.spawn(work);
                }
            }
        });
    }
    if decode.stray.load(Ordering::Relaxed) {
        return Err("packed key contains bits outside its schema".into());
    }
    unsafe { output.set_len(total) };
    Ok(output)
}
