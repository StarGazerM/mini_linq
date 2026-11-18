# Packed-run read path

## Workloads and protocol

All three configurations retain the original admission paths. The replay executes the engine first, then either the experimental plain-storage or packed-storage set in the same process, with an independent host oracle for each workload. B=256 and the 32,768-row plain cutoff stay unchanged.

- `prefix_sorted`: every distinct leading-column candidate value, sorted, as before.
- `prefix_shuffled`: exactly those probes, Fisher–Yates shuffled with fixed xorshift seed `0xd1b54a32d192ed03`.
- `prefix_mixed`: shuffled present prefixes plus one absent value per nonempty gap and an absent value below/above the extent when representable. Per-state reports record the available below/between/above counts; a domain-boundary state cannot have a representable probe beyond that boundary.
- `range_scan`: shuffled present prefixes; find each interval and read every row. Sum a lane-weighted wrapping checksum of non-prefix bits; scalar/no-suffix states checksum the whole key so that the scan really reads rows. The host computes the same checksum independently of run layout.
- `membership`: up to 65,536 evenly strided present full keys per state, plus each sampled key's successor when absent and representable, shuffled deterministically. The device performs a single lower-bound plus equality test (engine uses its existing membership primitive).

Each workload validates one warmup against the sorted host set, then performs ten timed repetitions. Input H2D and oracle D2H are outside timing. Output allocation, descriptor preparation, launches, and completion synchronization are inside timing. Reports include counts, ns, lookups/s, rows scanned/s and all per-state results; `largest_states` selects the five largest final states. The pod driver aggregates the new workloads, same-process engine ratios, and those five states.

Timing commands must query compute tenants before and after each run. Foreign-tenant measurements are indicative only and cannot by themselves justify keeping an optimization. Bulky reports/profiles live under `/data/design-notes/doop-memory/packed-run/read-path/`; the compact summaries also live under `/data/design-notes/doop-memory/`.

## Initial measurements

Three fresh processes per experimental storage configuration, no foreign compute process observed before/after; all admission, final-content and five read oracles passed. All 26 packed-run tests passed, including the new scan/membership test across four word widths, both storage modes, empty sets, multiple runs, and admission after reads. `read-base.json` contains distributions and the five-largest-state breakdown; raw `base-{packed,plain}-{1,2,3}.json` files are outside git.

Median milliseconds (same-process experimental/engine ratios in parentheses):

| Workload | Engine | New plain | New packed |
|---|---:|---:|---:|
| Sorted prefix | 17.488 | 20.080 (1.148) | 29.552 (1.692) |
| Shuffled prefix | 39.594 | 42.097 (1.061) | 67.845 (1.714) |
| Mixed absent prefix | 47.480 | 50.671 (1.066) | 82.822 (1.744) |
| Range scan | 906.987 | 1274.151 (1.397) | 5554.795 (6.129) |
| Full-key membership | 20.145 | 23.212 (1.150) | 35.995 (1.789) |

Median million lookups/s in the same order: engine 7342.848 / 3243.264 / 3709.116 / 141.582 / 4925.036; new plain 6395.137 / 3050.397 / 3475.577 / 100.783 / 4274.220; new packed 4345.320 / 1892.737 / 2126.362 / 23.117 / 2756.379. Range scans read 275.934 / 196.420 / 45.054 million rows/s respectively. Final bytes are 269,661,952 / 269,655,808 / 96,674,304.

The initial scan maps one probe to one thread, exposing extreme fanout skew: in `base-packed-2`, state 112 alone takes 3074.671 ms for 730 prefixes and 1,435,800 rows; state 136 takes 1106.703 ms for 26 prefixes and 364,594 rows. These dominate the measured cost, ahead of descriptor uploads or fence search.

`PACKED_READ_STAGES=1` enables diagnostic CUDA-event experiments on shuffled probes: fence only, fence+header/frames, complete lower bound, and lower bound+row extraction. These stage totals use resident descriptors and ten launches; they are not additive source-line timings. Plain runs do no work in the first two stages. Full profiles include admission and oracle traffic; transfer attribution must use calls adjacent to the read kernels rather than assigning whole-process totals to lookup.

### Initial device profile

`profile-base` used the required nsys invocation and observed no foreign tenant. Compact per-kernel launch/device-time, API and memory tables are in `read-profile-base.json`; raw `.nsys-rep`, SQLite, CSV and full replay output are outside git. The dominant packed scan/membership specialization launches 748 times and takes 5516.974 ms device time (the corresponding engine specialization takes 955.226 ms); another packed specialization takes 608.677 ms in 1870 launches. These counts include warmups and both scan/membership modes.

Adjacent repeated read launches reveal one kernel per call. The modal packed call adds **one H2D descriptor copy and three event records**, versus no H2D and one event record for the engine. Both have two stream synchronizations from the timer; neither has timed D2H. Input/oracle transfers occur at workload boundaries, not per timed call. The extra descriptor transfer is therefore confirmed rather than assumed.

Summed device stage intervals over all states, ten repetitions of shuffled lower-bound probes: fence 6.571 ms, fence+header/frames 9.723 ms, complete lower bound 32.814 ms, lower bound+row 40.294 ms. This implicates in-block search/extraction more than fences, but the stage differences are not independent additive costs: stopping changes instructions/register pressure, and plain runs contribute only to the last two stages.

## Attempt log

### Cooperative fanout scans — kept

For batches averaging at least four rows per probe, use one warp per probe, search in lane zero, broadcast bounds, and scan 32 consecutive rows cooperatively. Smaller average fanouts retain one thread per probe. Apply exactly the same scheduling policy to the engine adapter and both experimental storage modes; no engine implementation is modified. No extra device allocation or retained bytes.

Three interleaved clean repetitions per storage, all five oracles and all 26 tests pass (`read-warp.json`). Median scan time falls **5554.795→465.148 ms packed**, **1274.151→131.175 ms new plain**, and **906.987→80.678 ms engine**. Packed scan reaches 538.040 million rows/s; its same-process engine ratio improves 6.129→5.765. Other workloads are unchanged within their observed variation. This fixes the measured largest cost (serial skew), rather than optimizing the much smaller fence stage first.

### Resident run-view descriptors — kept

Cache the pool-owned read descriptor buffer lazily on the set; all read workloads reuse it. Invalidate only when a successful publication changes the run list, after all fallible merge work. Admission itself does not populate this cache or change its kernels. The scan/membership regression reads before and after multiple publications, covering stale-pointer invalidation. Final pool accounting now occurs after reads, including both the engine's existing snapshot metadata and the new resident descriptors.

Three interleaved clean repetitions, all oracles and 26 tests pass (`read-resident.json`). Sorted/shuffled/mixed/membership packed times improve **29.769→27.181 / 68.041→65.598 / 83.024→80.555 / 36.389→33.893 ms**. New-plain sorted/shuffled/mixed/membership ratios fall to **1.004 / 0.997 / 1.012 / 1.029**, resolving most of its lookup gap. Packed scan is 462.937 ms, essentially unchanged as expected for the device-dominated workload.

Resident packed final bytes are **96,710,144**, +35,840 bytes (0.037%) over run storage alone. New plain is 269,691,648 bytes. Engine final accounting now includes its previously excluded existing snapshot metadata: 269,729,792 bytes, +67,840; this is an accounting correction, not a new engine allocation strategy.

### Block-local projected scan cursors — kept

Decode each requested non-prefix field with an incrementing bit cursor and hoist header/frame loads outside the row loop. The checksum consumes the decoded field directly, including fields crossing lane boundaries; it does not materialize/reinsert an entire logical key. Plain runs take a direct pointer scan. Memory and B=256 are unchanged.

Three clean repetitions per storage and all 27 tests/oracles pass (`read-cursor.json`). The added test covers full-width/uncompressed payloads, compressed frames, cross-lane fields, suffix masks 0/1/17/63/64/word−1/word, partial blocks and both thread/warp scheduling across four word widths. Packed scan improves **462.937→250.754 ms** (998.063 million rows/s; engine ratio **5.723→3.101**); new-plain scan improves **128.791→74.491 ms**. Lookup intervals remain unchanged. The shared scan/membership kernel's increased register pressure costs membership **33.893→35.360 ms packed**, **20.781→22.136 ms new plain**; this small regression is retained temporarily because the scan saves 212 ms, and motivates separating membership from scan code next.

### Separate membership from the scan body — kept

Launch a dedicated membership kernel rather than carrying the scan decoder's register footprint through a runtime branch. No extra launch, allocation or bytes. Three clean repetitions per storage, all 27 tests/oracles pass (`read-split.json`). Packed membership recovers **35.360→33.650 ms** and new plain **22.136→20.732 ms**, eliminating the cursor change's membership regression. Scan also improves **250.754→246.237 ms packed**, **74.491→70.328 ms new plain**. Prefix lookups remain unchanged.

### Residual-space bounds wider than 64 bits — kept

Preserve the existing ≤64-bit fast path. For wider compressed blocks, translate the probe once into a multi-lane residual word, including frame underflow/overflow, then compare extracted residual lanes instead of decoding/reinserting every field at each binary-search step. Uncompressed blocks retain the original exact reader. No format, allocation or admission pipeline changes.

Three clean repetitions per storage, all 28 tests/oracles pass (`read-wide.json`). New coverage forces compressed residuals wider than u64 across 128/192/256-bit words and B=17/256, checking exact keys and neighboring misses. Packed sorted/shuffled/mixed prefixes improve **27.186→21.777 / 65.485→44.599 / 80.494→53.918 ms**; membership **33.650→27.452 ms**; scan **246.237→228.426 ms**. Same-process ratios are **1.241 / 1.122 / 1.132 / 1.358 / 2.826** respectively. This confirms the profile's in-block extraction lead; changing fence layout first would have targeted the smaller stage.

### Lower warp crossover to two rows/probe — kept

The per-state breakdown exposed a skewed state (208) just below the initial four-row crossover. Lowering the common engine/experimental crossover to two improves clean median scans **228.426→192.836 ms packed**, **71.040→67.683 ms new plain**, **80.871→69.859 ms engine**; packed reaches 1297.831 million rows/s and ratio 2.761. Three clean repetitions per storage, all 28 tests/oracles pass (`read-avg2.json`); no byte change or measurable non-scan effect.

### Whole-CTA cooperation for sparse, high-fanout probes — kept

When there are at most 1024 probes and at least 32 rows/probe, assign 256 threads to a probe instead of 32. Other cooperative scans keep one warp. Each warp searches the same bounds, scans its disjoint strided rows, and contributes to a 64-byte shared-memory reduction; no extra global memory or launch. Both engine and experimental adapters use the identical policy.

Three clean repetitions per storage, all 28 tests/oracles pass (`read-cta.json`). Median scan improves **192.836→118.660 ms packed**, **67.683→63.462 ms new plain**, **69.859→63.009 ms engine**. Packed throughput reaches **2109.128 million rows/s**, ratio **1.883**. The existing scan regressions exercise thread/warp/CTA cooperation, partial workgroups and all word widths. Non-scan workloads and retained bytes are unchanged.

## Final results

Final is B=256, 32K plain cutoff, resident read views and adaptive thread/warp/CTA scans. Three fresh, interleaved, tenant-free processes per experimental storage configuration; the engine column uses the paired packed-storage processes. All medians below are local measurements; ratios are medians of each process's ratio, not ratios of independent medians.

| Workload | Engine ms | New plain ms (engine ratio) | New packed ms (engine ratio) |
|---|---:|---:|---:|
| Sorted prefix | 17.555 | 17.727 (1.013) | 21.792 (1.239) |
| Shuffled prefix | 39.779 | 39.338 (0.989) | 44.552 (1.121) |
| Mixed absent prefix | 47.674 | 47.861 (1.004) | 53.905 (1.131) |
| Range scan | 63.009 | 63.462 (1.008) | 118.660 (1.883) |
| Full-key membership | 20.310 | 21.116 (1.038) | 27.486 (1.355) |

| Workload | Engine million lookups/s | New plain million lookups/s | New packed million lookups/s |
|---|---:|---:|---:|
| Sorted prefix | 7315.009 | 7243.778 | 5892.567 |
| Shuffled prefix | 3228.132 | 3264.336 | 2882.286 |
| Mixed absent prefix | 3694.053 | 3679.595 | 3267.050 |
| Range scan | 2038.007 | 2023.460 | 1082.194 |
| Full-key membership | 4885.051 | 4698.491 | 3609.656 |

Range-scan million rows/s: **3971.950 / 3943.599 / 2109.128**. Final pool bytes: **269,729,792 / 269,691,648 / 96,710,144**; admission peak: **291,988,736 / 291,941,376 / 103,330,816**. Packed retains the memory saving with only 35,840 additional descriptor bytes. No whole run is unpacked by a timed read; full-run downloads remain explicit oracle validation.

### Five largest states

Times below are ms for ten repetitions, shown as **engine / new plain / new packed**. Full throughput, ratio distributions and min/max values for each state/workload are in `read-cta.json`; the pod driver prints and saves the same breakdown.

| State (final rows) | Sorted prefix | Shuffled prefix | Mixed prefix | Range scan | Membership |
|---|---|---|---|---|---|
| 90 (2,741,586) | .256/.256/.283 | .345/.337/.371 | .403/.389/.426 | 1.562/2.035/2.649 | .330/.389/.448 |
| 98 (2,741,586) | .261/.270/.299 | .314/.311/.346 | .317/.312/.346 | .533/.723/.951 | .392/.491/.594 |
| 112 (1,435,800) | .233/.206/.244 | .275/.237/.286 | .280/.241/.286 | 4.353/4.466/23.446 | .301/.312/.400 |
| 219 (1,435,800) | .258/.239/.283 | .301/.262/.316 | .300/.261/.315 | .319/.443/.665 | .335/.368/.510 |
| 137 (1,224,859) | .502/.546/.774 | 2.986/2.943/3.567 | 3.639/3.595/4.360 | 3.169/2.956/3.847 | .248/.254/.349 |

### Decisions and remaining gap

All seven isolated reader changes above helped in clean measurements; there are no timing-rejected commits in the final stack. Superseded designs are the serial high-fanout scan, four-row warp crossover and combined scan/membership kernel. Fence reordering, block/header format changes and a different B were not attempted: the initial stage experiment and subsequent wide-residual gain identified in-block work and fanout scheduling as the larger costs, without needing extra index bytes.

The new-plain reader is now near parity with the engine on every workload. Packed residual decoding still costs 12–24% on prefix lookups, 35.5% on membership and 88.3% on scans. The largest scan state, 112, still needs 23.446 ms versus 4.353 ms engine; its 730 prefixes cover 1,435,800 rows. State 136 has only 26 probes for 364,594 rows and takes 7.910 ms in `cta-packed-2`. [INFERENCE] The remaining scan gap combines bit extraction/projection with uneven range lengths and insufficient independent work for very sparse probes; splitting individual long ranges among multiple CTAs would require a different task-distribution scheme. This is not an upload or retained-memory problem.

## Final CUDA profile and reproduction

Local device: NVIDIA RTX 6000 Ada Generation, driver 580.178.04, 49,140 MiB as reported by `nvidia-smi`. Both `profile-final-packed` and `profile-final-plain` observed no foreign tenant and passed all replay oracles. Compact reports are `read-profile-final-{packed,plain}.json`; raw profiles remain outside git.

Read-only device totals below include one warmup plus ten measured launches per state: **1441 launches per workload per representation**. Ordered read launches were matched to the replay's state/workload sequence and checked against its iteration counts.

| Workload device ms | Engine | New plain | New packed |
|---|---:|---:|---:|
| Sorted prefix | 12.559 | 12.719 | 17.203 |
| Shuffled prefix | 37.024 | 36.747 | 42.249 |
| Mixed prefix | 45.808 | 46.300 | 52.507 |
| Range scan | 63.136 | 63.250 | 124.117 |
| Membership | 15.451 | 16.443 | 23.441 |

Every final read specialization has the same modal adjacent-call API pattern: **one launch, one event record, two timer stream synchronizations, no H2D and no D2H**. The packed full profile's H2D calls fall **17,022→9817**, exactly 7205 fewer repeated descriptor uploads; D2H stays 10,929, stream synchronizations 60,105, and total launches 61,465. These whole-profile counts include both representations, admission, oracle work and opt-in stage diagnostics; only the adjacent-call pattern describes timed reads.

Final packed lower-bound stage event intervals (ten repetitions): **6.700 / 9.990 / 22.289 / 29.477 ms** for fence / +headers / complete lower bound / +row. The complete bound falls from 32.814 ms to 22.289 ms while fence time stays near 6.6 ms. The plain-storage diagnostic gives 4.579 / 4.140 / 19.435 / 20.306 ms; its first two modes do no search, illustrating launch/submission floors and why these experiments must not be treated as additive line-level attribution.

Reproduce from this worktree's `experiments/gpu`:

```sh
source /dev/shm/gpu-env.sh
flock -s /tmp/mini_linq-timing.lock timeout 900 \
  cargo +1.99.1 oxide build --arch sm_89 --materialize-cubin -- \
  --release -p gpu-datalog-runtime --bin packed-replay
# Build tests with oxide test and --test packed_run --no-run; run the emitted
# executable with timeout 100 and --test-threads=1 under the GPU lock.
flock /tmp/mini_linq-gpu.lock bash -c '
  nvidia-smi --query-compute-apps=pid,process_name --format=csv,noheader
  target/release/packed-replay --output /data/design-notes/doop-memory/packed-run/read-path/repeat.json
  nvidia-smi --query-compute-apps=pid,process_name --format=csv,noheader'
```

Add `--plain-storage` for the experimental plain control; the engine is measured in every process. For profiling, prefix the executable with `PACKED_READ_STAGES=1 /opt/nvidia/hpc_sdk/Linux_x86_64/25.5/compilers/bin/nsys profile -t cuda --sample=none --cpuctxsw=none -o PROFILE` inside the same GPU-lock/tenant-query protocol. Never hold the build and GPU locks together.

The pod driver CLI and bundle conventions remain those in `OPTIMIZATION.md`; its aggregation was exercised against all local three-configuration summaries and the stage-enabled final profiles. The JSON summaries under `/data/design-notes/doop-memory/` are below 200 KB each. No raw profile, trace, throwaway script, engine implementation, RCCL source or unrelated subsystem is committed.

## Pod confirmation (RunPod RTX 6000 Ada, 2026-11-15)

Session `20261115T234934`: bundles `df162881` (read workloads added, readers unchanged), `312d5ce0` (final), `df162881` again; five fresh processes per configuration and bundle, no other GPU process, all oracles passed. Median ms and median same-process ratio to the engine path for the packed-storage configuration:

| Workload | `df162881` (run 1) | `312d5ce0` | `df162881` (run 3) |
|---|---:|---:|---:|
| Sorted prefix | 29.99 (1.679) | 22.30 (1.235) | 31.39 (1.661) |
| Shuffled prefix | 66.27 (1.695) | 45.04 (1.122) | 67.49 (1.668) |
| Mixed absent prefix | 80.85 (1.723) | 54.32 (1.129) | 82.36 (1.712) |
| Full-key membership | 36.10 (1.773) | 27.97 (1.347) | 38.05 (1.764) |
| Range scan | 5376.44 (6.157) | 118.46 (1.871) | 5401.17 (6.185) |

Plain-storage control at `312d5ce0`: 1.015 / 1.000 / 1.009 / 1.052 / 1.013. Engine-path scans fall from 875 ms to 63 ms because the replay's scan kernel is shared by all three configurations. Packed admission is 169.5 ms against 199.2 ms for the engine path in the same session; final pool 96.7 MB against 269.7 MB. The local measurements above are confirmed.
