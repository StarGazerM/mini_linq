//! Hand-written runtime oracle/benchmark; no generated query kernel shapes.
//! Build package `gpu-device-columns-oracle`, binary `leveled-index`, with oxide;
//! run `leveled-index --oracle` or `--bench`. Enable `cq-gpu-runtime/nvtx` and
//! profile the four leveled/bench/* ranges to count launches.
use cq_gpu_runtime::{
    Buffer, Codec, DeviceColumns, Dictionary, FrozenDeviceIndex, LeveledDeviceIndex,
    LeveledMergeReason, LeveledRunsView, MAX_LEVELS, Result, RunView, Runtime,
};
use cuda_core::LaunchConfig1D;
use std::{ffi::CStr, time::Instant};

type Row = (i64, i64, i64);
const A: &[&[usize]] = &[&[0], &[1], &[2]];
const B: &[&[usize]] = &[&[1], &[0], &[2]];
const CHAINS: &[&[&[usize]]] = &[A, B];
const KEYS: usize = 256;

fn config(n: usize) -> LaunchConfig1D {
    LaunchConfig1D::new(n.div_ceil(256).max(1) as u32, 256, 0)
}
fn rows(n: usize, offset: usize, zipf: bool, seed: &mut u64) -> Vec<Row> {
    let harmonic: f64 = (1..=KEYS).map(|k| 1.0 / k as f64).sum();
    let mut sum = 0.0;
    let cdf: Vec<_> = (1..=KEYS)
        .map(|k| {
            sum += 1.0 / k as f64 / harmonic;
            sum
        })
        .collect();
    (0..n)
        .map(|i| {
            *seed ^= *seed << 13;
            *seed ^= *seed >> 7;
            *seed ^= *seed << 17;
            let first = if zipf {
                let u = (*seed >> 11) as f64 / (1u64 << 53) as f64;
                cdf.partition_point(|&p| p < u).min(KEYS - 1)
            } else {
                (*seed as usize) % KEYS
            };
            (
                first as i64,
                ((*seed >> 32) % 64) as i64,
                (offset + i) as i64,
            )
        })
        .collect()
}
fn upload(
    runtime: &Runtime,
    data: &[Row],
    dict: &mut Dictionary<'static>,
) -> Result<DeviceColumns<'static, Row>> {
    DeviceColumns::from_rows(runtime.pool().clone(), data, dict, &[Codec::Plain; 3])
}
fn one_run(run: RunView) -> LeveledRunsView {
    let mut runs = [run; MAX_LEVELS];
    runs[0] = run;
    LeveledRunsView { runs, count: 1 }
}
struct PendingDump {
    rows: Buffer<i64>,
    ranges: Buffer<u64>,
    view: LeveledRunsView,
    keys: usize,
}
fn enqueue_dump(
    runtime: &Runtime,
    kernels: &oracle::LoadedModule,
    view: LeveledRunsView,
    keys: usize,
    snapshot: Option<*const LeveledRunsView>,
) -> Result<PendingDump> {
    let n = view.runs[..view.count as usize]
        .iter()
        .map(|r| r.len as usize)
        .sum::<usize>();
    let output = unsafe { Buffer::uninitialized(runtime.pool().clone(), n * 3)? };
    let ranges =
        unsafe { Buffer::uninitialized(runtime.pool().clone(), keys * view.count as usize * 2)? };
    if n != 0 {
        if let Some(snapshot) = snapshot {
            let launch = kernels.prepare_dump_ptr(config(n))?;
            unsafe {
                kernels.dump_ptr(runtime.stream(), &launch, snapshot, output.ptr())?;
            }
        } else {
            let launch = kernels.prepare_dump(config(n))?;
            unsafe {
                kernels.dump(runtime.stream(), &launch, view, output.ptr())?;
            }
        }
    }
    if view.count != 0 {
        let launch = kernels.prepare_ranges(config(keys * view.count as usize))?;
        unsafe {
            kernels.ranges(runtime.stream(), &launch, view, keys, ranges.ptr())?;
        }
    }
    Ok(PendingDump {
        rows: output,
        ranges,
        view,
        keys,
    })
}
fn download(pending: &PendingDump) -> Result<Vec<Row>> {
    Ok(pending
        .rows
        .to_host()?
        .chunks_exact(3)
        .map(|r| (r[0], r[1], r[2]))
        .collect())
}
fn check_dump(pending: &PendingDump, data: &[Row], reference: &[Row], first: usize) -> Result<()> {
    let ranges = pending.ranges.to_host()?;
    let key = |row: &Row| if first == 0 { row.0 } else { row.1 };
    let mut expected = vec![Vec::<Row>::new(); pending.keys];
    for &row in reference {
        expected[key(&row) as usize].push(row);
    }
    for rows in &mut expected {
        rows.sort_unstable();
    }
    let mut found = vec![Vec::<Row>::new(); pending.keys];
    let mut offset = 0;
    for (run_index, run) in pending.view.runs[..pending.view.count as usize]
        .iter()
        .enumerate()
    {
        let local = &data[offset..offset + run.len as usize];
        assert!(
            local.windows(2).all(|w| {
                if first == 0 {
                    w[0] <= w[1]
                } else {
                    (w[0].1, w[0].0, w[0].2) <= (w[1].1, w[1].0, w[1].2)
                }
            }),
            "full-row run ordering"
        );
        for k in 0..pending.keys {
            let at = (run_index * pending.keys + k) * 2;
            let begin = ranges[at] as usize;
            let end = ranges[at + 1] as usize;
            assert!(begin <= end && end <= local.len(), "range bounds");
            assert!(
                local[begin..end].iter().all(|r| key(r) == k as i64),
                "directory returned wrong key"
            );
            found[k].extend_from_slice(&local[begin..end]);
        }
        offset += local.len();
    }
    for rows in &mut found {
        rows.sort_unstable();
    }
    assert_eq!(
        found, expected,
        "every key's unioned per-run ranges equals fresh rebuild"
    );
    let mut full = data.to_vec();
    full.sort_unstable();
    let mut reference = reference.to_vec();
    reference.sort_unstable();
    assert_eq!(full, reference, "full-tuple multiset");
    Ok(())
}
fn oracle_sequence(
    runtime: &Runtime,
    kernels: &oracle::LoadedModule,
    zipf: bool,
    ratio: u32,
) -> Result<()> {
    let mut index = LeveledDeviceIndex::<Row>::new(runtime, CHAINS, ratio)?;
    let mut dictionary = Dictionary::default();
    let mut all = Vec::new();
    let mut seed = 0x51ab_231f_7110_e3d9;
    for n in [
        0, 1, 7, 511, 512, 513, 1023, 4096, 4097, 100_000, 1_000_000, 1, 7, 1023, 100_000,
    ] {
        let delta_rows = rows(n, all.len(), zipf, &mut seed);
        let delta = upload(runtime, &delta_rows, &mut dictionary)?;
        // Enqueue round-t readers, then compact and retire their owners without
        // any intermediate synchronization. Download only after insertion.
        let old = enqueue_dump(
            runtime,
            kernels,
            index.snapshot(A)?,
            KEYS,
            Some(index.snapshot_ptr(A)?),
        )?;
        index.insert(&delta, n as u64)?;
        let old_rows = download(&old)?;
        check_dump(&old, &old_rows, &all, 0)?;
        all.extend(delta_rows);
        assert_eq!(index.rows(), all.len() as u64);
        assert!(index.run_count() <= MAX_LEVELS);
        let rebuilt_columns = upload(runtime, &all, &mut dictionary)?;
        let rebuilt = FrozenDeviceIndex::new(runtime, &rebuilt_columns, CHAINS)?;
        for (layout, first, keys) in [(A, 0, KEYS), (B, 1, 64)] {
            let got = enqueue_dump(
                runtime,
                kernels,
                index.snapshot(layout)?,
                keys,
                Some(index.snapshot_ptr(layout)?),
            )?;
            let fresh = enqueue_dump(
                runtime,
                kernels,
                one_run(rebuilt.resident_run(layout)?),
                keys,
                None,
            )?;
            let got_rows = download(&got)?;
            let fresh_rows = download(&fresh)?;
            check_dump(&fresh, &fresh_rows, &all, first)?;
            check_dump(&got, &got_rows, &fresh_rows, first)?;
            let errors = Buffer::<u32>::zeroed(runtime.pool().clone(), 1)?;
            if !all.is_empty() {
                let launch = kernels.prepare_check_prefixes(config(all.len()))?;
                unsafe {
                    kernels.check_prefixes(
                        runtime.stream(),
                        &launch,
                        got.view,
                        fresh.view.runs[0],
                        first,
                        errors.ptr(),
                    )?;
                }
            }
            assert_eq!(
                errors.to_host()?,
                vec![0],
                "every existing key prefix at depths 1/2/3"
            );
        }
        println!(
            "oracle zipf={zipf} ratio={ratio} delta={n} rows={} runs={} status={:?}",
            index.rows(),
            index.run_count(),
            index.status()
        );
    }
    if ratio == 2 {
        assert!(index.status().forced_merges > 0);
        assert_eq!(
            index.status().last_forced_merge,
            Some(LeveledMergeReason::MaxLevels)
        );
    }
    Ok(())
}
fn timing<T>(
    runtime: &Runtime,
    label: &'static CStr,
    f: impl FnOnce() -> Result<T>,
) -> Result<(T, f64, f64)> {
    runtime.stream().synchronize()?;
    let start = runtime
        .stream()
        .record_event(Some(cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT))?;
    let wall = Instant::now();
    let value = {
        let _range = cq_gpu_runtime::nvtx::range(label);
        f()?
    };
    let end = runtime
        .stream()
        .record_event(Some(cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT))?;
    end.synchronize()?;
    Ok((
        value,
        start.elapsed_ms(&end)? as f64,
        wall.elapsed().as_secs_f64() * 1000.0,
    ))
}
fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}
fn bench(runtime: &Runtime) -> Result<()> {
    let mut dictionary = Dictionary::default();
    let mut seed = 0x1853_4bb2_9d1c_421f;
    let base = rows(1_000_000, 0, true, &mut seed);
    let base_columns = upload(runtime, &base, &mut dictionary)?;
    for n in [1, 7, 1023, 4096, 100_000, 1_000_000] {
        let delta = rows(n, base.len(), true, &mut seed);
        let delta_columns = upload(runtime, &delta, &mut dictionary)?;
        let mut all = base.clone();
        all.extend_from_slice(&delta);
        let full_columns = upload(runtime, &all, &mut dictionary)?;
        let mut insert_ms = Vec::new();
        let mut rebuild_ms = Vec::new();
        let mut insert_wall = Vec::new();
        let mut rebuild_wall = Vec::new();
        for sample in 0..4 {
            let mut index = LeveledDeviceIndex::<Row>::new(runtime, &[A], 4)?;
            index.insert(&base_columns, base.len() as u64)?;
            let old = index.snapshot(A)?;
            let before = index.status();
            let label = match n {
                1 => c"leveled/bench/insert_one",
                7 => c"leveled/bench/insert_seven",
                1023 => c"leveled/bench/insert_1023",
                4096 => c"leveled/bench/insert_4096",
                1_000_000 => c"leveled/bench/insert_merge",
                _ => c"leveled/bench/insert_large",
            };
            let (_, gpu, wall) = timing(runtime, label, || index.insert(&delta_columns, n as u64))?;
            if n <= 4096 {
                let after = index.snapshot(A)?;
                assert_eq!(
                    index.status().merged_rows,
                    before.merged_rows,
                    "small insertion must not merge the base"
                );
                assert!(
                    after.runs[..after.count as usize]
                        .iter()
                        .any(|r| r.columns == old.runs[0].columns),
                    "base pointers must be unchanged"
                );
            }
            let label = match n {
                1 => c"leveled/bench/rebuild_one",
                7 => c"leveled/bench/rebuild_seven",
                1023 => c"leveled/bench/rebuild_1023",
                4096 => c"leveled/bench/rebuild_4096",
                1_000_000 => c"leveled/bench/rebuild_merge",
                _ => c"leveled/bench/rebuild_large",
            };
            let (_, full_gpu, full_wall) = timing(runtime, label, || {
                FrozenDeviceIndex::new(runtime, &full_columns, &[A])
            })?;
            println!(
                "sample delta={n} sample={sample} kind={} insert_gpu_ms={gpu:.6} rebuild_gpu_ms={full_gpu:.6} insert_wall_ms={wall:.6} rebuild_wall_ms={full_wall:.6} merge_rows={}",
                if sample == 0 {
                    "discarded-warmup"
                } else {
                    "warm"
                },
                index.status().merged_rows - before.merged_rows
            );
            if sample > 0 {
                insert_ms.push(gpu);
                rebuild_ms.push(full_gpu);
                insert_wall.push(wall);
                rebuild_wall.push(full_wall);
            }
        }
        println!(
            "median delta={n} warm_samples=3 insert_gpu_ms={:.6} rebuild_gpu_ms={:.6} insert_wall_ms={:.6} rebuild_wall_ms={:.6}",
            median(&mut insert_ms),
            median(&mut rebuild_ms),
            median(&mut insert_wall),
            median(&mut rebuild_wall)
        );
    }
    Ok(())
}
fn main() -> Result<()> {
    let mode = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "--oracle".to_owned());
    let runtime = Runtime::new()?;
    match mode.as_str() {
        "--oracle" => {
            let kernels = unsafe { oracle::load(runtime.context())? };
            let collective_errors = Buffer::<u32>::zeroed(runtime.pool().clone(), 1)?;
            let collective = kernels.prepare_check_block_any(config(512))?;
            unsafe {
                kernels.check_block_any(runtime.stream(), &collective, collective_errors.ptr())?;
            }
            assert_eq!(
                collective_errors.to_host()?,
                vec![0],
                "block-wide predicate OR"
            );
            oracle_sequence(&runtime, &kernels, false, 4)?;
            oracle_sequence(&runtime, &kernels, true, 4)?;
            oracle_sequence(&runtime, &kernels, true, 2)?;
            println!("leveled full-tuple and every-key oracle passed");
        }
        "--bench" => bench(&runtime)?,
        _ => return Err("expected --oracle or --bench".into()),
    }
    Ok(())
}

#[cuda_device::cuda_module]
mod oracle {
    use cq_gpu_runtime::{LeveledRunsView, RunView, run_column};
    use cuda_device::atomic::{AtomicOrdering, DeviceAtomicU32};
    use cuda_device::{device, kernel, launch_bounds, launch_contract, thread};
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn check_block_any(errors: *mut u32) {
        let empty = unsafe { cq_gpu_runtime::leveled_any(false) };
        let last_lane = unsafe { cq_gpu_runtime::leveled_any(thread::threadIdx_x() == 255) };
        let all = unsafe { cq_gpu_runtime::leveled_any(true) };
        if thread::threadIdx_x() == 0 && (empty || !last_lane || !all) {
            unsafe {
                DeviceAtomicU32::from_ptr(errors).fetch_add(1, AtomicOrdering::Relaxed);
            }
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn dump(view: LeveledRunsView, output: *mut i64) {
        let dst = thread::index_1d().get();
        let mut row = dst;
        let mut r = 0;
        while r < view.count as usize {
            let run = view.runs[r];
            if row < run.len as usize {
                let mut c = 0;
                while c < 3 {
                    unsafe {
                        output
                            .add(dst * 3 + c)
                            .write(run.columns.add(c).read().add(row).read());
                    }
                    c += 1;
                }
                return;
            }
            row -= run.len as usize;
            r += 1;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn dump_ptr(snapshot: *const LeveledRunsView, output: *mut i64) {
        let dst = thread::index_1d().get();
        let mut row = dst;
        let count = unsafe { cq_gpu_runtime::leveled_count(snapshot) } as usize;
        let mut r = 0;
        while r < count {
            let run = unsafe { cq_gpu_runtime::leveled_run(snapshot, r) };
            if row < run.len as usize {
                let mut c = 0;
                while c < 3 {
                    unsafe {
                        output
                            .add(dst * 3 + c)
                            .write(run.columns.add(c).read().add(row).read());
                    }
                    c += 1;
                }
                return;
            }
            row -= run.len as usize;
            r += 1;
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn ranges(view: LeveledRunsView, keys: usize, output: *mut u64) {
        let i = thread::index_1d().get();
        if i < keys * view.count as usize {
            let (begin, end) = unsafe {
                cq_gpu_runtime::blocked_run::point(view.runs[i / keys].rank, (i % keys) as i64)
            }
            .unwrap_or((0, 0));
            unsafe {
                output.add(i * 2).write(begin as u64);
                output.add(i * 2 + 1).write(end as u64);
            }
        }
    }

    #[device]
    unsafe fn refine(run: RunView, begin: u32, end: u32, slot: usize, key: i64) -> (u32, u32) {
        let mut lo = begin;
        let mut hi = end;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if unsafe { run_column(run, slot, mid) } < key {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        let start = lo;
        hi = end;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if unsafe { run_column(run, slot, mid) } <= key {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        (start, lo)
    }

    #[device]
    unsafe fn prefix_counts(run: RunView, first: usize, a: i64, b: i64, c: i64) -> [u64; 3] {
        let (begin, end) =
            unsafe { cq_gpu_runtime::blocked_run::point(run.rank, a) }.unwrap_or((0, 0));
        let (second_begin, second_end) = unsafe { refine(run, begin, end, 1 - first, b) };
        let (third_begin, third_end) = unsafe { refine(run, second_begin, second_end, 2, c) };
        [
            (end - begin) as u64,
            (second_end - second_begin) as u64,
            (third_end - third_begin) as u64,
        ]
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn check_prefixes(
        view: LeveledRunsView,
        fresh: RunView,
        first: usize,
        errors: *mut u32,
    ) {
        let row = thread::index_1d().get();
        if row >= fresh.len as usize {
            return;
        }
        let a = unsafe { run_column(fresh, first, row as u32) };
        let b = unsafe { run_column(fresh, 1 - first, row as u32) };
        let c = unsafe { run_column(fresh, 2, row as u32) };
        let expected = unsafe { prefix_counts(fresh, first, a, b, c) };
        let mut counts = [0u64; 3];
        let mut r = 0;
        while r < view.count as usize {
            let found = unsafe { prefix_counts(view.runs[r], first, a, b, c) };
            let mut depth = 0;
            while depth < 3 {
                counts[depth] += found[depth];
                depth += 1;
            }
            r += 1;
        }
        if counts[0] != expected[0] || counts[1] != expected[1] || counts[2] != expected[2] {
            unsafe {
                DeviceAtomicU32::from_ptr(errors).fetch_add(1, AtomicOrdering::Relaxed);
            }
        }
    }
}
