# Mac GPU sorted-index experiments

Imported from the independent `mini_linq-gpu-index-20261024` snapshot into the
working tree containing the CPU join improvements. Only this experiment was
copied; the snapshot's older compiler/runtime files were not imported.
The measurements below predate integration and retain their original provenance.
They are not new measurements of the current native provider.

This standalone Rust/Metal executable implements stable u32-key/u32-payload
sorting, stable old-run-before-delta merging, exact radix-prefix directories,
GPU group-boundary extraction, and flat/binary-carry maintenance experiments.
It does **not** implement a production Mac backend or GPU RadixSpline builder.
See [the implementation plan](../../../doc/MAC-IMPLEMENTATION.md).

## Run

From the repository root, on an Apple-silicon Mac with Metal available:

```sh
cargo run --release --locked --manifest-path experiments/mac/gpu-index/Cargo.toml -- --check-only
cargo run --release --locked --manifest-path experiments/mac/gpu-index/Cargo.toml -- \
  --sort-only --sizes 1048576,16777216,67108864 --runs 5 \
  --out experiments/mac/gpu-index/radix-rerun.json
cargo run --release --locked --manifest-path experiments/mac/gpu-index/Cargo.toml -- \
  --updates-only --sizes 1048576,16777216,67108864 --runs 7 \
  --out experiments/mac/gpu-index/maintenance-rerun.json
```

Without either sweep flag, the executable also measures ordinary native
`RadixRun` construction and GPU sorting/grouping followed by the native CPU
RadixSpline builder. That construction reference deliberately uses the original
4-bit sort. Updates use the selected 8-bit sort. `--sort-only` compares both.
No original evidence file is overwritten by the commands above.

Correctness checks execute real Metal kernels and compare complete rows against
stable CPU sorting/merging: arbitrary nonmonotonic payloads, empty inputs,
duplicates, full-width u32 extrema, irregular tiles, both merge orientations,
all radix-directory boundaries, exact hit/miss ranges, and group starts.
Large timed outputs and intermediate history merges are validated outside the
timing window. Equal-key occurrences are preserved, not deduplicated.

## Evidence and measured choices

M3 Max, 40 GPU cores, 128 GiB memory:

- [Initial construction/native comparison](m3max-baseline.json).
- [Controlled merge-partition comparison](m3max-partitioned.json).
- [Radix digit/distribution sweep](m3max-radix-digits.json).
- [Final maintenance sweep](m3max-tuned-maintenance.json): 116 measurement records,
  seven samples each, after warmup.

Choose four **8-bit radix passes with 8 rows/thread** for large builds.
Choose **parallel merge-path partitioning followed by 4-row/thread tiled merge**
for large maintenance. The separate partition dispatch avoids serial global
boundary searches inside every merging threadgroup.

67,108,864 uniform full-width keys, five-sample GPU medians:

| Operation | GPU ms | Modeled logical GB/s |
|---|---:|---:|
| Copy reference | 3.044 | 352.77 |
| Stable sort, 4-bit digits, 8 rows/thread | 35.443 | 305.79 |
| Stable sort, 8-bit digits, 8 rows/thread | 22.228 | 277.79 |
| 8-bit sort plus exact radix directory | 22.284 | not assigned |

The faster sort moves fewer bytes; maximizing reported GB/s would select the
wrong algorithm. Directory-inclusive host time was 22.587 ms. The same build
with 97 distinct keys took 20.983 ms GPU time; all-equal keys took 20.789 ms.
SIMD-private histograms aggregate uniform digits before atomics. Stable ranks
never depend on atomic return order.

Final updates into a 67,108,864-row base, seven-sample medians:

| Delta/base | Output rows | Merge GPU ms | Merge logical GB/s | Whole update GPU ms | Whole update host ms |
|---|---:|---:|---:|---:|---:|
| 1% | 67,779,952 | 3.204 | 338.49 | 3.542 | 3.857 |
| 10% | 73,819,750 | 3.544 | 333.30 | 5.748 | 6.062 |
| 100% | 134,217,728 | 6.876 | 312.30 | 29.241 | 29.565 |

Whole update means sort raw delta, merge with the old run, and rebuild the exact
radix directory in one submission. In the controlled comparison, balanced
merge took 21.370 ms direct, 12.133 ms tiled, and 6.970 ms partitioned at
4 rows/thread. Eight rows/thread was slower for merging despite winning for sort.

Eight 1,048,576-row duplicate-heavy deltas merged 25,165,824 rows with binary
carries versus 36,700,160 with flat compaction: 31.4% less merge traffic.
Batched host times were 4.770 versus 5.523 ms; separately completed submissions
cost 6.752 versus 7.087 ms. At 65,536 rows/delta, leveled separate submissions
cost 2.555 ms versus 1.281 ms batched and did not beat flat's 2.472 ms separate
submissions. The histories pre-generate their deltas and do not charge query
work or extra probes across live levels. They are not a persistent LSM provider.

## Measurement boundaries

- Shared, pre-touched, hazard-tracked buffers; dependencies use separate compute
  encoders in one command buffer. GPU allocation, compilation, initial CPU
  population and validation are excluded from timed GPU regions.
- Both GPU command-buffer timestamps and host encode/submit/wait times are
  recorded. Native-provider times include its own allocation. Comparisons are
  not cold end-to-end transfer/allocation speedups.
- Decimal GB/s counts modeled global-buffer reads/writes, **not DRAM counters**.
  Copy counts 16 bytes per 8-byte row; merge counts 16 bytes per output row,
  excluding merge-path searches and partition metadata. Sort counts every pass
  and histogram-scan metadata, excluding threadgroup traffic. The merge's
  89–96% of copy rate is not a measured physical-bandwidth utilization ratio.
- Small-input timings varied substantially between invocations. Other activity
  on the shared Mac affects timing. Prefer controlled within-run comparisons.
- Keys/payload positions are u32. Wide payload materialization, 64-bit keys,
  deletion, concurrent readers, persistence and production-provider adoption
  have not been implemented here.

Keeping the native spline was CPU-bound in the initial snapshot: the 67M-row
hybrid build took 1,669 ms host time, with about 1,596 ms in CPU spline building;
ordinary native construction took 4,443 ms. The hybrid experiment does not
adopt its buffers into the native provider's layout. The exact GPU radix
alternative avoids that sequential spline build but has unmeasured probe
performance and potentially skewed high-bit buckets on dense domains.
Do not substitute it for RadixSpline based on build time alone.
