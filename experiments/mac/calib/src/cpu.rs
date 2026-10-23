//! CPU primitive calibration.
//!
//! Every primitive is a `Work`. Works for different thread counts of the same
//! (primitive, footprint) share one lazily-initialised state (`Arc<Lazy<S>>`):
//! buffers are allocated on the first `run` (i.e. during warmup, outside the
//! timed region) and freed when the last `Work` referencing them is dropped.
//! Drivers should therefore drop each work after timing it; `cpu_works` orders
//! works by footprint so that peak resident memory is bounded by one
//! primitive's buffers at the largest footprint (~2x footprint).
//!
//! Multi-threaded works run on a persistent spin-then-park pool (one pool per
//! thread count, caller thread participates as thread 0), so thread spawn is
//! not inside the timed region. Spawn cost is measured separately as
//! `cpu_spawn`, pool dispatch as `cpu_pool_dispatch`.

use crate::measure::Work;
use hashbrown::HashMap;
use serde_json::{Value, json};
use std::cell::UnsafeCell;
use std::ops::Range;
use std::sync::atomic::Ordering::{Acquire, Relaxed, Release};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread::JoinHandle;

pub struct CpuCfg {
    pub threads: Vec<usize>,
    pub footprint_bytes: Vec<u64>,
    pub seed: u64,
}

const LINE: usize = 128;
const U32_PER_LINE: usize = LINE / 4;
const MIB: u64 = 1 << 20;
/// Total dependent-load steps per `gather_dep` run (split across threads).
const DEP_STEPS: usize = 1 << 22;
const HIST_BINS: usize = 1 << 16;
/// Cap on hashbrown probes per run (K at 4 GiB is 2^27; 2^24 probes is enough).
const MAX_PROBES: usize = 1 << 24;
const FILL_THREADS: usize = 8;
/// Pool spin iterations before parking (~0.1 ms). Must be short: with more
/// pool threads than cores (hw.ncpu = 15 on M5 Pro), long spinners starve the
/// preempted thread for a scheduler quantum (~2 ms per dispatch).
const SPIN_LIMIT: u32 = 1 << 12;

// ---------------------------------------------------------------------------
// Random numbers

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng((seed ^ 0x9E37_79B9_7F4A_7C15) | 1)
    }
    /// xorshift64*
    #[inline]
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    #[inline]
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn salted(seed: u64, salt: u64, footprint: u64) -> u64 {
    seed ^ salt.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ footprint.rotate_left(17)
}

// ---------------------------------------------------------------------------
// Buffers

/// Heap buffer that may be written through `&self` from several threads at
/// disjoint ranges (or via atomics). Callers guarantee no overlapping access.
struct Buf<T> {
    ptr: *mut [T],
}

unsafe impl<T: Send> Send for Buf<T> {}
unsafe impl<T: Send> Sync for Buf<T> {}

impl<T> Buf<T> {
    fn from_vec(v: Vec<T>) -> Self {
        Buf { ptr: Box::into_raw(v.into_boxed_slice()) }
    }
    fn len(&self) -> usize {
        self.ptr.len()
    }
    fn ptr(&self) -> *mut T {
        self.ptr as *mut T
    }
    /// Caller guarantees no concurrent overlapping access to `r`.
    unsafe fn slice_mut(&self, r: Range<usize>) -> &mut [T] {
        debug_assert!(r.end <= self.len());
        unsafe { std::slice::from_raw_parts_mut(self.ptr().add(r.start), r.end - r.start) }
    }
    /// Caller guarantees no concurrent writes to `r`.
    unsafe fn slice(&self, r: Range<usize>) -> &[T] {
        debug_assert!(r.end <= self.len());
        unsafe { std::slice::from_raw_parts(self.ptr().add(r.start), r.end - r.start) }
    }
}

impl Buf<u32> {
    fn atomics(&self) -> &[AtomicU32] {
        unsafe { std::slice::from_raw_parts(self.ptr() as *const AtomicU32, self.len()) }
    }
}

impl<T> Drop for Buf<T> {
    fn drop(&mut self) {
        unsafe { drop(Box::from_raw(self.ptr)) }
    }
}

/// Touched (non-zero memset, so pages are resident) write buffers.
fn buf_u32(n: usize) -> Buf<u32> {
    Buf::from_vec(vec![0xA5A5_A5A5u32; n])
}
fn buf_u64(n: usize) -> Buf<u64> {
    Buf::from_vec(vec![0xA5A5_A5A5_A5A5_A5A5u64; n])
}

fn par_fill<T: Send>(v: &mut [T], seed: u64, f: impl Fn(&mut Rng) -> T + Sync) {
    let n = v.len();
    let chunk_len = n.div_ceil(FILL_THREADS).max(1);
    std::thread::scope(|s| {
        for (i, c) in v.chunks_mut(chunk_len).enumerate() {
            let f = &f;
            s.spawn(move || {
                let mut rng = Rng::new(seed.wrapping_add(i as u64 * 0x9E37_79B9_7F4A_7C15));
                for x in c {
                    *x = f(&mut rng);
                }
            });
        }
    });
}

fn random_u64(n: usize, seed: u64) -> Vec<u64> {
    let mut v = vec![0u64; n];
    par_fill(&mut v, seed, |r| r.next());
    v
}

fn random_u32(n: usize, seed: u64) -> Vec<u32> {
    let mut v = vec![0u32; n];
    par_fill(&mut v, seed, |r| (r.next() >> 32) as u32);
    v
}

/// `n` random line indices into a buffer of `lines` 128-byte lines, expressed
/// as u32 element offsets (line * 32) so every access is a distinct random line.
fn line_indices(n: usize, lines: usize, seed: u64) -> Vec<u32> {
    let mut v = vec![0u32; n];
    par_fill(&mut v, seed, |r| (r.below(lines) * U32_PER_LINE) as u32);
    v
}

/// Random single-cycle pointer chase over all lines of a `footprint` buffer:
/// `buf[line * 32]` holds the u32 offset of the next line.
fn chase_buffer(footprint: usize, seed: u64) -> Buf<u32> {
    let lines = footprint / LINE;
    let mut perm: Vec<u32> = (0..lines as u32).collect();
    let mut rng = Rng::new(seed);
    for i in (1..lines).rev() {
        let j = rng.below(i + 1);
        perm.swap(i, j);
    }
    // Every page gets touched by the scatter below (128 lines per 16 KiB page).
    let buf = Buf::from_vec(vec![0u32; footprint / 4]);
    let (perm, bufr) = (&perm, &buf);
    std::thread::scope(|s| {
        for t in 0..FILL_THREADS {
            let r = chunk(lines, t, FILL_THREADS);
            s.spawn(move || {
                for i in r {
                    let cur = perm[i] as usize * U32_PER_LINE;
                    let next = perm[(i + 1) % lines] as usize * U32_PER_LINE;
                    unsafe { *bufr.ptr().add(cur) = next as u32 };
                }
            });
        }
    });
    buf
}

#[inline]
fn chunk(n: usize, t: usize, parts: usize) -> Range<usize> {
    n * t / parts..n * (t + 1) / parts
}

// ---------------------------------------------------------------------------
// Persistent worker pool

#[cfg(target_os = "macos")]
fn set_qos() {
    unsafe extern "C" {
        fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: i32) -> i32;
    }
    const QOS_CLASS_USER_INTERACTIVE: u32 = 0x21;
    unsafe { pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE, 0) };
}
#[cfg(not(target_os = "macos"))]
fn set_qos() {}

type Job<'a> = dyn Fn(usize) -> u64 + Sync + 'a;

struct PoolInner {
    generation: AtomicU64,
    done: AtomicUsize,
    stop: AtomicBool,
    job: UnsafeCell<Option<&'static Job<'static>>>,
    /// Dispatching thread of the current job, unparked by the last worker.
    caller: UnsafeCell<Option<std::thread::Thread>>,
    results: Vec<AtomicU64>,
}

// `job`/`caller` are written by the dispatching thread before the `generation`
// release and only read by workers after the matching acquire; the dispatcher
// waits for all workers before returning, so the transmuted lifetime never
// escapes.
unsafe impl Sync for PoolInner {}

/// Spin on `ready` for at most `SPIN_LIMIT` iterations, then park until it
/// holds. Short spins keep back-to-back dispatches fast while letting
/// oversubscribed threads (more pool threads than cores) release their core
/// within ~0.1 ms instead of a scheduler quantum.
fn spin_then_park(ready: impl Fn() -> bool) {
    let mut spins = 0u32;
    while !ready() {
        if spins < SPIN_LIMIT {
            spins += 1;
            std::hint::spin_loop();
        } else {
            std::thread::park();
        }
    }
}

/// Workers 1..threads; the dispatching (caller) thread acts as thread 0.
struct Pool {
    inner: Arc<PoolInner>,
    handles: Vec<JoinHandle<()>>,
}

impl Pool {
    fn new(threads: usize) -> Pool {
        assert!(threads >= 2);
        let inner = Arc::new(PoolInner {
            generation: AtomicU64::new(0),
            done: AtomicUsize::new(0),
            stop: AtomicBool::new(false),
            job: UnsafeCell::new(None),
            caller: UnsafeCell::new(None),
            results: (1..threads).map(|_| AtomicU64::new(0)).collect(),
        });
        let handles = (1..threads)
            .map(|tid| {
                let inner = inner.clone();
                std::thread::Builder::new()
                    .name(format!("calib-{tid}"))
                    .spawn(move || worker(inner, tid))
                    .expect("spawn pool worker")
            })
            .collect();
        Pool { inner, handles }
    }

    fn workers(&self) -> usize {
        self.handles.len()
    }

    fn run(&self, f: &Job<'_>) -> u64 {
        let inner = &*self.inner;
        let workers = self.workers();
        // SAFETY: see `PoolInner` Sync comment.
        unsafe {
            *inner.job.get() = Some(std::mem::transmute::<&Job<'_>, &'static Job<'static>>(f));
            *inner.caller.get() = Some(std::thread::current());
        }
        inner.done.store(0, Relaxed);
        inner.generation.fetch_add(1, Release);
        for h in &self.handles {
            h.thread().unpark();
        }
        let mut acc = f(0);
        spin_then_park(|| inner.done.load(Acquire) == workers);
        for r in &inner.results {
            acc = acc.wrapping_add(r.load(Relaxed));
        }
        unsafe {
            *inner.job.get() = None;
            *inner.caller.get() = None;
        }
        acc
    }
}

fn worker(inner: Arc<PoolInner>, tid: usize) {
    set_qos();
    let workers = inner.results.len();
    let mut seen = 0u64;
    loop {
        spin_then_park(|| inner.stop.load(Relaxed) || inner.generation.load(Acquire) != seen);
        if inner.stop.load(Relaxed) {
            return;
        }
        seen = inner.generation.load(Acquire);
        let (f, caller) = unsafe { ((*inner.job.get()).expect("pool job set"), (*inner.caller.get()).clone().expect("pool caller set")) };
        let r = f(tid);
        inner.results[tid - 1].store(r, Relaxed);
        if inner.done.fetch_add(1, Release) + 1 == workers {
            caller.unpark();
        }
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        self.inner.stop.store(true, Relaxed);
        for h in &self.handles {
            h.thread().unpark();
        }
        for h in self.handles.drain(..) {
            h.join().ok();
        }
    }
}

/// Executes a per-thread closure on `threads` threads (inline when 1).
struct Runner {
    threads: usize,
    pool: Option<Arc<Pool>>,
}

impl Runner {
    fn exec(&self, f: &Job<'_>) -> u64 {
        match &self.pool {
            Some(p) => p.run(f),
            None => f(0),
        }
    }
}

// ---------------------------------------------------------------------------
// Generic work

type Lazy<S> = LazyLock<S, Box<dyn FnOnce() -> S + Send>>;

fn lazy<S>(f: impl FnOnce() -> S + Send + 'static) -> Arc<Lazy<S>> {
    let f: Box<dyn FnOnce() -> S + Send> = Box::new(f);
    Arc::new(LazyLock::new(f))
}

struct CpuWork<S: 'static> {
    op: &'static str,
    runner: Runner,
    bytes: u64,
    items: u64,
    params: Value,
    state: Arc<Lazy<S>>,
    body: fn(&S, &Runner) -> u64,
}

impl<S: Send + Sync + 'static> Work for CpuWork<S> {
    fn name(&self) -> String {
        self.op.into()
    }
    fn processor(&self) -> &'static str {
        "cpu"
    }
    fn threads(&self) -> usize {
        self.runner.threads
    }
    fn bytes(&self) -> u64 {
        self.bytes
    }
    fn items(&self) -> u64 {
        self.items
    }
    fn params(&self) -> Value {
        self.params.clone()
    }
    fn run(&self) -> u64 {
        let s: &S = &self.state;
        (self.body)(s, &self.runner)
    }
}

struct Ctx {
    pools: std::collections::HashMap<usize, Arc<Pool>>,
    out: Vec<Box<dyn Work>>,
}

impl Ctx {
    fn runner(&self, threads: usize) -> Runner {
        Runner { threads, pool: self.pools.get(&threads).cloned() }
    }

    /// Emit one work per thread count in `threads`, all sharing `state`.
    /// `meta(threads) -> (bytes, items, params)`.
    fn push<S: Send + Sync + 'static>(
        &mut self,
        op: &'static str,
        threads: &[usize],
        state: &Arc<Lazy<S>>,
        body: fn(&S, &Runner) -> u64,
        meta: impl Fn(usize) -> (u64, u64, Value),
    ) {
        for &t in threads {
            let (bytes, items, params) = meta(t);
            self.out.push(Box::new(CpuWork {
                op,
                runner: self.runner(t),
                bytes,
                items,
                params,
                state: state.clone(),
                body,
            }));
        }
    }
}

// ---------------------------------------------------------------------------
// Kernels

fn sum_u64(s: &[u64]) -> u64 {
    let mut acc = [0u64; 8];
    let mut it = s.chunks_exact(8);
    for c in &mut it {
        for i in 0..8 {
            acc[i] = acc[i].wrapping_add(c[i]);
        }
    }
    let mut r = it.remainder().iter().fold(0u64, |a, &x| a.wrapping_add(x));
    for a in acc {
        r = r.wrapping_add(a);
    }
    r
}

/// Independent random loads, 8 accumulators so many misses stay in flight.
/// `idx` entries must be in range.
unsafe fn gather_sum(data: &[u32], idx: &[u32]) -> u64 {
    let mut acc = [0u32; 8];
    let mut it = idx.chunks_exact(8);
    for c in &mut it {
        for l in 0..8 {
            acc[l] = acc[l].wrapping_add(unsafe { *data.get_unchecked(c[l] as usize) });
        }
    }
    for &i in it.remainder() {
        acc[0] = acc[0].wrapping_add(unsafe { *data.get_unchecked(i as usize) });
    }
    acc.iter().fold(0u64, |a, &x| a.wrapping_add(x as u64))
}

/// Merge-path split: for output diagonal `d`, returns `(i, j)` with `i + j == d`
/// such that a stable merge (ties taken from `a`) outputs exactly `a[..i]` and
/// `b[..j]` first.
fn merge_path(a: &[u32], b: &[u32], d: usize) -> (usize, usize) {
    let mut lo = d.saturating_sub(b.len());
    let mut hi = d.min(a.len());
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if a[mid] <= b[d - mid - 1] {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    (lo, d - lo)
}

/// Branchless sequential merge of sorted `a`, `b` into `out[..a.len()+b.len()]`.
unsafe fn merge_seq(a: &[u32], b: &[u32], out: *mut u32) {
    let (mut i, mut j, mut k) = (0usize, 0usize, 0usize);
    unsafe {
        while i < a.len() && j < b.len() {
            let x = *a.get_unchecked(i);
            let y = *b.get_unchecked(j);
            let take_a = x <= y;
            *out.add(k) = if take_a { x } else { y };
            i += take_a as usize;
            j += (!take_a) as usize;
            k += 1;
        }
        std::ptr::copy_nonoverlapping(a.as_ptr().add(i), out.add(k), a.len() - i);
        k += a.len() - i;
        std::ptr::copy_nonoverlapping(b.as_ptr().add(j), out.add(k), b.len() - j);
    }
}

/// Merge `part` of `parts` equal output slices of `a ∪ b` into `out`
/// (`out` points at the start of the full merged output).
unsafe fn merge_part(a: &[u32], b: &[u32], out: *mut u32, part: usize, parts: usize) {
    let total = a.len() + b.len();
    let r = chunk(total, part, parts);
    let (i0, j0) = merge_path(a, b, r.start);
    let (i1, j1) = merge_path(a, b, r.end);
    unsafe { merge_seq(&a[i0..i1], &b[j0..j1], out.add(r.start)) }
}

// ---------------------------------------------------------------------------
// States and bodies

struct SeqState {
    src: Vec<u64>,
    dst: Buf<u64>,
}

fn seq_read(s: &SeqState, r: &Runner) -> u64 {
    let n = s.src.len();
    r.exec(&|t| sum_u64(&s.src[chunk(n, t, r.threads)]))
}

fn seq_write(s: &SeqState, r: &Runner) -> u64 {
    let n = s.dst.len();
    r.exec(&|t| {
        let d = unsafe { s.dst.slice_mut(chunk(n, t, r.threads)) };
        let v = 0x1234_5678_9ABC_DEF0u64.wrapping_add(t as u64);
        d.fill(v);
        d.first().copied().unwrap_or(0)
    })
}

fn seq_copy(s: &SeqState, r: &Runner) -> u64 {
    let n = s.dst.len();
    r.exec(&|t| {
        let rg = chunk(n, t, r.threads);
        let d = unsafe { s.dst.slice_mut(rg.clone()) };
        d.copy_from_slice(&s.src[rg]);
        d.last().copied().unwrap_or(0)
    })
}

struct RandState {
    data: Vec<u32>,
    dst: Buf<u32>,
    idx: Vec<u32>,
}

fn gather_line(s: &RandState, r: &Runner) -> u64 {
    let n = s.idx.len();
    r.exec(&|t| unsafe { gather_sum(&s.data, &s.idx[chunk(n, t, r.threads)]) })
}

fn scatter_line(s: &RandState, r: &Runner) -> u64 {
    let n = s.idx.len();
    r.exec(&|t| {
        let at = s.dst.atomics();
        let rg = chunk(n, t, r.threads);
        let v = t as u32 + 1;
        for &i in &s.idx[rg.clone()] {
            unsafe { at.get_unchecked(i as usize) }.store(v, Relaxed);
        }
        rg.len() as u64
    })
}

fn rmw_atomic(s: &RandState, r: &Runner) -> u64 {
    let n = s.idx.len();
    r.exec(&|t| {
        let at = s.dst.atomics();
        let mut acc = 0u64;
        for &i in &s.idx[chunk(n, t, r.threads)] {
            acc = acc.wrapping_add(unsafe { at.get_unchecked(i as usize) }.fetch_add(1, Relaxed) as u64);
        }
        acc
    })
}

struct ChaseState {
    chase: Buf<u32>,
}

fn gather_dep(s: &ChaseState, r: &Runner) -> u64 {
    let lines = s.chase.len() / U32_PER_LINE;
    let steps = DEP_STEPS / r.threads;
    let buf = unsafe { s.chase.slice(0..s.chase.len()) };
    r.exec(&|t| {
        let mut p = (lines * t / r.threads) * U32_PER_LINE;
        for _ in 0..steps {
            p = unsafe { *buf.get_unchecked(p) } as usize;
        }
        p as u64
    })
}

struct HashState {
    keys: Vec<u64>,
    probes: Vec<u64>,
    map: Mutex<HashMap<u64, u32>>,
}

fn hashbrown_build(s: &HashState, _r: &Runner) -> u64 {
    let mut m = s.map.lock().unwrap();
    m.clear();
    for (i, &k) in s.keys.iter().enumerate() {
        m.insert(k, i as u32);
    }
    m.len() as u64
}

fn hashbrown_probe(s: &HashState, r: &Runner) -> u64 {
    let g = s.map.lock().unwrap();
    let m: &HashMap<u64, u32> = &g;
    let n = s.probes.len();
    r.exec(&|t| {
        let mut acc = 0u64;
        for k in &s.probes[chunk(n, t, r.threads)] {
            if let Some(&v) = m.get(k) {
                acc = acc.wrapping_add(v as u64 + 1);
            }
        }
        acc
    })
}

struct SortState {
    input: Vec<u32>,
    work: Buf<u32>,
    tmp: Buf<u32>,
}

/// threads == 1: copy + `sort_unstable`. threads > 1: per-thread chunk sort,
/// then a 2-way merge tree where every merge is merge-path split across all
/// threads.
fn sort_u32(s: &SortState, r: &Runner) -> u64 {
    let t = r.threads;
    let n = s.input.len();
    r.exec(&|tid| {
        let rg = chunk(n, tid, t);
        let w = unsafe { s.work.slice_mut(rg.clone()) };
        w.copy_from_slice(&s.input[rg]);
        w.sort_unstable();
        0
    });
    let mut runs: Vec<Range<usize>> = (0..t).map(|i| chunk(n, i, t)).collect();
    let (mut src, mut dst) = (&s.work, &s.tmp);
    while runs.len() > 1 {
        let pairs: Vec<(Range<usize>, Range<usize>)> = runs
            .chunks(2)
            .map(|c| (c[0].clone(), c.get(1).cloned().unwrap_or(c[0].end..c[0].end)))
            .collect();
        r.exec(&|tid| {
            for (ra, rb) in &pairs {
                unsafe {
                    let a = src.slice(ra.clone());
                    let b = src.slice(rb.clone());
                    merge_part(a, b, dst.ptr().add(ra.start), tid, t);
                }
            }
            0
        });
        runs = pairs.iter().map(|(ra, rb)| ra.start..rb.end).collect();
        std::mem::swap(&mut src, &mut dst);
    }
    unsafe { *src.ptr().add(n / 2) as u64 }
}

struct MergeState {
    a: Vec<u32>,
    b: Vec<u32>,
    out: Buf<u32>,
}

fn merge_u32(s: &MergeState, r: &Runner) -> u64 {
    let n = s.out.len();
    r.exec(&|t| {
        unsafe { merge_part(&s.a, &s.b, s.out.ptr(), t, r.threads) };
        unsafe { *s.out.ptr().add(chunk(n, t, r.threads).start) as u64 }
    })
}

struct HistState {
    keys: Vec<u32>,
    /// `max_threads * HIST_BINS` private histograms.
    hists: Buf<u32>,
    global: Buf<u32>,
}

fn hist_scan(s: &HistState, r: &Runner) -> u64 {
    let t = r.threads;
    let n = s.keys.len();
    r.exec(&|tid| {
        let h = unsafe { s.hists.slice_mut(tid * HIST_BINS..(tid + 1) * HIST_BINS) };
        let h: &mut [u32; HIST_BINS] = h.try_into().unwrap();
        h.fill(0);
        for &k in &s.keys[chunk(n, tid, t)] {
            h[(k >> 16) as usize] += 1;
        }
        0
    });
    r.exec(&|tid| {
        let rg = chunk(HIST_BINS, tid, t);
        let g = unsafe { s.global.slice_mut(rg.clone()) };
        let all = unsafe { s.hists.slice(0..t * HIST_BINS) };
        for (gi, bin) in g.iter_mut().zip(rg) {
            let mut sum = 0u32;
            for w in 0..t {
                sum = sum.wrapping_add(all[w * HIST_BINS + bin]);
            }
            *gi = sum;
        }
        0
    });
    let g = unsafe { s.global.slice_mut(0..HIST_BINS) };
    let mut acc = 0u32;
    for x in g.iter_mut() {
        let v = *x;
        *x = acc;
        acc = acc.wrapping_add(v);
    }
    ((acc as u64) << 32) | g[HIST_BINS / 2] as u64
}

fn cpu_spawn(_: &(), r: &Runner) -> u64 {
    std::thread::scope(|s| {
        let hs: Vec<_> = (0..r.threads).map(|i| s.spawn(move || i as u64)).collect();
        hs.into_iter().map(|h| h.join().unwrap()).sum()
    })
}

fn pool_dispatch(_: &(), r: &Runner) -> u64 {
    r.exec(&|t| t as u64)
}

// ---------------------------------------------------------------------------
// Suite

pub fn cpu_works(cfg: &CpuCfg) -> Vec<Box<dyn Work>> {
    set_qos();
    let mut threads: Vec<usize> = cfg.threads.iter().copied().filter(|&t| t >= 1).collect();
    threads.sort_unstable();
    threads.dedup();
    let mut footprints = cfg.footprint_bytes.clone();
    footprints.sort_unstable();
    footprints.dedup();
    let max_threads = threads.last().copied().unwrap_or(1);
    let multi: Vec<usize> = threads.iter().copied().filter(|&t| t > 1).collect();
    let seed = cfg.seed;

    let mut cx = Ctx {
        pools: multi.iter().map(|&t| (t, Arc::new(Pool::new(t)))).collect(),
        out: Vec::new(),
    };

    let unit = lazy(|| ());
    cx.push("cpu_spawn", &threads, &unit, cpu_spawn, |t| (0, t as u64, json!({})));
    cx.push("cpu_pool_dispatch", &multi, &unit, pool_dispatch, |_| (0, 1, json!({})));

    let sort_fps: Vec<u64> = footprints.iter().copied().filter(|&f| (64 * MIB..=1024 * MIB).contains(&f)).collect();
    let sort_fps = if sort_fps.is_empty() { footprints.clone() } else { sort_fps };

    for &f in &footprints {
        let fp = f as usize;
        let n64 = fp / 8;
        let n32 = fp / 4;
        let lines = fp / LINE;

        // 1-3: sequential bandwidth.
        let seq = lazy(move || SeqState { src: random_u64(n64, salted(seed, 1, f)), dst: buf_u64(n64) });
        let p = move |extra: Value| {
            let mut v = json!({"footprint_bytes": f, "n_items": n64});
            v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            v
        };
        cx.push("seq_read", &threads, &seq, seq_read, |_| (f, n64 as u64, p(json!({"elem_bytes": 8, "unroll": 8}))));
        cx.push("seq_write", &threads, &seq, seq_write, |_| (f, n64 as u64, p(json!({"elem_bytes": 8}))));
        cx.push("seq_copy", &threads, &seq, seq_copy, |_| (2 * f, n64 as u64, p(json!({"elem_bytes": 8}))));

        // 4, 6, 7: random line access, one index per line.
        let rand = lazy(move || RandState {
            data: random_u32(n32, salted(seed, 2, f)),
            dst: buf_u32(n32),
            idx: line_indices(lines, lines, salted(seed, 3, f)),
        });
        let rp = move |bytes_per_item: u64| {
            json!({"footprint_bytes": f, "n_items": lines, "line_bytes": LINE, "elem_bytes": 4, "bytes_per_item": bytes_per_item})
        };
        cx.push("gather_line", &threads, &rand, gather_line, |_| (lines as u64 * 128, lines as u64, rp(128)));
        cx.push("scatter_line", &threads, &rand, scatter_line, |_| (lines as u64 * 128, lines as u64, rp(128)));
        cx.push("rmw_atomic", &threads, &rand, rmw_atomic, |_| (lines as u64 * 256, lines as u64, rp(256)));

        // 5: dependent loads.
        let chase = lazy(move || ChaseState { chase: chase_buffer(fp, salted(seed, 4, f)) });
        cx.push("gather_dep", &threads, &chase, gather_dep, |t| {
            (
                DEP_STEPS as u64 * 128,
                DEP_STEPS as u64,
                json!({"footprint_bytes": f, "n_items": DEP_STEPS, "steps_per_thread": DEP_STEPS / t, "line_bytes": LINE}),
            )
        });

        // 8: hashbrown build + probe.
        let k = fp / 32;
        let n_probes = k.min(MAX_PROBES) & !1;
        let hash = lazy(move || {
            // Present keys are even, absent probe keys odd: hit rate is exact.
            let keys: Vec<u64> = random_u64(k, salted(seed, 5, f)).into_iter().map(|x| x & !1).collect();
            let mut map = HashMap::with_capacity(k);
            for (i, &key) in keys.iter().enumerate() {
                map.insert(key, i as u32);
            }
            let mut rng = Rng::new(salted(seed, 6, f));
            let mut probes: Vec<u64> = (0..n_probes)
                .map(|i| if i % 2 == 0 { keys[rng.below(k)] } else { rng.next() | 1 })
                .collect();
            for i in (1..probes.len()).rev() {
                let j = rng.below(i + 1);
                probes.swap(i, j);
            }
            HashState { keys, probes, map: Mutex::new(map) }
        });
        // Rebuilding 2^27 keys takes ~10 s/run; the 4 GiB table is still built
        // once (in init) for the probe.
        let build_threads: &[usize] = if f <= 1024 * MIB { &[1] } else { &[] };
        cx.push("hashbrown_build", build_threads, &hash, hashbrown_build, |_| {
            (f, k as u64, json!({"footprint_bytes": f, "n_items": k, "n_keys": k, "prealloc": true, "key_bytes": 8, "val_bytes": 4}))
        });
        cx.push("hashbrown_probe", &threads, &hash, hashbrown_probe, |_| {
            (
                n_probes as u64 * 128,
                n_probes as u64,
                json!({"footprint_bytes": f, "n_items": n_probes, "n_keys": k, "hit_rate": 0.5, "key_bytes": 8, "val_bytes": 4}),
            )
        });

        // 9: sort (restricted footprints).
        if sort_fps.contains(&f) {
            let sort = lazy(move || SortState { input: random_u32(n32, salted(seed, 7, f)), work: buf_u32(n32), tmp: buf_u32(n32) });
            cx.push("sort_u32", &threads, &sort, sort_u32, |t| {
                (
                    2 * n32 as u64 * 4,
                    n32 as u64,
                    json!({"footprint_bytes": f, "n_items": n32, "elem_bytes": 4, "parallel": t > 1, "algo": if t > 1 {"chunk sort_unstable + merge-path tree"} else {"sort_unstable"}, "includes_input_copy": true}),
                )
            });

            // 10: merge two sorted halves.
            let merge = lazy(move || {
                let half = n32 / 2;
                let (mut a, mut b) = (random_u32(half, salted(seed, 8, f)), random_u32(half, salted(seed, 9, f)));
                std::thread::scope(|s| {
                    s.spawn(|| a.sort_unstable());
                    s.spawn(|| b.sort_unstable());
                });
                MergeState { a, b, out: buf_u32(n32) }
            });
            cx.push("merge_u32", &threads, &merge, merge_u32, |_| {
                (3 * n32 as u64 * 4, n32 as u64, json!({"footprint_bytes": f, "n_items": n32, "elem_bytes": 4, "split": "merge-path"}))
            });
        }

        // 11: histogram + exclusive scan.
        let hist = lazy(move || HistState {
            keys: random_u32(n32, salted(seed, 10, f)),
            hists: buf_u32(max_threads * HIST_BINS),
            global: buf_u32(HIST_BINS),
        });
        cx.push("hist_scan", &threads, &hist, hist_scan, |_| {
            (n32 as u64 * 4, n32 as u64, json!({"footprint_bytes": f, "n_items": n32, "bins": HIST_BINS, "elem_bytes": 4}))
        });
    }
    cx.out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runner(threads: usize) -> Runner {
        Runner { threads, pool: (threads > 1).then(|| Arc::new(Pool::new(threads))) }
    }

    #[test]
    fn merge_path_partitions_are_consistent() {
        let mut a: Vec<u32> = random_u32(1001, 1).into_iter().map(|x| x % 50).collect();
        let mut b: Vec<u32> = random_u32(777, 2).into_iter().map(|x| x % 50).collect();
        a.sort_unstable();
        b.sort_unstable();
        let mut expect = [a.clone(), b.clone()].concat();
        expect.sort_unstable();
        for parts in [1, 2, 3, 7, 18] {
            let out = buf_u32(a.len() + b.len());
            for p in 0..parts {
                unsafe { merge_part(&a, &b, out.ptr(), p, parts) };
            }
            assert_eq!(unsafe { out.slice(0..out.len()) }, &expect[..], "parts={parts}");
        }
    }

    #[test]
    fn parallel_sort_sorts() {
        for threads in [1, 3, 6] {
            let r = runner(threads);
            let n = 100_003;
            let s = SortState { input: random_u32(n, 42), work: buf_u32(n), tmp: buf_u32(n) };
            let mid = sort_u32(&s, &r);
            let mut expect = s.input.clone();
            expect.sort_unstable();
            assert_eq!(mid, expect[n / 2] as u64);
            // The final buffer is `work` when the merge tree has an even number of levels.
            let levels = (threads as f64).log2().ceil() as usize;
            let fin = if levels % 2 == 0 { &s.work } else { &s.tmp };
            assert_eq!(unsafe { fin.slice(0..n) }, &expect[..], "threads={threads}");
        }
    }

    #[test]
    fn hist_counts_everything() {
        let r = runner(4);
        let n = 1 << 18;
        let s = HistState { keys: random_u32(n, 7), hists: buf_u32(4 * HIST_BINS), global: buf_u32(HIST_BINS) };
        let total = hist_scan(&s, &r) >> 32;
        assert_eq!(total as usize, n);
    }

    #[test]
    fn chase_is_single_cycle() {
        let fp = 1 << 16;
        let c = chase_buffer(fp, 3);
        let lines = fp / LINE;
        let buf = unsafe { c.slice(0..c.len()) };
        let mut p = 0usize;
        for _ in 0..lines {
            p = buf[p] as usize;
        }
        assert_eq!(p, 0);
        let mut seen = vec![false; lines];
        for _ in 0..lines {
            assert!(!seen[p / U32_PER_LINE]);
            seen[p / U32_PER_LINE] = true;
            p = buf[p] as usize;
        }
    }

    #[test]
    fn pool_runs_every_thread() {
        let r = runner(6);
        assert_eq!(r.exec(&|t| t as u64), 15);
        assert_eq!(r.exec(&|t| 1 + t as u64), 21);
    }
}
