mod common;
mod maintain;
mod sort;
// Reuse the actual native spline builder, without changing its visibility or
// pretending that the alternative GPU radix directory is a RadixSpline.
#[path = "../../../../runtime/cq-storage/src/radix_spline.rs"]
mod radix_spline;

use common::{Gpu, Groups, Row, Sample, upload, view};
use maintain::Maintain;
use metal::{BufferRef, CommandBufferRef};
use serde_json::{Value, json};
use sort::Sort;
use std::time::Instant;

#[derive(Clone, Copy, Debug)]
enum Merge {
    Direct,
    Tiled,
    Partitioned,
}
impl Merge {
    fn encode(
        self,
        maintain: &Maintain,
        gpu: &Gpu,
        cb: &CommandBufferRef,
        a: &BufferRef,
        b: &BufferRef,
        out: &BufferRef,
        na: usize,
        nb: usize,
    ) {
        match self {
            Self::Direct => maintain.merge(gpu, cb, a, b, out, na, nb, false),
            Self::Tiled => maintain.merge(gpu, cb, a, b, out, na, nb, true),
            Self::Partitioned => maintain.merge_partitioned(gpu, cb, a, b, out, na, nb),
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Tiled => "tiled",
            Self::Partitioned => "partitioned",
        }
    }
}
const MERGES: [Merge; 3] = [Merge::Direct, Merge::Tiled, Merge::Partitioned];

fn random(s: &mut u64) -> u32 {
    *s ^= *s << 13;
    *s ^= *s >> 7;
    *s ^= *s << 17;
    (*s >> 16) as u32
}
fn rows(n: usize, mode: usize, start: u32) -> Vec<Row> {
    let mut seed = 0x83a6_237c_9fb1_429d_u64 ^ u64::from(start);
    (0..n)
        .map(|i| {
            let r = random(&mut seed);
            let key = match mode {
                0 => r,
                1 => r % 97,
                2 => u32::MAX,
                3 => {
                    if i % 2 == 0 {
                        0
                    } else {
                        u32::MAX
                    }
                }
                4 => i as u32,
                5 => (n - i) as u32,
                _ => r % (n / 4).max(1) as u32,
            };
            // Deliberately nonmonotonic payloads: key/value sort is NOT stable sort.
            Row {
                key,
                value: (start + i as u32).wrapping_mul(2_654_435_761),
            }
        })
        .collect()
}
fn median(values: impl Iterator<Item = f64>) -> f64 {
    let mut v: Vec<_> = values.collect();
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}
fn measure(gpu: &Gpu, runs: usize, mut encode: impl FnMut(&CommandBufferRef)) -> Vec<Sample> {
    gpu.run(|cb| encode(cb));
    (0..runs).map(|_| gpu.run(|cb| encode(cb))).collect()
}
fn record(
    results: &mut Vec<Value>,
    stage: &str,
    n: usize,
    logical_bytes: Option<u64>,
    samples: &[Sample],
    params: Value,
) {
    let wall = median(samples.iter().map(|s| s.wall_ms));
    let gpu = median(samples.iter().map(|s| s.gpu_ms));
    let bw = logical_bytes.map(|b| b as f64 / (gpu * 1e6));
    println!(
        "{stage:25} n={n:10} wall={wall:9.3}ms gpu={gpu:9.3}ms logical_GB/s={} {params}",
        bw.map_or_else(|| "n/a".into(), |x| format!("{x:.2}"))
    );
    results.push(json!({"stage":stage,"rows":n,"params":params,"logical_bytes":logical_bytes,
        "wall_ms":wall,"gpu_ms":gpu,"logical_gbps":bw,
        "samples":samples.iter().map(|s|json!({"wall_ms":s.wall_ms,"gpu_ms":s.gpu_ms})).collect::<Vec<_>>()}));
}
fn cpu_record(results: &mut Vec<Value>, stage: &str, n: usize, ms: Vec<f64>, params: Value) {
    let time = median(ms.iter().copied());
    println!("{stage:25} n={n:10} wall={time:9.3}ms {params}");
    results.push(json!({"stage":stage,"rows":n,"wall_ms":time,"samples_ms":ms,"params":params}));
}
fn check_groups(groups: &Groups, sorted: &[Row]) {
    let count = groups.len();
    let keys = view::<u32>(&groups.keys, count);
    let starts = view::<u32>(&groups.starts, count + 1);
    assert_eq!(starts[count] as usize, sorted.len());
    let mut group = 0;
    for (i, row) in sorted.iter().enumerate() {
        if i == 0 || sorted[i - 1].key != row.key {
            assert_eq!(keys[group], row.key);
            assert_eq!(starts[group] as usize, i);
            group += 1;
        }
    }
    assert_eq!(group, count);
}
fn check_directory(
    gpu: &Gpu,
    maintain: &Maintain,
    data: &BufferRef,
    dir: &BufferRef,
    expected: &[Row],
) {
    let bounds = view::<u32>(dir, 4097);
    for (prefix, &bound) in bounds.iter().enumerate() {
        let key = (prefix as u64) << 20;
        assert_eq!(
            bound as usize,
            expected.partition_point(|r| u64::from(r.key) < key)
        );
    }
    let mut queries = vec![0, 1, u32::MAX, u32::MAX - 1];
    let mut seed = 41571;
    for i in 0..4096 {
        queries.push(random(&mut seed));
        if !expected.is_empty() {
            queries.push(expected[i * expected.len() / 4096].key);
        }
    }
    let q = gpu.buffer(queries.len() * 4);
    let out = gpu.buffer(queries.len() * 8);
    upload(&q, &queries);
    gpu.run(|cb| maintain.lookup(gpu, cb, data, dir, &q, &out, expected.len(), queries.len()));
    for (&query, range) in queries.iter().zip(view::<[u32; 2]>(&out, queries.len())) {
        let lo = expected.partition_point(|r| r.key < query);
        let hi = expected.partition_point(|r| r.key <= query);
        let wanted = if lo == hi {
            [expected.len() as u32; 2]
        } else {
            [lo as u32, hi as u32]
        };
        assert_eq!(*range, wanted, "query {query}");
    }
}
fn merge_reference(a: &[Row], b: &[Row]) -> Vec<Row> {
    let mut result = Vec::with_capacity(a.len() + b.len());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].key <= b[j].key {
            result.push(a[i]);
            i += 1;
        } else {
            result.push(b[j]);
            j += 1;
        }
    }
    result.extend_from_slice(&a[i..]);
    result.extend_from_slice(&b[j..]);
    result
}
fn correctness(gpu: &Gpu) {
    for (items, bits) in [(4, 4), (8, 4), (4, 8), (8, 8)] {
        let sort = Sort::with_digit_bits(gpu, 65539, items, bits);
        let maintain = Maintain::new(gpu, 131078, items);
        let groups = Groups::new(gpu, 131078);
        let input = gpu.buffer(65539 * 8);
        let output = gpu.buffer(65539 * 8);
        let right = gpu.buffer(65539 * 8);
        let merged = gpu.buffer(131078 * 8);
        let dir = gpu.buffer(4097 * 4);
        for n in [
            0, 1, 2, 31, 32, 33, 255, 256, 257, 1023, 1024, 1025, 2047, 2048, 2049, 65539,
        ] {
            for mode in 0..7 {
                let source = rows(n, mode, 0);
                let mut expected = source.clone();
                expected.sort_by_key(|r| r.key);
                upload(&input, &source);
                gpu.run(|cb| {
                    sort.encode(gpu, cb, &input, &output, n);
                    maintain.directory(gpu, cb, &output, &dir, n);
                    groups.encode(gpu, cb, &output, n);
                });
                assert_eq!(
                    view::<Row>(&output, n),
                    expected,
                    "sort items={items} n={n} mode={mode}"
                );
                assert_eq!(view::<Row>(&input, n), source, "sort modified input");
                check_groups(&groups, &expected);
                check_directory(gpu, &maintain, &output, &dir, &expected);
                for nb in [0, 1, n / 97, n] {
                    let mut b = rows(nb, mode, n as u32);
                    b.sort_by_key(|r| r.key);
                    let combined = merge_reference(&expected, &b);
                    upload(&right, &b);
                    for algorithm in MERGES {
                        gpu.run(|cb| {
                            algorithm.encode(&maintain, gpu, cb, &output, &right, &merged, n, nb)
                        });
                        assert_eq!(
                            view::<Row>(&merged, n + nb),
                            combined,
                            "merge items={items} n={n} nb={nb} mode={mode} algorithm={algorithm:?}"
                        );
                        // Also exercise an empty left run and right-before-left keys.
                        gpu.run(|cb| {
                            algorithm.encode(&maintain, gpu, cb, &right, &output, &merged, nb, n)
                        });
                        assert_eq!(view::<Row>(&merged, n + nb), merge_reference(&b, &expected));
                    }
                }
            }
        }
    }
    println!(
        "Correctness: full stable row equality, input preservation, grouping, all radix bounds, hit/miss ranges, both merge orientations passed."
    );
}
fn bench_size(gpu: &Gpu, n: usize, runs: usize, results: &mut Vec<Value>) {
    let input_rows = rows(n, 0, 0);
    let mut expected = input_rows.clone();
    let start = Instant::now();
    expected.sort_by_key(|r| r.key);
    cpu_record(
        results,
        "cpu_stable_sort",
        n,
        vec![start.elapsed().as_secs_f64() * 1000.0],
        json!({"runs":1,"copy_excluded":true}),
    );
    let input = gpu.buffer(n * 8);
    upload(&input, &input_rows);
    let output = gpu.buffer(n * 8);
    let dir = gpu.buffer(4097 * 4);
    let copy = gpu.pipeline("copy_rows");
    for grid in [256, 1024, 4096, 16384] {
        let samples = measure(gpu, runs, |cb| {
            gpu.dispatch(cb, &copy, &[&input, &output], &[n as u32], grid, 256)
        });
        assert_eq!(view::<Row>(&output, n), input_rows);
        record(
            results,
            "copy",
            n,
            Some((n * 16) as u64),
            &samples,
            json!({"grid":grid}),
        );
    }
    for items in [4, 8] {
        let sort = Sort::new(gpu, n, items);
        let maintain = Maintain::new(gpu, n, items);
        let groups = Groups::new(gpu, n);
        let samples = measure(gpu, runs, |cb| sort.encode(gpu, cb, &input, &output, n));
        assert_eq!(view::<Row>(&output, n), expected);
        record(
            results,
            "sort",
            n,
            Some(sort.logical_bytes(n)),
            &samples,
            json!({"items":items,"distribution":"uniform_u32"}),
        );
        let samples = measure(gpu, runs, |cb| {
            maintain.directory(gpu, cb, &output, &dir, n)
        });
        check_directory(gpu, &maintain, &output, &dir, &expected);
        record(
            results,
            "radix_directory",
            n,
            None,
            &samples,
            json!({"items":items,"entries":4097}),
        );
        let samples = measure(gpu, runs, |cb| groups.encode(gpu, cb, &output, n));
        check_groups(&groups, &expected);
        record(
            results,
            "group_boundaries",
            n,
            None,
            &samples,
            json!({"items":items,"unique_keys":groups.len()}),
        );
        let samples = measure(gpu, runs, |cb| {
            sort.encode(gpu, cb, &input, &output, n);
            maintain.directory(gpu, cb, &output, &dir, n);
        });
        assert_eq!(view::<Row>(&output, n), expected);
        record(
            results,
            "gpu_radix_build",
            n,
            None,
            &samples,
            json!({"items":items,"grouped_metadata":false}),
        );
        let mut samples = Vec::new();
        let mut cpu_times = Vec::new();
        let mut total_times = Vec::new();
        for rep in 0..=runs {
            let start = Instant::now();
            let sample = gpu.run(|cb| {
                sort.encode(gpu, cb, &input, &output, n);
                groups.encode(gpu, cb, &output, n);
            });
            let keys = view::<u32>(&groups.keys, groups.len());
            let cpu_start = Instant::now();
            let spline = radix_spline::RadixSpline::build(keys.len(), |i| u64::from(keys[i]));
            let cpu_ms = cpu_start.elapsed().as_secs_f64() * 1000.0;
            let total_ms = start.elapsed().as_secs_f64() * 1000.0;
            for &key in keys.iter().step_by((keys.len() / 4096).max(1)) {
                let range = spline.search_bound(u64::from(key));
                assert!(keys[range].binary_search(&key).is_ok());
            }
            std::hint::black_box(&spline);
            if rep > 0 {
                samples.push(sample);
                cpu_times.push(cpu_ms);
                total_times.push(total_ms);
            }
        }
        check_groups(&groups, &expected);
        record(
            results,
            "sort_and_group",
            n,
            None,
            &samples,
            json!({"items":items}),
        );
        cpu_record(
            results,
            "native_spline_only",
            n,
            cpu_times,
            json!({"items":items,"unique_keys":groups.len()}),
        );
        cpu_record(
            results,
            "hybrid_spline_build",
            n,
            total_times,
            json!({"items":items,"gpu_sort_and_group_then_cpu_spline":true}),
        );
    }
    // Actual production provider, including its allocations/grouping/directory.
    let mut times = Vec::new();
    for rep in 0..=runs.min(3) {
        let start = Instant::now();
        let index = cq_storage::RadixRun::from_pairs(input_rows.iter().map(|r| (r.key, r.value)));
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(index.items().len(), n);
        assert!(
            index
                .items()
                .iter()
                .zip(&expected)
                .all(|(&v, r)| v == r.value)
        );
        for r in expected.iter().step_by((n / 1024).max(1)) {
            let (lo, hi) = index.range(&r.key);
            assert_eq!(lo, expected.partition_point(|x| x.key < r.key));
            assert_eq!(hi, expected.partition_point(|x| x.key <= r.key));
        }
        std::hint::black_box(&index);
        if rep > 0 {
            times.push(ms);
        }
    }
    cpu_record(
        results,
        "native_radixrun_build",
        n,
        times,
        json!({"allocation_included":true,"threads":1}),
    );
    drop(input_rows);
    drop(expected);
}
fn bench_updates(gpu: &Gpu, n: usize, runs: usize, results: &mut Vec<Value>) {
    // A sorted sparse full-width base, with payload order unrelated to key.
    let base: Vec<_> = (0..n)
        .map(|i| Row {
            key: ((i as u64 * u32::MAX as u64) / n.max(1) as u64) as u32,
            value: (i as u32).wrapping_mul(2654435761),
        })
        .collect();
    let a = gpu.buffer(n * 8);
    upload(&a, &base);
    for percent in [1, 10, 100] {
        let d = (n * percent / 100).max(1);
        let total = n + d;
        let delta = rows(d, 0, n as u32);
        let mut delta_sorted = delta.clone();
        delta_sorted.sort_by_key(|r| r.key);
        let expected = merge_reference(&base, &delta_sorted);
        let raw = gpu.buffer(d * 8);
        upload(&raw, &delta);
        let b = gpu.buffer(d * 8);
        upload(&b, &delta_sorted);
        let out = gpu.buffer(total * 8);
        let dir = gpu.buffer(4097 * 4);
        for items in [4, 8] {
            let sort = Sort::with_digit_bits(gpu, d, 8, 8);
            let maintain = Maintain::new(gpu, total, items);
            for algorithm in MERGES {
                let samples = measure(gpu, runs, |cb| {
                    algorithm.encode(&maintain, gpu, cb, &a, &b, &out, n, d)
                });
                assert_eq!(view::<Row>(&out, total), expected);
                record(
                    results,
                    "merge",
                    total,
                    Some((total * 16) as u64),
                    &samples,
                    json!({"items":items,"algorithm":algorithm.name(),"base_rows":n,"delta_rows":d,"delta_percent":percent}),
                );
                let samples = measure(gpu, runs, |cb| {
                    sort.encode(gpu, cb, &raw, &b, d);
                    algorithm.encode(&maintain, gpu, cb, &a, &b, &out, n, d);
                    maintain.directory(gpu, cb, &out, &dir, total);
                });
                assert_eq!(view::<Row>(&out, total), expected);
                check_directory(gpu, &maintain, &out, &dir, &expected);
                record(
                    results,
                    "flat_update",
                    total,
                    None,
                    &samples,
                    json!({"items":items,"algorithm":algorithm.name(),"base_rows":n,"delta_rows":d,"delta_percent":percent,"sort_delta_merge_directory":true,"sort_bits":8,"sort_items":8}),
                );
            }
        }
    }
}
fn history(gpu: &Gpu, delta: usize, runs: usize, results: &mut Vec<Value>) {
    let rounds = 8;
    let n = delta * rounds;
    let input = rows(n, 1, 0);
    let mut expected = input.clone();
    expected.sort_by_key(|r| r.key);
    let chunks: Vec<_> = input
        .chunks(delta)
        .map(|rows| {
            let b = gpu.buffer(delta * 8);
            upload(&b, rows);
            b
        })
        .collect();
    let sort = Sort::with_digit_bits(gpu, delta, 8, 8);
    let maintain = Maintain::new(gpu, n, 4);
    // Flat compaction publishes one new run per round. Binary carries publish
    // at most one run per level; all levels must be probed until compaction.
    for leveled in [false, true] {
        let mut buffers: Vec<metal::Buffer> = Vec::new();
        let mut operations: Vec<(usize, usize, usize, usize, usize)> = Vec::new();
        let mut round_ends = Vec::with_capacity(rounds);
        let mut sorted_slots = Vec::new();
        let mut levels: Vec<Option<(usize, usize)>> = vec![None; 4];
        let mut flat: Option<(usize, usize)> = None;
        for _ in 0..rounds {
            sorted_slots.push(buffers.len());
            buffers.push(gpu.buffer(delta * 8));
        }
        let mut merged_rows = 0;
        for &slot in &sorted_slots {
            let mut current = (slot, delta);
            if leveled {
                for level in &mut levels {
                    if let Some(old) = level.take() {
                        let output = buffers.len();
                        buffers.push(gpu.buffer((old.1 + current.1) * 8));
                        operations.push((old.0, current.0, output, old.1, current.1));
                        merged_rows += old.1 + current.1;
                        current = (output, old.1 + current.1);
                    } else {
                        *level = Some(current);
                        break;
                    }
                }
            } else {
                if let Some(old) = flat {
                    let output = buffers.len();
                    buffers.push(gpu.buffer((old.1 + current.1) * 8));
                    operations.push((old.0, current.0, output, old.1, current.1));
                    merged_rows += old.1 + current.1;
                    current = (output, old.1 + current.1);
                }
                flat = Some(current);
            }
            round_ends.push(operations.len());
        }
        let final_slot = if leveled {
            levels.iter().flatten().next().unwrap().0
        } else {
            flat.unwrap().0
        };
        let dirs: Vec<_> = buffers.iter().map(|_| gpu.buffer(4097 * 4)).collect();
        let encode_round = |cb: &CommandBufferRef, round: usize| {
            let slot = sorted_slots[round];
            sort.encode(gpu, cb, &chunks[round], &buffers[slot], delta);
            maintain.directory(gpu, cb, &buffers[slot], &dirs[slot], delta);
            let begin = if round == 0 { 0 } else { round_ends[round - 1] };
            for &(a, b, out, na, nb) in &operations[begin..round_ends[round]] {
                maintain.merge_partitioned(
                    gpu,
                    cb,
                    &buffers[a],
                    &buffers[b],
                    &buffers[out],
                    na,
                    nb,
                );
                maintain.directory(gpu, cb, &buffers[out], &dirs[out], na + nb);
            }
        };
        for single_submission in [true, false] {
            let samples = if single_submission {
                measure(gpu, runs, |cb| {
                    for round in 0..rounds {
                        encode_round(cb, round);
                    }
                })
            } else {
                let mut samples = Vec::with_capacity(runs);
                for rep in 0..=runs {
                    let start = Instant::now();
                    let mut gpu_ms = 0.0;
                    for round in 0..rounds {
                        gpu_ms += gpu.run(|cb| encode_round(cb, round)).gpu_ms;
                    }
                    if rep > 0 {
                        samples.push(Sample {
                            wall_ms: start.elapsed().as_secs_f64() * 1000.0,
                            gpu_ms,
                        });
                    }
                }
                samples
            };
            assert_eq!(view::<Row>(&buffers[final_slot], n), expected);
            check_directory(
                gpu,
                &maintain,
                &buffers[final_slot],
                &dirs[final_slot],
                &expected,
            );
            // Every published run is checked, not just the final power-of-two level.
            for &(a, b, out, na, nb) in &operations {
                assert_eq!(
                    view::<Row>(&buffers[out], na + nb),
                    merge_reference(view::<Row>(&buffers[a], na), view::<Row>(&buffers[b], nb))
                );
            }
            record(
                results,
                "maintenance_history",
                n,
                None,
                &samples,
                json!({"delta_rows":delta,"rounds":rounds,"leveled":leveled,"merged_rows":merged_rows,"merge_logical_bytes":merged_rows*16,"distribution":"97_keys","single_submission":single_submission,"sort_bits":8,"sort_items":8,"merge_items":4,"algorithm":"partitioned"}),
            );
        }
    }
}
fn bench_radix_digits(gpu: &Gpu, n: usize, runs: usize, results: &mut Vec<Value>) {
    for mode in 0..3 {
        let source = rows(n, mode, 0);
        let mut expected = source.clone();
        expected.sort_by_key(|r| r.key);
        let input = gpu.buffer(n * 8);
        upload(&input, &source);
        let output = gpu.buffer(n * 8);
        let dir = gpu.buffer(4097 * 4);
        let maintain = Maintain::new(gpu, n, 4);
        if mode == 0 {
            let copy = gpu.pipeline("copy_rows");
            let samples = measure(gpu, runs, |cb| {
                gpu.dispatch(cb, &copy, &[&input, &output], &[n as u32], 16384, 256)
            });
            assert_eq!(view::<Row>(&output, n), source);
            record(
                results,
                "copy",
                n,
                Some((n * 16) as u64),
                &samples,
                json!({"grid":16384}),
            );
        }
        for (items, bits) in [(4, 4), (8, 4), (4, 8), (8, 8)] {
            let sort = Sort::with_digit_bits(gpu, n, items, bits);
            let samples = measure(gpu, runs, |cb| sort.encode(gpu, cb, &input, &output, n));
            assert_eq!(view::<Row>(&output, n), expected);
            let distribution = ["uniform_u32", "97_keys", "all_equal"][mode];
            record(
                results,
                "radix_tuning",
                n,
                Some(sort.logical_bytes(n)),
                &samples,
                json!({"items":items,"bits":bits,"distribution":distribution}),
            );
            let samples = measure(gpu, runs, |cb| {
                sort.encode(gpu, cb, &input, &output, n);
                maintain.directory(gpu, cb, &output, &dir, n);
            });
            assert_eq!(view::<Row>(&output, n), expected);
            check_directory(gpu, &maintain, &output, &dir, &expected);
            record(
                results,
                "radix_tuned_build",
                n,
                None,
                &samples,
                json!({"items":items,"bits":bits,"distribution":distribution}),
            );
        }
    }
}

fn main() {
    metal::objc::rc::autoreleasepool(|| {
        let args: Vec<_> = std::env::args().collect();
        let value = |flag: &str| {
            args.iter()
                .position(|x| x == flag)
                .map(|i| args.get(i + 1).expect("missing option value").as_str())
        };
        let sizes: Vec<usize> = value("--sizes")
            .unwrap_or("1048576,16777216")
            .split(',')
            .map(|x| x.parse().unwrap())
            .collect();
        assert!(sizes.iter().all(|&n| n > 0 && n <= u32::MAX as usize / 2));
        let runs: usize = value("--runs").unwrap_or("5").parse().unwrap();
        assert!(runs > 0);
        let output = value("--out").unwrap_or("results.json");
        let gpu = Gpu::new();
        println!("Device: {}", gpu.device.name());
        correctness(&gpu);
        if args.iter().any(|x| x == "--check-only") {
            return;
        }
        let mut results = Vec::new();
        let sort_only = args.iter().any(|x| x == "--sort-only");
        for &n in &sizes {
            if sort_only {
                bench_radix_digits(&gpu, n, runs, &mut results);
            } else {
                if !args.iter().any(|x| x == "--updates-only") {
                    bench_size(&gpu, n, runs, &mut results);
                }
                bench_updates(&gpu, n, runs, &mut results);
            }
        }
        if !sort_only {
            for delta in [65536, 1048576] {
                history(&gpu, delta, runs, &mut results);
            }
        }
        let provenance = json!({"device":gpu.device.name(),"os":std::env::consts::OS,"arch":std::env::consts::ARCH,
            "args":args,"runs":runs,"seed":"83a6237c9fb1429d","correctness":"passed",
            "storage":"shared; tracked hazards; allocations and input generation outside GPU timed regions",
            "timing":"GPU command-buffer timestamp and host encode+commit+wait; warmup excluded; medians",
            "bandwidth":"decimal GB/s; modeled logical read+write bytes, NOT measured DRAM traffic",
            "scope":"append-only u32 key/u32 payload; stable duplicates; GPU radix12 exact directory is not RadixSpline; native spline builder remains CPU",
            "origin":"developed in an isolated GPU-index snapshot; integrated alongside native join improvements",
            "source_directory":env!("CARGO_MANIFEST_DIR"),
            "results":results});
        std::fs::write(output, serde_json::to_vec_pretty(&provenance).unwrap())
            .expect("write results");
        println!("Results: {output}");
    });
}
