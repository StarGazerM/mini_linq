//! GPU primitives as `Work`s (Metal compute). See README for what each measures.
//!
//! Buffers live in a lazily-allocated pool shared by all works so the suite's
//! peak memory is the largest footprint, not the sum. Slot `a` is read-only
//! random data (keys / tables), `b` is scratch (outputs, scatter targets),
//! `sorted` is a sorted array for merge, `idx:<lines>` are precomputed random
//! line numbers.

use crate::measure::Work;
use block::ConcreteBlock;
use metal::objc::rc::autoreleasepool;
use metal::objc::{class, msg_send, sel, sel_impl};
use metal::*;
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub const MIB: u64 = 1 << 20;
pub const GIB: u64 = 1 << 30;
const TG: u64 = 256;
const TILE: u64 = 2048;
const LB_CAP: u32 = 1 << 15;

pub struct GpuCfg {
    /// Bytes of the primary array per primitive.
    pub footprints: Vec<u64>,
    /// Random accesses per gather / scatter / rmw run.
    pub probes: u64,
    pub seed: u64,
}

// ---------------------------------------------------------------------------
// buffer pool

#[derive(Clone, Copy, Debug)]
enum Init {
    Random,
    Zero,
    Sorted,
    Lines(u64),
}

struct Slot {
    len: u64,
    init: Init,
    buf: Option<Buffer>,
}

struct Ctx {
    device: Device,
    queue: CommandQueue,
    pipes: HashMap<&'static str, ComputePipelineState>,
    slots: Mutex<HashMap<String, Slot>>,
    seed: u64,
}

fn xorshift(s: &mut u64) -> u64 {
    let mut x = *s;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *s = x;
    x
}

fn ceil_div(a: u64, b: u64) -> u64 {
    a.div_ceil(b)
}

impl Ctx {
    fn new(seed: u64) -> Self {
        let device = Device::system_default().expect("no Metal device");
        let lib = device
            .new_library_with_file(env!("KERNELS_METALLIB"))
            .expect("load metallib");
        let names = [
            "seq_read",
            "seq_write",
            "seq_copy",
            "gather_line",
            "gather_coop",
            "scatter_line",
            "rmw_atomic",
            "hist_tg",
            "scan_reduce",
            "scan_downsweep",
            "scan_blocksums",
            "lookback_scan",
            "part_hist",
            "part_scatter",
            "merge_path",
            "empty",
        ];
        let mut pipes = HashMap::new();
        for n in names {
            let f = lib.get_function(n, None).expect(n);
            let p = device.new_compute_pipeline_state_with_function(&f).expect(n);
            assert_eq!(p.thread_execution_width(), 32, "{n}: SIMD width != 32");
            assert!(p.max_total_threads_per_threadgroup() >= TG, "{n}: threadgroup too small");
            pipes.insert(n, p);
        }
        let queue = device.new_command_queue();
        Self { device, queue, pipes, slots: Mutex::new(HashMap::new()), seed }
    }

    fn opts() -> MTLResourceOptions {
        MTLResourceOptions::StorageModeShared | MTLResourceOptions::HazardTrackingModeUntracked
    }

    fn pipe(&self, name: &str) -> &ComputePipelineState {
        &self.pipes[name]
    }

    /// Register (or grow) a pool slot. Allocation happens on first `buf`.
    fn reserve(&self, name: &str, len: u64, init: Init) {
        let mut slots = self.slots.lock().unwrap();
        let e = slots.entry(name.to_string()).or_insert(Slot { len: 0, init, buf: None });
        assert!(e.buf.is_none(), "reserve after allocation: {name}");
        e.len = e.len.max(len.max(4096));
    }

    fn buf(&self, name: &str) -> Buffer {
        let mut slots = self.slots.lock().unwrap();
        let slot = slots.get_mut(name).unwrap_or_else(|| panic!("unknown slot {name}"));
        if let Some(b) = &slot.buf {
            return b.clone();
        }
        assert!(
            slot.len <= self.device.max_buffer_length(),
            "slot {name}: {} bytes exceeds maxBufferLength",
            slot.len
        );
        let b = self.device.new_buffer(slot.len, Self::opts());
        let words = unsafe { std::slice::from_raw_parts_mut(b.contents() as *mut u32, (slot.len / 4) as usize) };
        self.fill(words, slot.init);
        slot.buf = Some(b.clone());
        b
    }

    /// Touch every page once (the fill) so timing never sees first-touch faults.
    fn fill(&self, words: &mut [u32], init: Init) {
        let chunk = 1usize << 20;
        let seed = self.seed;
        std::thread::scope(|s| {
            for (ci, w) in words.chunks_mut(chunk).enumerate() {
                s.spawn(move || {
                    let mut st = seed ^ ((ci as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15));
                    match init {
                        Init::Zero => w.fill(0),
                        Init::Random => w.iter_mut().for_each(|x| *x = xorshift(&mut st) as u32),
                        // increments in [0, 8): chunk base = ci * chunk * 8 keeps global order
                        Init::Sorted => {
                            let mut v = (ci * chunk * 8) as u32;
                            for x in w.iter_mut() {
                                v += (xorshift(&mut st) & 7) as u32;
                                *x = v;
                            }
                        }
                        Init::Lines(lines) => w.iter_mut().for_each(|x| *x = (xorshift(&mut st) % lines) as u32),
                    }
                });
            }
        });
    }
}

/// Bind buffers 0..k, then 32-bit constants k..k+m, dispatch `tgs` threadgroups of 256.
fn dispatch(enc: &ComputeCommandEncoderRef, pipe: &ComputePipelineState, bufs: &[(&Buffer, u64)], consts: &[u32], tgs: u64) {
    enc.set_compute_pipeline_state(pipe);
    for (i, &(b, off)) in bufs.iter().enumerate() {
        enc.set_buffer(i as u64, Some(b), off);
    }
    for (j, c) in consts.iter().enumerate() {
        enc.set_bytes((bufs.len() + j) as u64, 4, c as *const u32 as *const _);
    }
    enc.dispatch_thread_groups(MTLSize::new(tgs, 1, 1), MTLSize::new(TG, 1, 1));
}

/// Reduce-then-scan exclusive scan of `n` masked uints: 3 dispatches
/// (block sums, scan of block sums in one threadgroup, downsweep).
fn scan(enc: &ComputeCommandEncoderRef, ctx: &Ctx, input: &Buffer, output: &Buffer, bs: &Buffer, n: u64, mask: u32) {
    let blocks = ceil_div(n, TILE);
    dispatch(enc, ctx.pipe("scan_reduce"), &[(input, 0), (bs, 0)], &[n as u32, mask], blocks);
    dispatch(enc, ctx.pipe("scan_blocksums"), &[(bs, 0)], &[blocks as u32], 1);
    dispatch(enc, ctx.pipe("scan_downsweep"), &[(input, 0), (output, 0), (bs, 0)], &[n as u32, mask], blocks);
}

fn word(b: &BufferRef, i: u64) -> u32 {
    unsafe { std::ptr::read_volatile((b.contents() as *const u32).add(i as usize)) }
}

fn words(b: &BufferRef, n: u64) -> &[u32] {
    unsafe { std::slice::from_raw_parts(b.contents() as *const u32, n as usize) }
}

fn zero(b: &BufferRef, bytes: u64) {
    unsafe { std::ptr::write_bytes(b.contents() as *mut u8, 0, bytes as usize) }
}

// ---------------------------------------------------------------------------
// works

enum Kind {
    SeqRead { n4: u64 },
    SeqWrite { n4: u64 },
    SeqCopy { n4: u64 },
    Gather { probes: u64, lines: u64, coop: bool },
    Scatter { probes: u64, lines: u64 },
    Rmw { probes: u64, lines: u64 },
    HistScan { n: u64, bins: u64, per_tg: u64 },
    Partition { n: u64, ntiles: u64 },
    Merge { n: u64 },
    ScanRts { n: u64 },
    Lookback { n: u64, ntiles: u64 },
    DispatchEmpty { count: u64, encoders: bool },
    CmdBufEmpty,
    SyncRoundtrip { rounds: u64, event: SharedEvent, listener: Option<SharedEventListener> },
}

struct GpuWork {
    ctx: Arc<Ctx>,
    op: String,
    bytes: u64,
    items: u64,
    params: Map<String, Value>,
    kind: Kind,
    /// GPUEndTime - GPUStartTime per run, drained by `params()`.
    gpu_ms: Mutex<Vec<f64>>,
    /// Params discovered at run time (correctness, forward progress).
    extra: Mutex<Map<String, Value>>,
    counter: AtomicU64,
}

fn grid_tgs(n4: u64) -> u64 {
    ceil_div(n4, TG).clamp(1, 4096)
}

impl GpuWork {
    fn set_extra(&self, k: &str, v: Value) {
        self.extra.lock().unwrap().insert(k.into(), v);
    }

    fn first_run(&self) -> bool {
        self.counter.fetch_add(1, Ordering::Relaxed) == 0
    }

    fn res(&self) -> Buffer {
        let r = self.ctx.buf("res");
        zero(&r, 64);
        r
    }

    /// Encode the work into `cb`; returns a closure computing the checksum after completion.
    fn encode(&self, cb: &CommandBufferRef) -> Box<dyn FnOnce() -> u64 + '_> {
        let ctx = &self.ctx;
        match &self.kind {
            Kind::SeqRead { n4 } => {
                let (a, r) = (ctx.buf("a"), self.res());
                let enc = cb.new_compute_command_encoder();
                dispatch(enc, ctx.pipe("seq_read"), &[(&a, 0), (&r, 0)], &[*n4 as u32], grid_tgs(*n4));
                enc.end_encoding();
                Box::new(move || word(&r, 0) as u64)
            }
            Kind::SeqWrite { n4 } => {
                let b = ctx.buf("b");
                let val = self.counter.fetch_add(1, Ordering::Relaxed) as u32;
                let enc = cb.new_compute_command_encoder();
                dispatch(enc, ctx.pipe("seq_write"), &[(&b, 0)], &[*n4 as u32, val], grid_tgs(*n4));
                enc.end_encoding();
                Box::new(move || word(&b, n4 * 4 - 1) as u64)
            }
            Kind::SeqCopy { n4 } => {
                let (a, b) = (ctx.buf("a"), ctx.buf("b"));
                let enc = cb.new_compute_command_encoder();
                dispatch(enc, ctx.pipe("seq_copy"), &[(&a, 0), (&b, 0)], &[*n4 as u32], grid_tgs(*n4));
                enc.end_encoding();
                Box::new(move || word(&b, n4 * 4 - 1) as u64)
            }
            Kind::Gather { probes, lines, coop } => {
                let (a, idx, r) = (ctx.buf("a"), ctx.buf(&format!("idx:{lines}")), self.res());
                let enc = cb.new_compute_command_encoder();
                let pipe = if *coop { "gather_coop" } else { "gather_line" };
                dispatch(enc, ctx.pipe(pipe), &[(&a, 0), (&idx, 0), (&r, 0)], &[*probes as u32], ceil_div(*probes, TG));
                enc.end_encoding();
                Box::new(move || word(&r, 0) as u64)
            }
            Kind::Scatter { probes, lines } => {
                let (b, idx) = (ctx.buf("b"), ctx.buf(&format!("idx:{lines}")));
                let enc = cb.new_compute_command_encoder();
                dispatch(enc, ctx.pipe("scatter_line"), &[(&b, 0), (&idx, 0)], &[*probes as u32], ceil_div(*probes, TG));
                enc.end_encoding();
                let first = word(&idx, 0) as u64 * 32;
                Box::new(move || word(&b, first) as u64)
            }
            Kind::Rmw { probes, lines } => {
                let (b, idx) = (ctx.buf("b"), ctx.buf(&format!("idx:{lines}")));
                let enc = cb.new_compute_command_encoder();
                dispatch(enc, ctx.pipe("rmw_atomic"), &[(&b, 0), (&idx, 0)], &[*probes as u32], ceil_div(*probes, TG));
                enc.end_encoding();
                let first = word(&idx, 0) as u64 * 32;
                Box::new(move || word(&b, first) as u64)
            }
            Kind::HistScan { n, bins, per_tg } => {
                let (a, hist, out, bs) = (ctx.buf("a"), ctx.buf("hist"), ctx.buf("hist_out"), ctx.buf("bs"));
                zero(&hist, bins * 4);
                let enc = cb.new_compute_command_encoder();
                dispatch(enc, ctx.pipe("hist_tg"), &[(&a, 0), (&hist, 0)], &[*n as u32, *bins as u32, *per_tg as u32], ceil_div(*n, *per_tg));
                scan(enc, ctx, &hist, &out, &bs, *bins, u32::MAX);
                enc.end_encoding();
                let (n, bins, first) = (*n, *bins, self.first_run());
                Box::new(move || {
                    let total = word(&out, bins - 1) as u64 + word(&hist, bins - 1) as u64;
                    if first {
                        self.set_extra("correct", json!(total == n));
                    }
                    total
                })
            }
            Kind::Partition { n, ntiles } => {
                let (a, b, counts, offsets, bs) =
                    (ctx.buf("a"), ctx.buf("b"), ctx.buf("counts"), ctx.buf("offsets"), ctx.buf("bs"));
                let len = 256 * ntiles;
                let enc = cb.new_compute_command_encoder();
                let (nn, nt) = (*n as u32, *ntiles as u32);
                dispatch(enc, ctx.pipe("part_hist"), &[(&a, 0), (&counts, 0)], &[nn, nt, 0], *ntiles);
                scan(enc, ctx, &counts, &offsets, &bs, len, u32::MAX);
                dispatch(
                    enc,
                    ctx.pipe("part_scatter"),
                    &[(&a, 0), (&a, n * 4), (&b, 0), (&b, n * 4), (&offsets, 0)],
                    &[nn, nt, 0],
                    *ntiles,
                );
                enc.end_encoding();
                let (n, first) = (*n, self.first_run());
                Box::new(move || {
                    if first {
                        let ok = words(&b, n).windows(2).all(|w| (w[0] & 255) <= (w[1] & 255));
                        self.set_extra("correct", json!(ok));
                    }
                    word(&b, n - 1) as u64
                })
            }
            Kind::Merge { n } => {
                let (s, b) = (ctx.buf("sorted"), ctx.buf("b"));
                let half = n / 2;
                let enc = cb.new_compute_command_encoder();
                dispatch(enc, ctx.pipe("merge_path"), &[(&s, 0), (&s, half * 4), (&b, 0)], &[half as u32, half as u32], ceil_div(*n, TILE));
                enc.end_encoding();
                let (n, first) = (*n, self.first_run());
                Box::new(move || {
                    if first {
                        let out = words(&b, n);
                        let ok = out.windows(2).all(|w| w[0] <= w[1]) && out[n as usize - 1] == word(&s, n - 1);
                        self.set_extra("correct", json!(ok));
                    }
                    word(&b, n - 1) as u64
                })
            }
            Kind::ScanRts { n } => {
                let (a, b, bs) = (ctx.buf("a"), ctx.buf("b"), ctx.buf("bs"));
                let enc = cb.new_compute_command_encoder();
                scan(enc, ctx, &a, &b, &bs, *n, 3);
                enc.end_encoding();
                let (n, first) = (*n, self.first_run());
                Box::new(move || {
                    if first {
                        self.set_extra("correct", json!(self.check_scan(&a, &b, n)));
                    }
                    word(&b, n - 1) as u64
                })
            }
            Kind::Lookback { n, ntiles } => {
                let (a, b, status, r) = (ctx.buf("a"), ctx.buf("b"), ctx.buf("status"), self.res());
                zero(&status, ntiles * 4);
                let enc = cb.new_compute_command_encoder();
                dispatch(
                    enc,
                    ctx.pipe("lookback_scan"),
                    &[(&a, 0), (&b, 0), (&status, 0), (&r, 8), (&r, 4)],
                    &[*n as u32, 3, LB_CAP],
                    *ntiles,
                );
                enc.end_encoding();
                let (n, first) = (*n, self.first_run());
                Box::new(move || {
                    let failed = word(&r, 1) != 0;
                    let prev = self.extra.lock().unwrap().get("forward_progress").and_then(|v| v.as_bool()).unwrap_or(true);
                    self.set_extra("forward_progress", json!(prev && !failed));
                    if first {
                        self.set_extra("correct", json!(!failed && self.check_scan(&a, &b, n)));
                    }
                    word(&b, n - 1) as u64
                })
            }
            Kind::DispatchEmpty { count, encoders } => {
                let pipe = ctx.pipe("empty");
                if *encoders {
                    for _ in 0..*count {
                        let enc = cb.new_compute_command_encoder();
                        enc.set_compute_pipeline_state(pipe);
                        enc.dispatch_thread_groups(MTLSize::new(1, 1, 1), MTLSize::new(1, 1, 1));
                        enc.end_encoding();
                    }
                } else {
                    let enc = cb.new_compute_command_encoder();
                    enc.set_compute_pipeline_state(pipe);
                    for _ in 0..*count {
                        enc.dispatch_thread_groups(MTLSize::new(1, 1, 1), MTLSize::new(1, 1, 1));
                    }
                    enc.end_encoding();
                }
                Box::new(|| 0)
            }
            Kind::CmdBufEmpty => Box::new(|| 0),
            Kind::SyncRoundtrip { .. } => unreachable!("handled in run"),
        }
    }

    /// Exclusive scan check: out[n-1] + (in[n-1] & 3) == sum(in & 3).
    fn check_scan(&self, a: &BufferRef, b: &BufferRef, n: u64) -> bool {
        let total: u64 = words(a, n).iter().map(|&x| (x & 3) as u64).sum();
        word(b, n - 1) as u64 + (word(a, n - 1) & 3) as u64 == total
    }

    fn finish(&self, cb: &CommandBufferRef) {
        cb.commit();
        cb.wait_until_completed();
        let st = cb.status();
        assert!(st == MTLCommandBufferStatus::Completed, "{}: command buffer status {:?}", self.op, st);
        let (s, e): (f64, f64) = unsafe { (msg_send![cb, GPUStartTime], msg_send![cb, GPUEndTime]) };
        self.gpu_ms.lock().unwrap().push((e - s) * 1e3);
    }

    fn run_sync(&self, rounds: u64, event: &SharedEvent, listener: Option<&SharedEventListener>) -> u64 {
        // event values must increase monotonically across runs: each run owns 2*rounds+2 values
        let base = self.counter.fetch_add(2 * rounds + 2, Ordering::Relaxed);
        autoreleasepool(|| {
            let cb = self.ctx.queue.new_command_buffer();
            for r in 0..rounds {
                cb.encode_signal_event(event, base + 2 * r + 1);
                cb.encode_wait_for_event(event, base + 2 * r + 2);
            }
            match listener {
                Some(l) => {
                    // CPU side: `rounds` notifications registered up front, each answers v with v+1.
                    for r in 0..rounds {
                        let target = base + 2 * r + 2;
                        let block = ConcreteBlock::new(move |ev: &SharedEventRef, _v: u64| ev.set_signaled_value(target));
                        event.notify(l, base + 2 * r + 1, block.copy());
                    }
                    self.finish(cb);
                }
                None => {
                    // CPU side: spin on signaledValue, answer v with v+1.
                    std::thread::scope(|s| {
                        s.spawn(|| {
                            for r in 0..rounds {
                                let want = base + 2 * r + 1;
                                while event.signaled_value() < want {
                                    std::hint::spin_loop();
                                }
                                event.set_signaled_value(want + 1);
                            }
                        });
                        self.finish(cb);
                    });
                }
            }
            event.signaled_value()
        })
    }
}

impl Work for GpuWork {
    fn name(&self) -> String {
        self.op.clone()
    }
    fn processor(&self) -> &'static str {
        "gpu"
    }
    fn threads(&self) -> usize {
        1
    }
    fn bytes(&self) -> u64 {
        self.bytes
    }
    fn items(&self) -> u64 {
        self.items
    }
    fn params(&self) -> Value {
        let mut p = self.params.clone();
        for (k, v) in self.extra.lock().unwrap().iter() {
            p.insert(k.clone(), v.clone());
        }
        let mut t: Vec<f64> = std::mem::take(&mut *self.gpu_ms.lock().unwrap());
        if !t.is_empty() {
            t.sort_by(|a, b| a.partial_cmp(b).unwrap());
            p.insert("gpu_ms".into(), json!(t[t.len() / 2]));
        }
        Value::Object(p)
    }
    fn run(&self) -> u64 {
        if let Kind::SyncRoundtrip { rounds, event, listener } = &self.kind {
            return self.run_sync(*rounds, event, listener.as_ref());
        }
        autoreleasepool(|| {
            let cb = self.ctx.queue.new_command_buffer();
            let checksum = self.encode(cb);
            self.finish(cb);
            checksum()
        })
    }
}

// ---------------------------------------------------------------------------
// suite

struct Builder {
    ctx: Arc<Ctx>,
    works: Vec<Box<dyn Work>>,
}

impl Builder {
    fn add(&mut self, op: &str, bytes: u64, items: u64, params: Value, kind: Kind) {
        let params = match params {
            Value::Object(m) => m,
            _ => Map::new(),
        };
        self.works.push(Box::new(GpuWork {
            ctx: self.ctx.clone(),
            op: op.into(),
            bytes,
            items,
            params,
            kind,
            gpu_ms: Mutex::new(Vec::new()),
            extra: Mutex::new(Map::new()),
            counter: AtomicU64::new(0),
        }));
    }
}

/// Keys per histogram threadgroup: enough to amortise the per-bin flush, and
/// below 65536 when the kernel packs two 16-bit counters per word (bins > 8192).
fn hist_per_tg(n: u64, bins: u64) -> u64 {
    let cap = if bins > 8192 { 65536 - TG } else { 256 * 1024 };
    (n / 512).clamp(16 * 1024, cap) / TG * TG
}

pub fn gpu_works(cfg: &GpuCfg) -> Vec<Box<dyn Work>> {
    let ctx = Arc::new(Ctx::new(cfg.seed));
    let mut b = Builder { ctx: ctx.clone(), works: Vec::new() };
    let fps: Vec<u64> = cfg.footprints.iter().map(|f| f / 4096 * 4096).filter(|&f| f >= 4096).collect();
    let big_cap = GIB; // partition / merge / scan footprints are capped (2-4 arrays resident)
    let probes = cfg.probes;

    ctx.reserve("res", 4096, Init::Zero);
    ctx.reserve("hist", 65536 * 4, Init::Zero);
    ctx.reserve("hist_out", 65536 * 4, Init::Zero);
    ctx.reserve("bs", 4096, Init::Zero);
    ctx.reserve(&format!("idx:{}", 1024), probes * 4, Init::Lines(1024));

    for &fp in &fps {
        let n = fp / 4;
        let n4 = fp / 16;
        let lines = fp / 128;
        let p = json!({ "footprint_bytes": fp });
        ctx.reserve("a", fp, Init::Random);
        ctx.reserve("b", fp, Init::Zero);
        ctx.reserve(&format!("idx:{lines}"), probes * 4, Init::Lines(lines));

        b.add("seq_read", fp, n, p.clone(), Kind::SeqRead { n4 });
        b.add("seq_write", fp, n, p.clone(), Kind::SeqWrite { n4 });
        b.add("seq_copy", 2 * fp, n, p.clone(), Kind::SeqCopy { n4 });
        b.add("gather_line", probes * 128, probes, json!({"footprint_bytes": fp, "probes": probes}), Kind::Gather { probes, lines, coop: false });
        b.add("gather_coop", probes * 128, probes, json!({"footprint_bytes": fp, "probes": probes}), Kind::Gather { probes, lines, coop: true });
        b.add("scatter_line", probes * 128, probes, json!({"footprint_bytes": fp, "probes": probes}), Kind::Scatter { probes, lines });
        b.add("rmw_atomic", probes * 256, probes, json!({"footprint_bytes": fp, "probes": probes}), Kind::Rmw { probes, lines });

        for bins in [256u64, 65536] {
            let per_tg = hist_per_tg(n, bins);
            b.add(
                "hist_scan",
                n * 4 + bins * 16,
                n,
                json!({"footprint_bytes": fp, "bins": bins, "keys_per_tg": per_tg}),
                Kind::HistScan { n, bins, per_tg },
            );
        }

        if fp <= big_cap {
            let ntiles = ceil_div(n, TILE);
            ctx.reserve("a", 2 * fp, Init::Random);
            ctx.reserve("b", 2 * fp, Init::Zero);
            ctx.reserve("counts", 256 * ntiles * 4, Init::Zero);
            ctx.reserve("offsets", 256 * ntiles * 4, Init::Zero);
            ctx.reserve("bs", ceil_div(256 * ntiles, TILE) * 4, Init::Zero);
            b.add(
                "partition_pass",
                16 * n + 20 * 256 * ntiles,
                n,
                json!({"footprint_bytes": fp, "tiles": ntiles, "radix_bits": 8, "stable": false}),
                Kind::Partition { n, ntiles },
            );

            ctx.reserve("sorted", fp, Init::Sorted);
            b.add("merge_path", 12 * n, n, p.clone(), Kind::Merge { n });

            ctx.reserve("bs", ntiles * 4, Init::Zero);
            b.add("scan_rts", 12 * n, n, p.clone(), Kind::ScanRts { n });
            ctx.reserve("status", ntiles * 4, Init::Zero);
            b.add(
                "lookback_scan",
                8 * n,
                n,
                json!({"footprint_bytes": fp, "tiles": ntiles, "watchdog_cap": LB_CAP}),
                Kind::Lookback { n, ntiles },
            );
        }
    }

    b.add("rmw_atomic_contended", probes * 256, probes, json!({"footprint_bytes": 1024 * 128, "probes": probes, "lines": 1024}), Kind::Rmw { probes, lines: 1024 });

    b.add("cmdbuf_empty", 0, 1, json!({}), Kind::CmdBufEmpty);
    b.add("dispatch_empty", 0, 1, json!({"dispatches": 1}), Kind::DispatchEmpty { count: 1, encoders: false });
    b.add("dispatch_empty", 0, 100, json!({"dispatches": 100, "encoders": 1}), Kind::DispatchEmpty { count: 100, encoders: false });
    b.add("dispatch_empty_enc", 0, 100, json!({"dispatches": 100, "encoders": 100}), Kind::DispatchEmpty { count: 100, encoders: true });

    let rounds = 100;
    b.add(
        "sync_roundtrip",
        0,
        rounds,
        json!({"rounds": rounds, "cpu_side": "spin on signaledValue"}),
        Kind::SyncRoundtrip { rounds, event: ctx.device.new_shared_event(), listener: None },
    );
    let listener: SharedEventListener = unsafe { msg_send![class!(MTLSharedEventListener), new] };
    b.add(
        "sync_roundtrip_notify",
        0,
        rounds,
        json!({"rounds": rounds, "cpu_side": "MTLSharedEventListener notify block"}),
        Kind::SyncRoundtrip { rounds, event: ctx.device.new_shared_event(), listener: Some(listener) },
    );

    b.works
}
