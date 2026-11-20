//! Hand-written rule holes (feature `manual-joins`), called by the round
//! printer in place of generated holes (`print_rounds(manual(...))`).
//!
//! `VPT_LoadArray@3`, the version driven by new `arrayindexpointsto` rows:
//!
//! ```text
//! varpointsto(heap, to) :- reachable(m), loadarrayindex(base, to, m),
//!     varpointsto(baseheap, base), Δarrayindexpointsto(baseheap, heap),
//!     var_type(to, type), heapallocation_type(baseheap, bht),
//!     componenttype(bht, bct), supertypeof(type, bct).
//! ```
//!
//! The generated plan expands `varpointsto(baseheap, ·)` once per delta row
//! and then probes `loadarrayindex` by base: on scala that is 23.2 G rows over
//! all rounds, of which 175 M pass the probe. Nothing between the expansion
//! and the output reads `heap`, and the delta's 52.8 M rows have only 0.64 M
//! distinct `baseheap`s. So the join is factorized by `baseheap`:
//! 1. `delta_groups`: one descriptor per distinct delta `baseheap` (a group
//!    of heaps) and component type of its allocation type;
//! 2. `group_spans`: each group's `varpointsto(baseheap, ·)` range per run;
//!    the host prefix-sums them (360 M rows on scala instead of 23.2 G);
//! 3. `scan_points`: one thread per spanned row, balanced over the grid,
//!    probes the small `loadarrayindex` by base, then `reachable`,
//!    `var_type` and `supertypeof`, and appends one pair (group, `to`) with
//!    the group's output rows reserved;
//! 4. `expand_pairs`: one warp per pair writes every heap of its group.
//! The candidate multiset is the generated plan's.
use cuda_device::device;
use gpu_datalog_runtime::radix_run::device::{decode_key, encode_prefix, interval, run};
use gpu_datalog_runtime::{Buffer, CandidateArena, CandidateBatch, Packing, RadixLayout, RadixLayoutView, Scalar, Word};

/// Inclusive key bounds of the one-field prefix `a`.
#[device]
unsafe fn prefix1<W: Word>(view: RadixLayoutView<W, u64>, a: i64) -> (W, W, bool) {
    let values = [a];
    unsafe { encode_prefix(view, values.as_ptr(), 1) }
}

/// Inclusive key bounds of the two-field prefix `(a, b)`.
#[device]
unsafe fn prefix2<W: Word>(view: RadixLayoutView<W, u64>, a: i64, b: i64) -> (W, W, bool) {
    let values = [a, b];
    unsafe { encode_prefix(view, values.as_ptr(), 2) }
}

/// Whether any run holds a key within the bounds.
#[device]
unsafe fn any<W: Word>(view: RadixLayoutView<W, u64>, (low, high, fits): (W, W, bool)) -> bool {
    if !fits {
        return false;
    }
    let mut index = 0;
    while index < view.keys.run_count {
        let (first, last) = unsafe { interval(run(view.keys, index), low, high) };
        if first < last {
            return true;
        }
        index += 1;
    }
    false
}

/// Run and slot of a position in the concatenation of all runs' keys.
#[device]
unsafe fn locate<W: Word>(view: RadixLayoutView<W, u64>, mut position: u64) -> (u32, u64) {
    let mut index = 0;
    while index < view.keys.run_count {
        let length = unsafe { run(view.keys, index) }.keys_len;
        if position < length {
            break;
        }
        position -= length;
        index += 1;
    }
    (index, position)
}

/// Keys in all runs.
#[device]
unsafe fn keys<W: Word>(view: RadixLayoutView<W, u64>) -> u64 {
    let mut total = 0;
    let mut index = 0;
    while index < view.keys.run_count {
        total += unsafe { run(view.keys, index) }.keys_len;
        index += 1;
    }
    total
}

/// Words of one group: delta run, first and end slot, `baseheap`, and the
/// component type of its allocation type.
const GROUP_WORDS: usize = 5;

/// Words of one output pair: the group's first delta slot, the first output
/// row, `(heaps << 32) | to`, and the group's delta run.
const PAIR_WORDS: usize = 4;

/// Record that every heap of the delta group `[first, last)` of run
/// `group_run` pairs with `to`: reserve its output rows (`counts[0]`) and
/// append the pair (`counts[1]`). `expand_pairs` writes the rows.
#[device]
unsafe fn pair(group: (u32, u64, u64), to: i64, pairs: *mut u64, pair_capacity: u64, counts: *mut u64) {
    let (group_run, first, last) = group;
    let rows = last - first;
    let base = unsafe { cuda_device::atomic::DeviceAtomicU64::from_ptr(counts) }
        .fetch_add(rows, cuda_device::atomic::AtomicOrdering::Relaxed);
    let at = unsafe { cuda_device::atomic::DeviceAtomicU64::from_ptr(counts.add(1)) }
        .fetch_add(1, cuda_device::atomic::AtomicOrdering::Relaxed);
    if at < pair_capacity {
        let pair = unsafe { pairs.add(at as usize * PAIR_WORDS) };
        unsafe {
            *pair = first;
            *pair.add(1) = base;
            *pair.add(2) = (rows << 32) | (to as u64 & 0xffff_ffff);
            *pair.add(3) = group_run as u64;
        }
    }
}

/// One `loadarrayindex(base, to, m)` row at slot `slot` of run `load_run`:
/// reachable `m`, then each type of `to` that is a supertype of `component`.
#[device]
#[allow(clippy::too_many_arguments)]
unsafe fn load_row<WR: Word, WS: Word, WT: Word, WL: Word>(
    reachable: RadixLayoutView<WR, u64>,
    supertype: RadixLayoutView<WS, u64>,
    var_type: RadixLayoutView<WT, u64>,
    load: RadixLayoutView<WL, u64>,
    load_run: u32,
    slot: u64,
    component: i64,
    group: (u32, u64, u64),
    pairs: *mut u64,
    pair_capacity: u64,
    counts: *mut u64,
) {
    let to = unsafe { decode_key(load, load_run, slot, 1) };
    let method = unsafe { decode_key(load, load_run, slot, 2) };
    if !unsafe { any(reachable, prefix1(reachable, method)) } {
        return;
    }
    let (low, high, fits) = unsafe { prefix1(var_type, to) };
    if !fits {
        return;
    }
    let mut index = 0;
    while index < var_type.keys.run_count {
        let (first, last) = unsafe { interval(run(var_type.keys, index), low, high) };
        let mut position = first;
        while position < last {
            let ty = unsafe { decode_key(var_type, index, position, 1) };
            if unsafe { any(supertype, prefix2(supertype, ty, component)) } {
                unsafe { pair(group, to, pairs, pair_capacity, counts) };
            }
            position += 1;
        }
        index += 1;
    }
}

#[cuda_device::cuda_module]
pub mod kernels {
    use super::*;
    use cuda_device::{kernel, launch_bounds, launch_contract};

    /// Step 1: a thread per delta position; the first position of each
    /// `baseheap` writes one group per component type of its allocation type.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn delta_groups<WD: Copy + Word, WH: Copy + Word, WC: Copy + Word>(
        delta: RadixLayoutView<WD, u64>,
        hat: RadixLayoutView<WH, u64>,
        component: RadixLayoutView<WC, u64>,
        groups: *mut u64,
        capacity: u64,
        count: *mut u64,
    ) {
        unsafe {
            let total = keys(delta);
            let stride = cuda_device::thread::gridDim_x() as u64 * 256;
            let mut position = cuda_device::thread::blockIdx_x() as u64 * 256 + cuda_device::thread::threadIdx_x() as u64;
            while position < total {
                let (group_run, slot) = locate(delta, position);
                position += stride;
                let heap_base = decode_key(delta, group_run, slot, 0);
                if slot != 0 && decode_key(delta, group_run, slot - 1, 0) == heap_base {
                    continue;
                }
                let (low, high, _) = prefix1(delta, heap_base);
                let (first, last) = interval(run(delta.keys, group_run), low, high);
                let (hat_low, hat_high, hat_fits) = prefix1(hat, heap_base);
                if !hat_fits {
                    continue;
                }
                let mut hat_run = 0;
                while hat_run < hat.keys.run_count {
                    let (hat_first, hat_last) = interval(run(hat.keys, hat_run), hat_low, hat_high);
                    let mut hat_slot = hat_first;
                    while hat_slot < hat_last {
                        let heap_type = decode_key(hat, hat_run, hat_slot, 1);
                        hat_slot += 1;
                        let (component_low, component_high, component_fits) = prefix1(component, heap_type);
                        if !component_fits {
                            continue;
                        }
                        let mut component_run = 0;
                        while component_run < component.keys.run_count {
                            let (component_first, component_last) =
                                interval(run(component.keys, component_run), component_low, component_high);
                            let mut component_slot = component_first;
                            while component_slot < component_last {
                                let element = decode_key(component, component_run, component_slot, 1);
                                component_slot += 1;
                                let at = cuda_device::atomic::DeviceAtomicU64::from_ptr(count)
                                    .fetch_add(1, cuda_device::atomic::AtomicOrdering::Relaxed);
                                if at < capacity {
                                    let group = groups.add(at as usize * GROUP_WORDS);
                                    *group = group_run as u64;
                                    *group.add(1) = first;
                                    *group.add(2) = last;
                                    *group.add(3) = heap_base as u64;
                                    *group.add(4) = element as u64;
                                }
                            }
                            component_run += 1;
                        }
                    }
                    hat_run += 1;
                }
            }
        }
    }

    /// Step 2: a thread per (group, `varpointsto` run) writes the group's
    /// `varpointsto(baseheap, ·)` slot range in that run as (first, length).
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn group_spans<WV: Copy + Word>(points: RadixLayoutView<WV, u64>, groups: *const u64, count: u64, spans: *mut u64) {
        unsafe {
            let runs = points.keys.run_count as u64;
            let stride = cuda_device::thread::gridDim_x() as u64 * 256;
            let mut index = cuda_device::thread::blockIdx_x() as u64 * 256 + cuda_device::thread::threadIdx_x() as u64;
            while index < count * runs {
                let heap_base = *groups.add((index / runs) as usize * GROUP_WORDS + 3) as i64;
                let (low, high, fits) = prefix1(points, heap_base);
                let (first, last) = if fits {
                    interval(run(points.keys, (index % runs) as u32), low, high)
                } else {
                    (0, 0)
                };
                *spans.add(index as usize * 2) = first;
                *spans.add(index as usize * 2 + 1) = last - first;
                index += stride;
            }
        }
    }

    /// Step 3: a thread per spanned `varpointsto` row of any group.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    #[allow(clippy::too_many_arguments)]
    pub fn scan_points<WR: Copy + Word, WS: Copy + Word, WT: Copy + Word, WL: Copy + Word, WV: Copy + Word>(
        reachable: RadixLayoutView<WR, u64>,
        supertype: RadixLayoutView<WS, u64>,
        var_type: RadixLayoutView<WT, u64>,
        load: RadixLayoutView<WL, u64>,
        points: RadixLayoutView<WV, u64>,
        groups: *const u64,
        spans: *const u64,
        offsets: *const u64,
        segments: u64,
        rows: u64,
        pairs: *mut u64,
        pair_capacity: u64,
        counts: *mut u64,
    ) {
        unsafe {
            let runs = points.keys.run_count as u64;
            let stride = cuda_device::thread::gridDim_x() as u64 * 256;
            let mut row = cuda_device::thread::blockIdx_x() as u64 * 256 + cuda_device::thread::threadIdx_x() as u64;
            while row < rows {
                // The last segment starting at or before `row` holds it.
                let (mut low, mut high) = (0u64, segments);
                while low + 1 < high {
                    let middle = (low + high) / 2;
                    if *offsets.add(middle as usize) <= row {
                        low = middle;
                    } else {
                        high = middle;
                    }
                }
                let segment = low;
                let group = groups.add((segment / runs) as usize * GROUP_WORDS);
                let points_run = (segment % runs) as u32;
                let slot = *spans.add(segment as usize * 2) + (row - *offsets.add(segment as usize));
                row += stride;
                let base = decode_key(points, points_run, slot, 1);
                let (load_low, load_high, load_fits) = prefix1(load, base);
                if !load_fits {
                    continue;
                }
                let delta_group = (*group as u32, *group.add(1), *group.add(2));
                let element = *group.add(4) as i64;
                let mut load_run = 0;
                while load_run < load.keys.run_count {
                    let (load_first, load_last) = interval(run(load.keys, load_run), load_low, load_high);
                    let mut load_slot = load_first;
                    while load_slot < load_last {
                        load_row(
                            reachable, supertype, var_type, load, load_run, load_slot, element, delta_group, pairs,
                            pair_capacity, counts,
                        );
                        load_slot += 1;
                    }
                    load_run += 1;
                }
            }
        }
    }

    /// Step 4: one warp per pair writes `(heap, to)` for every heap of its
    /// group into the pair's reserved rows.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn expand_pairs<WD: Copy + Word>(delta: RadixLayoutView<WD, u64>, pairs: *const u64, count: u64, out: *mut u64) {
        unsafe {
            let lane = (cuda_device::thread::threadIdx_x() % 32) as u64;
            let warps = cuda_device::thread::gridDim_x() as u64 * 8;
            let mut index = cuda_device::thread::blockIdx_x() as u64 * 8 + (cuda_device::thread::threadIdx_x() / 32) as u64;
            while index < count {
                let pair = pairs.add(index as usize * PAIR_WORDS);
                let (first, base, packed, group_run) = (*pair, *pair.add(1), *pair.add(2), *pair.add(3) as u32);
                let (rows, to) = (packed >> 32, packed & 0xffff_ffff);
                let mut row = lane;
                while row < rows {
                    let heap = decode_key(delta, group_run, first + row, 1);
                    *out.add((base + row) as usize) = ((heap as u64) << 32) | to;
                    row += 32;
                }
                index += warps;
            }
        }
    }
}

/// What each input of the replaced `VPT_LoadArray@3` hole must read, in order.
const READS: [&str; 8] = [
    "delta arrayindexpointsto",
    "heapallocation_type[0, 1]",
    "reachable[0]",
    "componenttype[0, 1]",
    "supertypeof[0, 1]",
    "var_type[0, 1]",
    "loadarrayindex[0, 1, 2]",
    "varpointsto[0, 1]",
];

thread_local! {
    static MODULE: std::cell::RefCell<Option<(std::sync::Arc<cuda_core::CudaContext>, kernels::LoadedModule)>> =
        const { std::cell::RefCell::new(None) };
    /// The previous call's pair count: the next call's pair capacity.
    static PAIRS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Blocks for `work` threads, at most the kernel's resident blocks.
fn grid(function: &cuda_core::CudaFunction, work: usize) -> gpu_datalog_runtime::Result<u32> {
    let resident = cq_gpu_runtime::launch_grid_for_kernel(function, usize::MAX, 256, 0)?;
    Ok((work.div_ceil(256) as u32).clamp(1, resident))
}

/// The `VPT_LoadArray@3` hole: see the module documentation.
#[allow(clippy::too_many_arguments)]
pub fn vpt_load_array_delta_index<KD, WD, KH, WH, KR, WR, KC, WC, KS, WS, KT, WT, KL, WL, KV, WV>(
    arena: &mut CandidateArena,
    reads: &[&str],
    delta: &RadixLayout<KD, (), WD, u64, ()>,
    hat: &RadixLayout<KH, (), WH, u64, ()>,
    reachable: &RadixLayout<KR, (), WR, u64, ()>,
    component: &RadixLayout<KC, (), WC, u64, ()>,
    supertype: &RadixLayout<KS, (), WS, u64, ()>,
    var_type: &RadixLayout<KT, (), WT, u64, ()>,
    load: &RadixLayout<KL, (), WL, u64, ()>,
    points: &RadixLayout<KV, (), WV, u64, ()>,
) -> CandidateBatch
where
    WD: Copy + Word + 'static,
    WH: Copy + Word + 'static,
    WR: Copy + Word + 'static,
    WC: Copy + Word + 'static,
    WS: Copy + Word + 'static,
    WT: Copy + Word + 'static,
    WL: Copy + Word + 'static,
    WV: Copy + Word + 'static,
{
    use cuda_core::LaunchConfig1D;
    assert_eq!(reads, &READS[..], "VPT_LoadArray@3 inputs differ from the hand-written kernel's");
    (|| -> gpu_datalog_runtime::Result<CandidateBatch> {
        let pool = arena.pool().clone();
        let stream = pool.stream();
        let packing = Packing::with_columns(&[<u32 as Scalar>::COLUMN, <u32 as Scalar>::COLUMN])?;
        let owners = vec![
            delta.owner(), hat.owner(), reachable.owner(), component.owner(),
            supertype.owner(), var_type.owner(), load.owner(), points.owner(),
        ];
        MODULE.with(|cache| -> gpu_datalog_runtime::Result<CandidateBatch> {
            let mut cache = cache.borrow_mut();
            if cache.as_ref().is_none_or(|(context, _)| !std::sync::Arc::ptr_eq(context, pool.context())) {
                *cache = Some((pool.context().clone(), unsafe { kernels::load(pool.context())? }));
            }
            let (_, module) = cache.as_ref().expect("loaded module");
            let probe = |blocks| LaunchConfig1D::new(blocks, 256, 0);

            // 1. Groups: at most one per delta row and component type.
            let mut capacity = delta.len().max(1);
            let (groups, group_count) = loop {
                // SAFETY: step 1 writes every group below the count it reports.
                let groups = unsafe { Buffer::<u64>::uninitialized(pool.clone(), capacity * GROUP_WORDS)? };
                let count = Buffer::<u64>::zeroed(pool.clone(), 1)?;
                let blocks = grid(module.prepare_delta_groups::<WD, WH, WC>(probe(1))?.function(), delta.len())?;
                module.delta_groups::<WD, WH, WC>(
                    stream, &module.prepare_delta_groups::<WD, WH, WC>(probe(blocks))?,
                    delta.view_on(&pool)?, hat.view_on(&pool)?, component.view_on(&pool)?,
                    groups.ptr(), capacity as u64, count.ptr(),
                )?;
                let found = count.to_host()?[0] as usize;
                gpu_datalog_runtime::record_readback();
                if found <= capacity {
                    break (groups, found);
                }
                capacity = found;
            };

            // 2. Spans of `varpointsto(baseheap, ·)` per group and run, prefix-summed on the host.
            let runs = points.run_count();
            let segments = group_count * runs;
            let (spans, offsets, rows) = if segments == 0 {
                (Buffer::<u64>::zeroed(pool.clone(), 2)?, Buffer::<u64>::zeroed(pool.clone(), 1)?, 0)
            } else {
                // SAFETY: step 2 writes both words of every segment.
                let spans = unsafe { Buffer::<u64>::uninitialized(pool.clone(), segments * 2)? };
                let blocks = grid(module.prepare_group_spans::<WV>(probe(1))?.function(), segments)?;
                module.group_spans::<WV>(
                    stream, &module.prepare_group_spans::<WV>(probe(blocks))?,
                    points.view_on(&pool)?, groups.ptr(), group_count as u64, spans.ptr(),
                )?;
                let host = spans.to_host()?;
                gpu_datalog_runtime::record_readback();
                let mut offsets = Vec::with_capacity(segments + 1);
                let mut total = 0u64;
                for segment in 0..segments {
                    offsets.push(total);
                    total += host[segment * 2 + 1];
                }
                offsets.push(total);
                (spans, Buffer::from_host(pool.clone(), &offsets)?, total as usize)
            };

            // 3. Pairs from every spanned row.
            let mut pair_capacity = (PAIRS.get() * 2).max(1 << 16);
            let (pairs, candidates, pair_count) = loop {
                // SAFETY: step 3 writes every pair below the count it reports.
                let pairs = unsafe { Buffer::<u64>::uninitialized(pool.clone(), pair_capacity * PAIR_WORDS)? };
                let counts = Buffer::<u64>::zeroed(pool.clone(), 2)?;
                if rows != 0 {
                    let blocks = grid(module.prepare_scan_points::<WR, WS, WT, WL, WV>(probe(1))?.function(), rows)?;
                    module.scan_points::<WR, WS, WT, WL, WV>(
                        stream, &module.prepare_scan_points::<WR, WS, WT, WL, WV>(probe(blocks))?,
                        reachable.view_on(&pool)?, supertype.view_on(&pool)?, var_type.view_on(&pool)?,
                        load.view_on(&pool)?, points.view_on(&pool)?,
                        groups.ptr(), spans.ptr(), offsets.ptr(), segments as u64, rows as u64,
                        pairs.ptr(), pair_capacity as u64, counts.ptr(),
                    )?;
                }
                let host = counts.to_host()?;
                gpu_datalog_runtime::record_readback();
                let (candidates, pair_count) = (host[0] as usize, host[1] as usize);
                if pair_count <= pair_capacity {
                    break (pairs, candidates, pair_count);
                }
                pair_capacity = pair_count;
            };
            PAIRS.set(pair_count);
            if gpu_datalog_runtime::trace_kernels() {
                eprintln!("gpu_manual_hole\trule=VPT_LoadArray@3\tdelta={}\tgroups={group_count}\tscanned={rows}\tpairs={pair_count}\tcandidates={candidates}", delta.len());
            }
            if candidates > u32::MAX as usize {
                return Err("candidate rows exceed u32 positions".into());
            }

            // 4. Rows.
            let lease = arena.reserve(packing.clone(), candidates)?;
            if pair_count != 0 {
                let blocks = grid(module.prepare_expand_pairs::<WD>(probe(1))?.function(), pair_count * 32)?;
                module.expand_pairs::<WD>(
                    stream, &module.prepare_expand_pairs::<WD>(probe(blocks))?,
                    delta.view_on(&pool)?, pairs.ptr(), pair_count as u64, lease.buffer().ptr(),
                )?;
            }
            arena.record_count(candidates);
            unsafe { lease.finish(candidates, owners) }
        })
    })()
    .expect("hand-written VPT_LoadArray@3 failed")
}
