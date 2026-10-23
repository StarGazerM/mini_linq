# maccal — Apple-silicon primitive-cost calibration

Measures the primitive operations a trace-driven cost model of index build /
merge maintenance / join probe is built from, on the CPU, on the GPU (Metal
compute) and with both running at once. Standalone cargo workspace; the Metal
kernels in `src/kernels.metal` are compiled by `build.rs` (`xcrun metal -O3`,
`metallib`) into `OUT_DIR` and loaded at start.

## Running

```
cargo run --release -- <cpu|gpu|hybrid|all> [flags]

  --footprints 4MiB,64MiB,1GiB,4GiB   primary array size per primitive (default shown)
  --threads 1,5,10,15                 CPU worker counts (default; hw.ncpu is 15 = 5P + 10E)
  --reps 9                            timed repetitions (plus 2 warm-ups); medians reported
  --seconds 2.0                       hybrid pairing window
  --probes 32M                        random accesses per gather/scatter/rmw run
  --seed 42
  --quick                             4MiB,64MiB / threads 1,6 / reps 3 / 0.5 s / 2M probes
```

Each run writes `results/<sub>-<YYYY-MM-DD>.json` (array of `Measurement`)
and `results/<sub>-<date>.md` (the table printed to stdout). `all` runs the
three phases in sequence (CPU, GPU, hybrid), dropping each phase's buffers
before the next.

## Conventions

* One `Work` = fixed `bytes`/`items`; `run()` returns a checksum consumed by
  `black_box`. Buffers are allocated and page-touched once, outside timing;
  random indices are generated once by a fixed-seed xorshift and stored in a
  buffer, so gathers time the gather, not the RNG.
* `median_ms` is the median of `reps` wall-clock runs (`Instant`), after 2
  warm-ups. For GPU works this includes command-buffer encode, commit and
  `waitUntilCompleted`; `gpu_ms` is `GPUEndTime - GPUStartTime` of the same
  command buffer (device time only), so dispatch/submission overhead is the
  difference.
* **Nominal bytes** are what the model counts, not what the hardware moves:
  `seq_read`/`seq_write` = footprint, `seq_copy` = 2x; a random 4-byte gather
  or scatter counts one 128 B line (`N*128`); an atomic RMW counts the line
  twice (`N*256`); partition counts 4 columns (`16*N`) plus the per-tile
  histogram traffic; merge counts `3*4*N`; scans count 2 or 3 passes of `4*N`.
  Hence GB/s above DRAM bandwidth for the 4 MiB (cache-resident) rows.
* `ns_per_item` is `median_ms * 1e6 / items`; for throughput primitives this is
  the amortised cost per element, not latency.

## GPU primitives (`src/gpu.rs`, `src/kernels.metal`)

All buffers are `StorageModeShared | HazardTrackingModeUntracked`, threadgroups
are 256 wide, elements are `u32`. The suite shares a lazily allocated buffer
pool (`a` = read-only random data, `b` = scratch/output, `sorted`, `idx:<lines>`).

| op | what it measures | items / bytes |
|---|---|---|
| `seq_read` | grid-stride `uint4` sum; `simd_sum` -> threadgroup atomic -> one device atomic per threadgroup | N / 4N |
| `seq_write` | grid-stride `uint4` fill | N / 4N |
| `seq_copy` | grid-stride `uint4` copy | N / 8N |
| `gather_line` | one random 4 B load per lane from a distinct random 128 B line (precomputed line index * 32) | probes / probes*128 |
| `gather_coop` | each SIMD-group walks 32 random lines, 32 lanes loading each 128 B line coalesced (same line list and thread count as `gather_line`) | lines / lines*128 |
| `scatter_line` | one random 4 B store per lane | probes / probes*128 |
| `rmw_atomic` | random `atomic_fetch_add_explicit` on device u32 | probes / probes*256 |
| `rmw_atomic_contended` | same, all lanes over 1024 lines | probes / probes*256 |
| `hist_scan` | histogram into 256 or 65536 bins, threadgroup-privatised in 32 KiB of threadgroup memory (u32 counters for <= 8192 bins; for 2^16 bins two packed 16-bit counters per word, 16384-bin slices, 4 passes over the threadgroup's key chunk of < 65536 keys), one device atomic per non-zero bin per threadgroup, then exclusive scan of the bins by reduce-then-scan | N / 4N + 16*bins |
| `partition_pass` | one 8-bit radix partition of (key,payload): per-tile (2048) histogram, reduce-then-scan of bin-major offsets, tile-local counting sort in threadgroup memory, coalesced scatter. Not stable within a tile. | N / 16N + 20*256*tiles |
| `merge_path` | merge two sorted N/2 arrays: 2 global diagonal searches per 2048-tile, tile staged in threadgroup memory, 8 outputs per thread by local merge path | N / 12N |
| `scan_rts` | reduce-then-scan exclusive scan, 3 dispatches: block sums, scan of block sums in one threadgroup, downsweep | N / 12N |
| `lookback_scan` | single-pass decoupled-look-back exclusive scan with a device atomic ticket; watchdog (`watchdog_cap` polls) sets `forward_progress=false` instead of hanging | N / 8N |
| `cmdbuf_empty` | commit + `waitUntilCompleted` of a command buffer with no encoders | 1 |
| `dispatch_empty` | 1 or 100 empty dispatches in one compute encoder | dispatches |
| `dispatch_empty_enc` | 100 empty dispatches, one encoder each | 100 |
| `sync_roundtrip` | `MTLSharedEvent` ping-pong, 100 rounds per command buffer: GPU `signal(v)`, CPU thread spins on `signaledValue`, sets `v+1`, GPU `wait(v+1)` | rounds |
| `sync_roundtrip_notify` | same with the CPU side as `MTLSharedEventListener` notify blocks | rounds |

`correct=true` in the notes column means the output was verified on the CPU
on the first run (histogram total, partition bin order, merged order, scan
total).

## CPU primitives

See `src/cpu.rs` (`seq_*`, `gather_line`, `scatter_line`, `rmw_atomic`,
`gather_dep`, `hashbrown_build/probe`, `sort_u32`, `merge_u32`, `hist_scan`),
each at every `--threads` x `--footprints`.

## Hybrid (`src/hybrid.rs`)

For each default pair, each side is timed solo (`time_work`), then a CPU
thread runs `run_for(cpu, seconds)` while the main thread runs
`run_for(gpu, seconds)` from one `Barrier`. Both rows carry
`partner`, `solo_median_ms`, `solo_gbps` and `slowdown = paired / solo`.
Pairs (`hybrid::DEFAULT_PAIRS`, CPU threads 15 = all cores, 5 = P-cores only,
10): cpu seq_read(15) x gpu seq_read; cpu gather_line(15, 1 GiB | 64 MiB) x
gpu seq_copy; cpu gather_line(15) x gpu merge_path; cpu hashbrown_probe(15,
64 MiB | 1 GiB) x gpu partition_pass; cpu hashbrown_probe(15) x gpu
merge_path; cpu merge_u32(15) x gpu gather_line; cpu seq_read(5) x gpu
gather_line; cpu gather_line(5 | 10) x gpu gather_line. GPU side always 1 GiB.
The hybrid phase builds exactly the footprints/threads the table needs
(capped at the largest `--footprints` value, so `--quick` pairs at 64 MiB);
requests are matched to the nearest available footprint/thread count.

## Results

Full suite, 2026-10-23, Apple M5 Pro (16-core GPU; hw.ncpu = 15 CPU cores, 5P + 10E),
24 GB unified memory, macOS 27, `all --reps 9 --seconds 2.0`, default footprints
(4 MiB, 64 MiB, 1 GiB, 4 GiB), threads 1/5/10/15, 32M probes, nothing else
running. Raw data in `results/{cpu,gpu,hybrid}-2026-10-23.json`.

### Headline numbers

* GPU streaming: seq_read 276 GB/s, seq_write 238 GB/s, seq_copy 254 GB/s at
  1-4 GiB (LPDDR5X spec 273 GB/s). 64 MiB rows are clock-ramp / launch
  dominated (0.5-0.7 ms per run); 4 MiB rows are pure overhead.
* GPU random 128 B lines from DRAM: 0.69 ns/line at 1 GiB, 0.77 ns/line at
  4 GiB (1.3-1.45 G lines/s, i.e. the same ~165-190 GB/s of line traffic for
  gather, coalesced gather and scatter); atomic RMW 1.35-1.51 ns/line.
  CPU, 15 threads, 4 GiB: gather 0.70 ns/line, scatter 0.99, RMW 3.48.
* Per command buffer: `cmdbuf_empty` 18 us; first dispatch in a command buffer
  128 us wall (2 us GPU); each further dispatch 0.5 us in the same encoder,
  0.6 us with one encoder per dispatch. Expect wall = gpu_ms + ~0.15-0.2 ms.
* CPU<->GPU sync: shared-event ping-pong 1.8 us/round with a spinning CPU
  thread, 13.1 us/round through `MTLSharedEventListener` notify blocks.
* Decoupled look-back scan makes forward progress at every footprint
  (`forward_progress=true`, correct) and is 1.35x faster than reduce-then-scan
  at 1 GiB (9.1 ms vs 12.3 ms; 235 vs 262 nominal GB/s because it moves 2N
  instead of 3N words).
* GPU partition_pass (8-bit radix, key+payload): 0.12 ns/pair (8.6 G pairs/s)
  at 1 GiB. GPU merge_path 0.04 ns/element at 1 GiB vs CPU merge_u32 0.14
  (15 threads). GPU hist_scan 2^16 bins 0.08 ns/key vs CPU 15 threads 0.05.
* Hybrid (1 GiB): CPU seq_read(15) + GPU seq_read share bandwidth (1.66x /
  1.98x slowdown, 134 + 138 = 272 GB/s combined = DRAM peak). GPU DRAM-latency
  gathers are barely disturbed by a 5- or 10-thread CPU partner (1.00-1.02x)
  and slow the CPU side 1.12-1.22x; a 15-thread CPU gather slows GPU seq_copy
  1.48x / merge_path 1.73x. hashbrown_probe(15, 1 GiB) next to GPU
  partition_pass: 1.67x / 1.32x; next to merge_path 1.53x / 1.54x.
* `cpu_pool_dispatch` 15 threads = 56 us (>= 16 threads oversubscribe the
  15 cores; an earlier run with 18 threads hit a 2 ms scheduler floor).

### GPU

| op | proc | thr | footprint | GB/s | ns/item | median ms | gpu ms | notes |
|---|---|---:|---:|---:|---:|---:|---:|---|
| seq_read | gpu | 1 | 4 MiB | 21.4 | 0.19 | 0.196 | 0.038 |  |
| seq_write | gpu | 1 | 4 MiB | 21.2 | 0.19 | 0.198 | 0.027 |  |
| seq_copy | gpu | 1 | 4 MiB | 33.1 | 0.24 | 0.254 | 0.055 |  |
| gather_line | gpu | 1 | 4 MiB | 2152.1 | 0.06 | 1.996 | 1.811 |  |
| gather_coop | gpu | 1 | 4 MiB | 838.0 | 0.15 | 5.125 | 4.912 |  |
| scatter_line | gpu | 1 | 4 MiB | 2041.8 | 0.06 | 2.104 | 2.016 |  |
| rmw_atomic | gpu | 1 | 4 MiB | 3568.6 | 0.07 | 2.407 | 2.199 |  |
| hist_scan | gpu | 1 | 4 MiB | 24.5 | 0.16 | 0.171 | 0.032 | bins=256 correct=true keys_per_tg=16384 |
| hist_scan | gpu | 1 | 4 MiB | 23.0 | 0.22 | 0.228 | 0.095 | bins=65536 correct=true keys_per_tg=16384 |
| partition_pass | gpu | 1 | 4 MiB | 87.3 | 0.21 | 0.222 | 0.073 | correct=true radix_bits=8 stable=false tiles=512 |
| merge_path | gpu | 1 | 4 MiB | 67.3 | 0.18 | 0.187 | 0.037 | correct=true |
| scan_rts | gpu | 1 | 4 MiB | 67.5 | 0.18 | 0.186 | 0.038 | correct=true |
| lookback_scan | gpu | 1 | 4 MiB | 53.5 | 0.15 | 0.157 | 0.031 | correct=true forward_progress=true tiles=512 watchdog_cap=32768 |
| seq_read | gpu | 1 | 64 MiB | 153.1 | 0.03 | 0.438 | 0.267 |  |
| seq_write | gpu | 1 | 64 MiB | 183.4 | 0.02 | 0.366 | 0.240 |  |
| seq_copy | gpu | 1 | 64 MiB | 200.1 | 0.04 | 0.671 | 0.515 |  |
| gather_line | gpu | 1 | 64 MiB | 517.7 | 0.25 | 8.296 | 8.322 |  |
| gather_coop | gpu | 1 | 64 MiB | 372.3 | 0.34 | 11.537 | 11.345 |  |
| scatter_line | gpu | 1 | 64 MiB | 303.4 | 0.42 | 14.154 | 13.942 |  |
| rmw_atomic | gpu | 1 | 64 MiB | 475.2 | 0.54 | 18.075 | 17.905 |  |
| hist_scan | gpu | 1 | 64 MiB | 117.1 | 0.03 | 0.573 | 0.404 | bins=256 correct=true keys_per_tg=32768 |
| hist_scan | gpu | 1 | 64 MiB | 41.8 | 0.10 | 1.630 | 1.426 | bins=65536 correct=true keys_per_tg=32768 |
| partition_pass | gpu | 1 | 64 MiB | 157.2 | 0.12 | 1.974 | 1.769 | correct=true radix_bits=8 stable=false tiles=8192 |
| merge_path | gpu | 1 | 64 MiB | 256.9 | 0.05 | 0.784 | 0.584 | correct=true |
| scan_rts | gpu | 1 | 64 MiB | 211.9 | 0.06 | 0.950 | 0.743 | correct=true |
| lookback_scan | gpu | 1 | 64 MiB | 181.2 | 0.04 | 0.741 | 0.532 | correct=true forward_progress=true tiles=8192 watchdog_cap=32768 |
| seq_read | gpu | 1 | 1 GiB | 275.9 | 0.01 | 3.892 | 3.707 |  |
| seq_write | gpu | 1 | 1 GiB | 237.5 | 0.02 | 4.522 | 4.336 |  |
| seq_copy | gpu | 1 | 1 GiB | 254.3 | 0.03 | 8.445 | 8.287 |  |
| gather_line | gpu | 1 | 1 GiB | 186.3 | 0.69 | 23.057 | 22.892 |  |
| gather_coop | gpu | 1 | 1 GiB | 186.8 | 0.69 | 22.996 | 22.840 |  |
| scatter_line | gpu | 1 | 1 GiB | 185.4 | 0.69 | 23.164 | 22.933 |  |
| rmw_atomic | gpu | 1 | 1 GiB | 189.4 | 1.35 | 45.357 | 45.164 |  |
| hist_scan | gpu | 1 | 1 GiB | 163.0 | 0.02 | 6.588 | 6.389 | bins=256 correct=true keys_per_tg=262144 |
| hist_scan | gpu | 1 | 1 GiB | 49.2 | 0.08 | 21.838 | 21.526 | bins=65536 correct=true keys_per_tg=65280 |
| partition_pass | gpu | 1 | 1 GiB | 160.0 | 0.12 | 31.045 | 30.831 | correct=true radix_bits=8 stable=false tiles=131072 |
| merge_path | gpu | 1 | 1 GiB | 302.0 | 0.04 | 10.667 | 10.481 | correct=true |
| scan_rts | gpu | 1 | 1 GiB | 262.1 | 0.05 | 12.289 | 11.992 | correct=true |
| lookback_scan | gpu | 1 | 1 GiB | 235.2 | 0.03 | 9.131 | 8.956 | correct=true forward_progress=true tiles=131072 watchdog_cap=32768 |
| seq_read | gpu | 1 | 4 GiB | 274.8 | 0.01 | 15.631 | 15.512 |  |
| seq_write | gpu | 1 | 4 GiB | 238.7 | 0.02 | 17.992 | 17.775 |  |
| seq_copy | gpu | 1 | 4 GiB | 254.9 | 0.03 | 33.706 | 33.511 |  |
| gather_line | gpu | 1 | 4 GiB | 166.3 | 0.77 | 25.823 | 25.608 |  |
| gather_coop | gpu | 1 | 4 GiB | 167.4 | 0.76 | 25.657 | 25.443 |  |
| scatter_line | gpu | 1 | 4 GiB | 166.2 | 0.77 | 25.839 | 25.628 |  |
| rmw_atomic | gpu | 1 | 4 GiB | 169.4 | 1.51 | 50.712 | 50.486 |  |
| hist_scan | gpu | 1 | 4 GiB | 173.2 | 0.02 | 24.792 | 24.597 | bins=256 correct=true keys_per_tg=262144 |
| hist_scan | gpu | 1 | 4 GiB | 50.5 | 0.08 | 85.074 | 84.869 | bins=65536 correct=true keys_per_tg=65280 |
| rmw_atomic_contended | gpu | 1 | 128 KiB | 5657.6 | 0.05 | 1.518 | 1.320 | lines=1024 |
| cmdbuf_empty | gpu | 1 | - | 0.0 | 17542.00 | 0.018 | 0.000 |  |
| dispatch_empty | gpu | 1 | - | 0.0 | 127917.00 | 0.128 | 0.002 | dispatches=1 |
| dispatch_empty | gpu | 1 | - | 0.0 | 1775.00 | 0.177 | 0.046 | dispatches=100 encoders=1 |
| dispatch_empty_enc | gpu | 1 | - | 0.0 | 1870.42 | 0.187 | 0.046 | dispatches=100 encoders=100 |
| sync_roundtrip | gpu | 1 | - | 0.0 | 1792.08 | 0.179 | 0.000 | cpu_side=spin on signaledValue rounds=100 |
| sync_roundtrip_notify | gpu | 1 | - | 0.0 | 13113.75 | 1.311 | 0.000 | cpu_side=MTLSharedEventListener notify block rounds=100 |

### Hybrid

`slowdown` = paired median / solo median; `partner` = processor:op:footprint.

| op | proc | thr | footprint | GB/s | ns/item | median ms | gpu ms | notes |
|---|---|---:|---:|---:|---:|---:|---:|---|
| seq_read | cpu | 15 | 1 GiB | 134.1 | 0.06 | 8.007 | - | elem_bytes=8 n_items=134217728 partner=gpu:seq_read:1073741824 partner_threads=1 slowdown=1.655 solo_gbps=221.958 solo_median_ms=4.838 unroll=8 |
| seq_read | gpu | 1 | 1 GiB | 138.3 | 0.03 | 7.766 | 5.349 | partner=cpu:seq_read:1073741824 partner_threads=15 slowdown=1.982 solo_gbps=274.019 solo_median_ms=3.919 |
| gather_line | cpu | 15 | 1 GiB | 84.2 | 1.52 | 12.755 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=8388608 partner=gpu:seq_copy:1073741824 partner_threads=1 slowdown=1.672 solo_gbps=140.769 solo_median_ms=7.628 |
| seq_copy | gpu | 1 | 1 GiB | 171.8 | 0.05 | 12.497 | 11.878 | partner=cpu:gather_line:1073741824 partner_threads=15 slowdown=1.484 solo_gbps=255.015 solo_median_ms=8.421 |
| gather_line | cpu | 15 | 64 MiB | 128.4 | 1.00 | 0.523 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=524288 partner=gpu:seq_copy:1073741824 partner_threads=1 slowdown=2.047 solo_gbps=262.829 solo_median_ms=0.255 |
| seq_copy | gpu | 1 | 1 GiB | 185.4 | 0.04 | 11.585 | 11.296 | partner=cpu:gather_line:67108864 partner_threads=15 slowdown=1.360 solo_gbps=252.045 solo_median_ms=8.520 |
| gather_line | cpu | 15 | 1 GiB | 105.5 | 1.21 | 10.180 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=8388608 partner=gpu:merge_path:1073741824 partner_threads=1 slowdown=1.780 solo_gbps=187.718 solo_median_ms=5.720 |
| merge_path | gpu | 1 | 1 GiB | 175.0 | 0.07 | 18.411 | 16.408 | correct=true partner=cpu:gather_line:1073741824 partner_threads=15 slowdown=1.730 solo_gbps=302.674 solo_median_ms=10.643 |
| hashbrown_probe | cpu | 15 | 64 MiB | 89.3 | 1.43 | 3.007 | - | hit_rate=0.500 key_bytes=8 n_items=2097152 n_keys=2097152 partner=gpu:partition_pass:1073741824 partner_threads=1 slowdown=1.455 solo_gbps=129.906 solo_median_ms=2.066 val_bytes=4 |
| partition_pass | gpu | 1 | 1 GiB | 141.3 | 0.13 | 35.145 | 34.413 | correct=true partner=cpu:hashbrown_probe:67108864 partner_threads=15 radix_bits=8 slowdown=1.152 solo_gbps=162.745 solo_median_ms=30.514 stable=false tiles=131072 |
| hashbrown_probe | cpu | 15 | 1 GiB | 56.3 | 2.27 | 38.165 | - | hit_rate=0.500 key_bytes=8 n_items=16777216 n_keys=33554432 partner=gpu:partition_pass:1073741824 partner_threads=1 slowdown=1.670 solo_gbps=93.940 solo_median_ms=22.860 val_bytes=4 |
| partition_pass | gpu | 1 | 1 GiB | 123.4 | 0.15 | 40.246 | 39.296 | correct=true partner=cpu:hashbrown_probe:1073741824 partner_threads=15 radix_bits=8 slowdown=1.315 solo_gbps=162.216 solo_median_ms=30.614 stable=false tiles=131072 |
| hashbrown_probe | cpu | 15 | 1 GiB | 61.7 | 2.08 | 34.819 | - | hit_rate=0.500 key_bytes=8 n_items=16777216 n_keys=33554432 partner=gpu:merge_path:1073741824 partner_threads=1 slowdown=1.533 solo_gbps=94.569 solo_median_ms=22.708 val_bytes=4 |
| merge_path | gpu | 1 | 1 GiB | 197.3 | 0.06 | 16.325 | 15.255 | correct=true partner=cpu:hashbrown_probe:1073741824 partner_threads=15 slowdown=1.539 solo_gbps=303.717 solo_median_ms=10.606 |
| merge_u32 | cpu | 15 | 1 GiB | 83.3 | 0.14 | 38.677 | - | elem_bytes=4 n_items=268435456 partner=gpu:gather_line:1073741824 partner_threads=1 slowdown=1.059 solo_gbps=88.183 solo_median_ms=36.529 split=merge-path |
| gather_line | gpu | 1 | 1 GiB | 184.3 | 0.69 | 23.304 | 22.915 | partner=cpu:merge_u32:1073741824 partner_threads=15 slowdown=1.006 solo_gbps=185.417 solo_median_ms=23.164 |
| seq_read | cpu | 5 | 1 GiB | 168.6 | 0.05 | 6.370 | - | elem_bytes=8 n_items=134217728 partner=gpu:gather_line:1073741824 partner_threads=1 slowdown=0.889 solo_gbps=149.913 solo_median_ms=7.162 unroll=8 |
| gather_line | gpu | 1 | 1 GiB | 184.8 | 0.69 | 23.237 | 22.987 | partner=cpu:seq_read:1073741824 partner_threads=5 slowdown=1.004 solo_gbps=185.645 solo_median_ms=23.135 |
| gather_line | cpu | 5 | 1 GiB | 85.1 | 1.50 | 12.621 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=8388608 partner=gpu:gather_line:1073741824 partner_threads=1 slowdown=1.118 solo_gbps=95.097 solo_median_ms=11.291 |
| gather_line | gpu | 1 | 1 GiB | 184.9 | 0.69 | 23.227 | 23.001 | partner=cpu:gather_line:1073741824 partner_threads=5 slowdown=1.014 solo_gbps=187.543 solo_median_ms=22.901 |
| gather_line | cpu | 10 | 1 GiB | 139.1 | 0.92 | 7.721 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=8388608 partner=gpu:gather_line:1073741824 partner_threads=1 slowdown=1.218 solo_gbps=169.416 solo_median_ms=6.338 |
| gather_line | gpu | 1 | 1 GiB | 181.2 | 0.71 | 23.707 | 23.084 | partner=cpu:gather_line:1073741824 partner_threads=10 slowdown=1.024 solo_gbps=185.463 solo_median_ms=23.158 |

### CPU

| op | proc | thr | footprint | GB/s | ns/item | median ms | gpu ms | notes |
|---|---|---:|---:|---:|---:|---:|---:|---|
| cpu_spawn | cpu | 1 | - | 0.0 | 27917.00 | 0.028 | - |  |
| cpu_spawn | cpu | 5 | - | 0.0 | 14158.40 | 0.071 | - |  |
| cpu_spawn | cpu | 10 | - | 0.0 | 12629.10 | 0.126 | - |  |
| cpu_spawn | cpu | 15 | - | 0.0 | 11800.00 | 0.177 | - |  |
| cpu_pool_dispatch | cpu | 5 | - | 0.0 | 750.00 | 0.001 | - |  |
| cpu_pool_dispatch | cpu | 10 | - | 0.0 | 1542.00 | 0.002 | - |  |
| cpu_pool_dispatch | cpu | 15 | - | 0.0 | 56125.00 | 0.056 | - |  |
| seq_read | cpu | 1 | 4 MiB | 86.2 | 0.09 | 0.049 | - | elem_bytes=8 n_items=524288 unroll=8 |
| seq_read | cpu | 5 | 4 MiB | 449.4 | 0.02 | 0.009 | - | elem_bytes=8 n_items=524288 unroll=8 |
| seq_read | cpu | 10 | 4 MiB | 58.3 | 0.14 | 0.072 | - | elem_bytes=8 n_items=524288 unroll=8 |
| seq_read | cpu | 15 | 4 MiB | 968.0 | 0.01 | 0.004 | - | elem_bytes=8 n_items=524288 unroll=8 |
| seq_write | cpu | 1 | 4 MiB | 99.9 | 0.08 | 0.042 | - | elem_bytes=8 n_items=524288 |
| seq_write | cpu | 5 | 4 MiB | 208.8 | 0.04 | 0.020 | - | elem_bytes=8 n_items=524288 |
| seq_write | cpu | 10 | 4 MiB | 57.7 | 0.14 | 0.073 | - | elem_bytes=8 n_items=524288 |
| seq_write | cpu | 15 | 4 MiB | 57.3 | 0.14 | 0.073 | - | elem_bytes=8 n_items=524288 |
| seq_copy | cpu | 1 | 4 MiB | 122.2 | 0.13 | 0.069 | - | elem_bytes=8 n_items=524288 |
| seq_copy | cpu | 5 | 4 MiB | 358.9 | 0.04 | 0.023 | - | elem_bytes=8 n_items=524288 |
| seq_copy | cpu | 10 | 4 MiB | 472.6 | 0.03 | 0.018 | - | elem_bytes=8 n_items=524288 |
| seq_copy | cpu | 15 | 4 MiB | 230.4 | 0.07 | 0.036 | - | elem_bytes=8 n_items=524288 |
| gather_line | cpu | 1 | 4 MiB | 116.5 | 1.10 | 0.036 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=32768 |
| gather_line | cpu | 5 | 4 MiB | 786.5 | 0.16 | 0.005 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=32768 |
| gather_line | cpu | 10 | 4 MiB | 1258.4 | 0.10 | 0.003 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=32768 |
| gather_line | cpu | 15 | 4 MiB | 92.9 | 1.38 | 0.045 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=32768 |
| scatter_line | cpu | 1 | 4 MiB | 98.2 | 1.30 | 0.043 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=32768 |
| scatter_line | cpu | 5 | 4 MiB | 387.2 | 0.33 | 0.011 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=32768 |
| scatter_line | cpu | 10 | 4 MiB | 495.9 | 0.26 | 0.008 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=32768 |
| scatter_line | cpu | 15 | 4 MiB | 56.6 | 2.26 | 0.074 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=32768 |
| rmw_atomic | cpu | 1 | 4 MiB | 185.6 | 1.38 | 0.045 | - | bytes_per_item=256 elem_bytes=4 line_bytes=128 n_items=32768 |
| rmw_atomic | cpu | 5 | 4 MiB | 65.0 | 3.94 | 0.129 | - | bytes_per_item=256 elem_bytes=4 line_bytes=128 n_items=32768 |
| rmw_atomic | cpu | 10 | 4 MiB | 37.2 | 6.88 | 0.225 | - | bytes_per_item=256 elem_bytes=4 line_bytes=128 n_items=32768 |
| rmw_atomic | cpu | 15 | 4 MiB | 44.5 | 5.75 | 0.188 | - | bytes_per_item=256 elem_bytes=4 line_bytes=128 n_items=32768 |
| gather_dep | cpu | 1 | 4 MiB | 16.7 | 7.66 | 32.125 | - | line_bytes=128 n_items=4194304 steps_per_thread=4194304 |
| gather_dep | cpu | 5 | 4 MiB | 64.7 | 1.98 | 8.296 | - | line_bytes=128 n_items=4194304 steps_per_thread=838860 |
| gather_dep | cpu | 10 | 4 MiB | 153.2 | 0.84 | 3.505 | - | line_bytes=128 n_items=4194304 steps_per_thread=419430 |
| gather_dep | cpu | 15 | 4 MiB | 228.1 | 0.56 | 2.354 | - | line_bytes=128 n_items=4194304 steps_per_thread=279620 |
| hashbrown_build | cpu | 1 | 4 MiB | 11.4 | 2.80 | 0.367 | - | key_bytes=8 n_items=131072 n_keys=131072 prealloc=true val_bytes=4 |
| hashbrown_probe | cpu | 1 | 4 MiB | 26.0 | 4.92 | 0.645 | - | hit_rate=0.500 key_bytes=8 n_items=131072 n_keys=131072 val_bytes=4 |
| hashbrown_probe | cpu | 5 | 4 MiB | 297.4 | 0.43 | 0.056 | - | hit_rate=0.500 key_bytes=8 n_items=131072 n_keys=131072 val_bytes=4 |
| hashbrown_probe | cpu | 10 | 4 MiB | 166.4 | 0.77 | 0.101 | - | hit_rate=0.500 key_bytes=8 n_items=131072 n_keys=131072 val_bytes=4 |
| hashbrown_probe | cpu | 15 | 4 MiB | 181.9 | 0.70 | 0.092 | - | hit_rate=0.500 key_bytes=8 n_items=131072 n_keys=131072 val_bytes=4 |
| hist_scan | cpu | 1 | 4 MiB | 7.2 | 0.55 | 0.579 | - | bins=65536 elem_bytes=4 n_items=1048576 |
| hist_scan | cpu | 5 | 4 MiB | 27.0 | 0.15 | 0.155 | - | bins=65536 elem_bytes=4 n_items=1048576 |
| hist_scan | cpu | 10 | 4 MiB | 20.9 | 0.19 | 0.200 | - | bins=65536 elem_bytes=4 n_items=1048576 |
| hist_scan | cpu | 15 | 4 MiB | 19.2 | 0.21 | 0.218 | - | bins=65536 elem_bytes=4 n_items=1048576 |
| seq_read | cpu | 1 | 64 MiB | 90.1 | 0.09 | 0.745 | - | elem_bytes=8 n_items=8388608 unroll=8 |
| seq_read | cpu | 5 | 64 MiB | 174.3 | 0.05 | 0.385 | - | elem_bytes=8 n_items=8388608 unroll=8 |
| seq_read | cpu | 10 | 64 MiB | 224.5 | 0.04 | 0.299 | - | elem_bytes=8 n_items=8388608 unroll=8 |
| seq_read | cpu | 15 | 64 MiB | 269.2 | 0.03 | 0.249 | - | elem_bytes=8 n_items=8388608 unroll=8 |
| seq_write | cpu | 1 | 64 MiB | 135.1 | 0.06 | 0.497 | - | elem_bytes=8 n_items=8388608 |
| seq_write | cpu | 5 | 64 MiB | 258.2 | 0.03 | 0.260 | - | elem_bytes=8 n_items=8388608 |
| seq_write | cpu | 10 | 64 MiB | 236.7 | 0.03 | 0.284 | - | elem_bytes=8 n_items=8388608 |
| seq_write | cpu | 15 | 64 MiB | 245.8 | 0.03 | 0.273 | - | elem_bytes=8 n_items=8388608 |
| seq_copy | cpu | 1 | 64 MiB | 135.6 | 0.12 | 0.990 | - | elem_bytes=8 n_items=8388608 |
| seq_copy | cpu | 5 | 64 MiB | 178.3 | 0.09 | 0.753 | - | elem_bytes=8 n_items=8388608 |
| seq_copy | cpu | 10 | 64 MiB | 244.1 | 0.07 | 0.550 | - | elem_bytes=8 n_items=8388608 |
| seq_copy | cpu | 15 | 64 MiB | 270.0 | 0.06 | 0.497 | - | elem_bytes=8 n_items=8388608 |
| gather_line | cpu | 1 | 64 MiB | 39.7 | 3.22 | 1.690 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=524288 |
| gather_line | cpu | 5 | 64 MiB | 184.5 | 0.69 | 0.364 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=524288 |
| gather_line | cpu | 10 | 64 MiB | 290.9 | 0.44 | 0.231 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=524288 |
| gather_line | cpu | 15 | 64 MiB | 255.7 | 0.50 | 0.262 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=524288 |
| scatter_line | cpu | 1 | 64 MiB | 25.9 | 4.94 | 2.591 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=524288 |
| scatter_line | cpu | 5 | 64 MiB | 107.6 | 1.19 | 0.624 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=524288 |
| scatter_line | cpu | 10 | 64 MiB | 192.9 | 0.66 | 0.348 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=524288 |
| scatter_line | cpu | 15 | 64 MiB | 278.8 | 0.46 | 0.241 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=524288 |
| rmw_atomic | cpu | 1 | 64 MiB | 23.5 | 10.90 | 5.715 | - | bytes_per_item=256 elem_bytes=4 line_bytes=128 n_items=524288 |
| rmw_atomic | cpu | 5 | 64 MiB | 57.3 | 4.47 | 2.343 | - | bytes_per_item=256 elem_bytes=4 line_bytes=128 n_items=524288 |
| rmw_atomic | cpu | 10 | 64 MiB | 50.9 | 5.03 | 2.635 | - | bytes_per_item=256 elem_bytes=4 line_bytes=128 n_items=524288 |
| rmw_atomic | cpu | 15 | 64 MiB | 66.4 | 3.85 | 2.020 | - | bytes_per_item=256 elem_bytes=4 line_bytes=128 n_items=524288 |
| gather_dep | cpu | 1 | 64 MiB | 1.2 | 105.06 | 440.672 | - | line_bytes=128 n_items=4194304 steps_per_thread=4194304 |
| gather_dep | cpu | 5 | 64 MiB | 6.2 | 20.55 | 86.205 | - | line_bytes=128 n_items=4194304 steps_per_thread=838860 |
| gather_dep | cpu | 10 | 64 MiB | 12.3 | 10.41 | 43.663 | - | line_bytes=128 n_items=4194304 steps_per_thread=419430 |
| gather_dep | cpu | 15 | 64 MiB | 18.2 | 7.05 | 29.575 | - | line_bytes=128 n_items=4194304 steps_per_thread=279620 |
| hashbrown_build | cpu | 1 | 64 MiB | 5.2 | 6.12 | 12.838 | - | key_bytes=8 n_items=2097152 n_keys=2097152 prealloc=true val_bytes=4 |
| hashbrown_probe | cpu | 1 | 64 MiB | 18.1 | 7.07 | 14.832 | - | hit_rate=0.500 key_bytes=8 n_items=2097152 n_keys=2097152 val_bytes=4 |
| hashbrown_probe | cpu | 5 | 64 MiB | 88.0 | 1.45 | 3.051 | - | hit_rate=0.500 key_bytes=8 n_items=2097152 n_keys=2097152 val_bytes=4 |
| hashbrown_probe | cpu | 10 | 64 MiB | 126.2 | 1.01 | 2.128 | - | hit_rate=0.500 key_bytes=8 n_items=2097152 n_keys=2097152 val_bytes=4 |
| hashbrown_probe | cpu | 15 | 64 MiB | 160.5 | 0.80 | 1.673 | - | hit_rate=0.500 key_bytes=8 n_items=2097152 n_keys=2097152 val_bytes=4 |
| sort_u32 | cpu | 1 | 64 MiB | 0.8 | 9.80 | 164.364 | - | algo=sort_unstable elem_bytes=4 includes_input_copy=true n_items=16777216 parallel=false |
| sort_u32 | cpu | 5 | 64 MiB | 2.8 | 2.82 | 47.328 | - | algo=chunk sort_unstable + merge-path tree elem_bytes=4 includes_input_copy=true n_items=16777216 parallel=true |
| sort_u32 | cpu | 10 | 64 MiB | 4.9 | 1.64 | 27.588 | - | algo=chunk sort_unstable + merge-path tree elem_bytes=4 includes_input_copy=true n_items=16777216 parallel=true |
| sort_u32 | cpu | 15 | 64 MiB | 5.6 | 1.42 | 23.891 | - | algo=chunk sort_unstable + merge-path tree elem_bytes=4 includes_input_copy=true n_items=16777216 parallel=true |
| merge_u32 | cpu | 1 | 64 MiB | 7.0 | 1.71 | 28.642 | - | elem_bytes=4 n_items=16777216 split=merge-path |
| merge_u32 | cpu | 5 | 64 MiB | 33.4 | 0.36 | 6.021 | - | elem_bytes=4 n_items=16777216 split=merge-path |
| merge_u32 | cpu | 10 | 64 MiB | 66.9 | 0.18 | 3.009 | - | elem_bytes=4 n_items=16777216 split=merge-path |
| merge_u32 | cpu | 15 | 64 MiB | 86.6 | 0.14 | 2.325 | - | elem_bytes=4 n_items=16777216 split=merge-path |
| hist_scan | cpu | 1 | 64 MiB | 8.3 | 0.48 | 8.104 | - | bins=65536 elem_bytes=4 n_items=16777216 |
| hist_scan | cpu | 5 | 64 MiB | 38.3 | 0.10 | 1.754 | - | bins=65536 elem_bytes=4 n_items=16777216 |
| hist_scan | cpu | 10 | 64 MiB | 49.3 | 0.08 | 1.362 | - | bins=65536 elem_bytes=4 n_items=16777216 |
| hist_scan | cpu | 15 | 64 MiB | 69.5 | 0.06 | 0.966 | - | bins=65536 elem_bytes=4 n_items=16777216 |
| seq_read | cpu | 1 | 1 GiB | 85.7 | 0.09 | 12.526 | - | elem_bytes=8 n_items=134217728 unroll=8 |
| seq_read | cpu | 5 | 1 GiB | 155.9 | 0.05 | 6.886 | - | elem_bytes=8 n_items=134217728 unroll=8 |
| seq_read | cpu | 10 | 1 GiB | 217.3 | 0.04 | 4.942 | - | elem_bytes=8 n_items=134217728 unroll=8 |
| seq_read | cpu | 15 | 1 GiB | 240.2 | 0.03 | 4.470 | - | elem_bytes=8 n_items=134217728 unroll=8 |
| seq_write | cpu | 1 | 1 GiB | 135.0 | 0.06 | 7.951 | - | elem_bytes=8 n_items=134217728 |
| seq_write | cpu | 5 | 1 GiB | 223.7 | 0.04 | 4.800 | - | elem_bytes=8 n_items=134217728 |
| seq_write | cpu | 10 | 1 GiB | 203.8 | 0.04 | 5.268 | - | elem_bytes=8 n_items=134217728 |
| seq_write | cpu | 15 | 1 GiB | 228.8 | 0.03 | 4.693 | - | elem_bytes=8 n_items=134217728 |
| seq_copy | cpu | 1 | 1 GiB | 130.3 | 0.12 | 16.475 | - | elem_bytes=8 n_items=134217728 |
| seq_copy | cpu | 5 | 1 GiB | 182.5 | 0.09 | 11.767 | - | elem_bytes=8 n_items=134217728 |
| seq_copy | cpu | 10 | 1 GiB | 230.0 | 0.07 | 9.336 | - | elem_bytes=8 n_items=134217728 |
| seq_copy | cpu | 15 | 1 GiB | 254.8 | 0.06 | 8.427 | - | elem_bytes=8 n_items=134217728 |
| gather_line | cpu | 1 | 1 GiB | 21.0 | 6.11 | 51.223 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=8388608 |
| gather_line | cpu | 5 | 1 GiB | 96.2 | 1.33 | 11.160 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=8388608 |
| gather_line | cpu | 10 | 1 GiB | 171.6 | 0.75 | 6.258 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=8388608 |
| gather_line | cpu | 15 | 1 GiB | 197.4 | 0.65 | 5.441 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=8388608 |
| scatter_line | cpu | 1 | 1 GiB | 19.3 | 6.65 | 55.769 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=8388608 |
| scatter_line | cpu | 5 | 1 GiB | 78.4 | 1.63 | 13.701 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=8388608 |
| scatter_line | cpu | 10 | 1 GiB | 120.5 | 1.06 | 8.911 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=8388608 |
| scatter_line | cpu | 15 | 1 GiB | 115.0 | 1.11 | 9.335 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=8388608 |
| rmw_atomic | cpu | 1 | 1 GiB | 20.9 | 12.25 | 102.799 | - | bytes_per_item=256 elem_bytes=4 line_bytes=128 n_items=8388608 |
| rmw_atomic | cpu | 5 | 1 GiB | 44.8 | 5.72 | 47.954 | - | bytes_per_item=256 elem_bytes=4 line_bytes=128 n_items=8388608 |
| rmw_atomic | cpu | 10 | 1 GiB | 77.7 | 3.30 | 27.643 | - | bytes_per_item=256 elem_bytes=4 line_bytes=128 n_items=8388608 |
| rmw_atomic | cpu | 15 | 1 GiB | 117.1 | 2.19 | 18.341 | - | bytes_per_item=256 elem_bytes=4 line_bytes=128 n_items=8388608 |
| gather_dep | cpu | 1 | 1 GiB | 1.1 | 114.13 | 478.708 | - | line_bytes=128 n_items=4194304 steps_per_thread=4194304 |
| gather_dep | cpu | 5 | 1 GiB | 5.5 | 23.45 | 98.347 | - | line_bytes=128 n_items=4194304 steps_per_thread=838860 |
| gather_dep | cpu | 10 | 1 GiB | 10.5 | 12.16 | 51.011 | - | line_bytes=128 n_items=4194304 steps_per_thread=419430 |
| gather_dep | cpu | 15 | 1 GiB | 14.2 | 8.99 | 37.713 | - | line_bytes=128 n_items=4194304 steps_per_thread=279620 |
| hashbrown_build | cpu | 1 | 1 GiB | 2.0 | 16.12 | 540.757 | - | key_bytes=8 n_items=33554432 n_keys=33554432 prealloc=true val_bytes=4 |
| hashbrown_probe | cpu | 1 | 1 GiB | 11.2 | 11.41 | 191.424 | - | hit_rate=0.500 key_bytes=8 n_items=16777216 n_keys=33554432 val_bytes=4 |
| hashbrown_probe | cpu | 5 | 1 GiB | 37.1 | 3.45 | 57.930 | - | hit_rate=0.500 key_bytes=8 n_items=16777216 n_keys=33554432 val_bytes=4 |
| hashbrown_probe | cpu | 10 | 1 GiB | 69.7 | 1.84 | 30.819 | - | hit_rate=0.500 key_bytes=8 n_items=16777216 n_keys=33554432 val_bytes=4 |
| hashbrown_probe | cpu | 15 | 1 GiB | 87.5 | 1.46 | 24.549 | - | hit_rate=0.500 key_bytes=8 n_items=16777216 n_keys=33554432 val_bytes=4 |
| sort_u32 | cpu | 1 | 1 GiB | 0.7 | 11.10 | 2980.484 | - | algo=sort_unstable elem_bytes=4 includes_input_copy=true n_items=268435456 parallel=false |
| sort_u32 | cpu | 5 | 1 GiB | 2.5 | 3.21 | 861.232 | - | algo=chunk sort_unstable + merge-path tree elem_bytes=4 includes_input_copy=true n_items=268435456 parallel=true |
| sort_u32 | cpu | 10 | 1 GiB | 4.3 | 1.84 | 493.679 | - | algo=chunk sort_unstable + merge-path tree elem_bytes=4 includes_input_copy=true n_items=268435456 parallel=true |
| sort_u32 | cpu | 15 | 1 GiB | 5.5 | 1.45 | 388.451 | - | algo=chunk sort_unstable + merge-path tree elem_bytes=4 includes_input_copy=true n_items=268435456 parallel=true |
| merge_u32 | cpu | 1 | 1 GiB | 6.9 | 1.73 | 464.850 | - | elem_bytes=4 n_items=268435456 split=merge-path |
| merge_u32 | cpu | 5 | 1 GiB | 33.7 | 0.36 | 95.706 | - | elem_bytes=4 n_items=268435456 split=merge-path |
| merge_u32 | cpu | 10 | 1 GiB | 67.4 | 0.18 | 47.817 | - | elem_bytes=4 n_items=268435456 split=merge-path |
| merge_u32 | cpu | 15 | 1 GiB | 86.7 | 0.14 | 37.136 | - | elem_bytes=4 n_items=268435456 split=merge-path |
| hist_scan | cpu | 1 | 1 GiB | 8.3 | 0.48 | 129.091 | - | bins=65536 elem_bytes=4 n_items=268435456 |
| hist_scan | cpu | 5 | 1 GiB | 26.0 | 0.15 | 41.257 | - | bins=65536 elem_bytes=4 n_items=268435456 |
| hist_scan | cpu | 10 | 1 GiB | 54.3 | 0.07 | 19.789 | - | bins=65536 elem_bytes=4 n_items=268435456 |
| hist_scan | cpu | 15 | 1 GiB | 78.2 | 0.05 | 13.737 | - | bins=65536 elem_bytes=4 n_items=268435456 |
| seq_read | cpu | 1 | 4 GiB | 85.6 | 0.09 | 50.188 | - | elem_bytes=8 n_items=536870912 unroll=8 |
| seq_read | cpu | 5 | 4 GiB | 232.1 | 0.03 | 18.503 | - | elem_bytes=8 n_items=536870912 unroll=8 |
| seq_read | cpu | 10 | 4 GiB | 235.8 | 0.03 | 18.217 | - | elem_bytes=8 n_items=536870912 unroll=8 |
| seq_read | cpu | 15 | 4 GiB | 239.3 | 0.03 | 17.945 | - | elem_bytes=8 n_items=536870912 unroll=8 |
| seq_write | cpu | 1 | 4 GiB | 134.1 | 0.06 | 32.026 | - | elem_bytes=8 n_items=536870912 |
| seq_write | cpu | 5 | 4 GiB | 204.4 | 0.04 | 21.010 | - | elem_bytes=8 n_items=536870912 |
| seq_write | cpu | 10 | 4 GiB | 248.3 | 0.03 | 17.294 | - | elem_bytes=8 n_items=536870912 |
| seq_write | cpu | 15 | 4 GiB | 243.8 | 0.03 | 17.619 | - | elem_bytes=8 n_items=536870912 |
| seq_copy | cpu | 1 | 4 GiB | 129.9 | 0.12 | 66.108 | - | elem_bytes=8 n_items=536870912 |
| seq_copy | cpu | 5 | 4 GiB | 256.3 | 0.06 | 33.520 | - | elem_bytes=8 n_items=536870912 |
| seq_copy | cpu | 10 | 4 GiB | 256.4 | 0.06 | 33.503 | - | elem_bytes=8 n_items=536870912 |
| seq_copy | cpu | 15 | 4 GiB | 253.8 | 0.06 | 33.848 | - | elem_bytes=8 n_items=536870912 |
| gather_line | cpu | 1 | 4 GiB | 19.9 | 6.43 | 215.777 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=33554432 |
| gather_line | cpu | 5 | 4 GiB | 92.2 | 1.39 | 46.603 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=33554432 |
| gather_line | cpu | 10 | 4 GiB | 165.6 | 0.77 | 25.929 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=33554432 |
| gather_line | cpu | 15 | 4 GiB | 183.7 | 0.70 | 23.386 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=33554432 |
| scatter_line | cpu | 1 | 4 GiB | 18.5 | 6.90 | 231.624 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=33554432 |
| scatter_line | cpu | 5 | 4 GiB | 76.4 | 1.67 | 56.193 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=33554432 |
| scatter_line | cpu | 10 | 4 GiB | 117.5 | 1.09 | 36.564 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=33554432 |
| scatter_line | cpu | 15 | 4 GiB | 129.5 | 0.99 | 33.161 | - | bytes_per_item=128 elem_bytes=4 line_bytes=128 n_items=33554432 |
| rmw_atomic | cpu | 1 | 4 GiB | 9.6 | 26.58 | 891.945 | - | bytes_per_item=256 elem_bytes=4 line_bytes=128 n_items=33554432 |
| rmw_atomic | cpu | 5 | 4 GiB | 27.8 | 9.20 | 308.775 | - | bytes_per_item=256 elem_bytes=4 line_bytes=128 n_items=33554432 |
| rmw_atomic | cpu | 10 | 4 GiB | 51.4 | 4.98 | 167.008 | - | bytes_per_item=256 elem_bytes=4 line_bytes=128 n_items=33554432 |
| rmw_atomic | cpu | 15 | 4 GiB | 73.5 | 3.48 | 116.871 | - | bytes_per_item=256 elem_bytes=4 line_bytes=128 n_items=33554432 |
| gather_dep | cpu | 1 | 4 GiB | 1.1 | 119.82 | 502.573 | - | line_bytes=128 n_items=4194304 steps_per_thread=4194304 |
| gather_dep | cpu | 5 | 4 GiB | 4.3 | 29.72 | 124.644 | - | line_bytes=128 n_items=4194304 steps_per_thread=838860 |
| gather_dep | cpu | 10 | 4 GiB | 9.1 | 14.04 | 58.901 | - | line_bytes=128 n_items=4194304 steps_per_thread=419430 |
| gather_dep | cpu | 15 | 4 GiB | 13.0 | 9.87 | 41.378 | - | line_bytes=128 n_items=4194304 steps_per_thread=279620 |
| hashbrown_probe | cpu | 1 | 4 GiB | 10.6 | 12.10 | 203.038 | - | hit_rate=0.500 key_bytes=8 n_items=16777216 n_keys=134217728 val_bytes=4 |
| hashbrown_probe | cpu | 5 | 4 GiB | 34.0 | 3.77 | 63.174 | - | hit_rate=0.500 key_bytes=8 n_items=16777216 n_keys=134217728 val_bytes=4 |
| hashbrown_probe | cpu | 10 | 4 GiB | 64.1 | 2.00 | 33.518 | - | hit_rate=0.500 key_bytes=8 n_items=16777216 n_keys=134217728 val_bytes=4 |
| hashbrown_probe | cpu | 15 | 4 GiB | 76.0 | 1.68 | 28.240 | - | hit_rate=0.500 key_bytes=8 n_items=16777216 n_keys=134217728 val_bytes=4 |
| hist_scan | cpu | 1 | 4 GiB | 8.3 | 0.48 | 518.153 | - | bins=65536 elem_bytes=4 n_items=1073741824 |
| hist_scan | cpu | 5 | 4 GiB | 28.7 | 0.14 | 149.578 | - | bins=65536 elem_bytes=4 n_items=1073741824 |
| hist_scan | cpu | 10 | 4 GiB | 56.8 | 0.07 | 75.609 | - | bins=65536 elem_bytes=4 n_items=1073741824 |
| hist_scan | cpu | 15 | 4 GiB | 79.7 | 0.05 | 53.878 | - | bins=65536 elem_bytes=4 n_items=1073741824 |
