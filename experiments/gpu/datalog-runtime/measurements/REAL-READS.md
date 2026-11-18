# Real DOOP xalan read replay

## Decision summary

On the captured real mix, **E/P/C read stream time is 152.019 / 145.264 / 181.981 ms**: paired **P/E = 0.956**, **C/E = 1.198**, **C/P = 1.253**. Packed is not a throughput win at this size. It is a substantial memory trade-off.

**Projection, not an engine result:** using the supplied prior wall partition and the measured paired admission ratios gives **E/P/C = 669.000 / 640.230 / 710.825 ms**. C is projected **6.25% slower**, while peak pool-allocated application-buffer memory falls from **301.673 to 120.782 MiB** under the assumptions below. The separately batched execution model still has **C/E = 1.090**; its lower absolute time is not an engine speedup.

The capture contains 240 static sites. All 209 sites with observed reads are replayed; the other 31 issue no recorded probes. There are no silently dropped active sites.

## Scope and exactness

This is a measurement adapter, not an engine integration or a decoder change. The existing synthetic workloads remain available and unchanged. E means the engine's plain radix run reader, P means the experimental set's plain reader, and C means its packed reader, retaining plain storage for runs of at most 32,768 rows. C uses the existing all-plain specialization when every run of a snapshot is plain.

The local device was **NVIDIA RTX 6000 Ada Generation**, driver **580.178.04**, with **49,140 MiB** reported device memory. No cloud instance was used. Raw captures, binaries, logs and process-level results are under `/data/design-notes/doop-memory/real-replay/`; none of the bulky traces belongs in git.

### Why a new capture was necessary

The preceding read trace recorded exact probe keys and bounds, but only aggregate child-enumeration counts. A count cannot identify which children a root work-sharing helper actually consumed. Its separately captured admissions also did not provide a direct snapshot/epoch join. Replaying every returned interval would therefore be wrong.

The throwaway `packed-trace` worktree was extended, without committing there, to capture **admissions, immutable snapshots, probes and actual child addresses in one fixpoint**. `joint-v2/` is the final capture; `admission-v2/` contains its admission stream. The additions record:

- The snapshot identity immediately after each full-state admission/publication, with its state, stratum, phase and round.
- Each immutable run's complete sorted keys and the snapshot's packing fields, including signedness. Repeated observations on different worker threads describe the same immutable snapshot.
- One additional tagged record for every child actually enumerated: site, snapshot, run and run-local row address. This preserves helper probes without multiplying enumerated rows by their returned range lengths.
- The original directory and encoded-column headers/payload for legacy nonradix sites, identified by launch and site rather than by a potentially recycled device address.
- Observed scalar/cartesian child values for whole-source sites. These unchanged E/P/C sites use a common native-value replay carrier; their row identities and checksums are checked, but this is not a claim to reproduce their original scalar producer's physical encoding.

The final capture passed the DOOP driver's **exact full-tuple comparison for all 26 outputs**. It contains **222,416,739 probes**, **69,170,748 enumerated rows**, and **291,587,487 probe/child records**, across **8,882 generated launches**. The complete replay exactness run checked those records across **19,324 nonempty launch/site groups**, covering **209 sites with observed reads**. Empty read groups do not acquire invented probes or enumeration work.

`prepare_real.py prepare` verifies the admission provenance before publishing the replay manifest. The final proof records **2,121 distinct snapshots**, **1,036 exact full-admission snapshot joins**, **983 exact fresh-run matches**, and **no unmatched runs**. Empty snapshots have no run payload to compare. Two static negative projections are additionally checked byte-for-byte against admitted source relations:

| Snapshot | Admitted source | Projected packed-field positions | Existing constant filter | Rows |
|---|---|---|---|---:|
| 412 | `method_modifier`, state 59 | 1 | field 0 = encoded value 1088198 | 8,681 |
| 416 | `methodimplemented`, state 208 | 0, 1, 2 | none | 93,196 |

These are provenance checks for existing anti-join key sets, not new negative filters. Their construction is included in the admission replay: **2,161 recorded full-state batches plus two captured negative-projection constructions**. Full-state snapshots are compared against the exact accumulated admissions and recorded run-length geometry, not matched merely by row count. Fresh-run matches compare complete captured bytes. Negative-projection proofs compare complete projected sets.

### Replay semantics and boundaries

- Each recorded run probe retains its keys, bound-column count, membership/anti-join identity, hit/miss outcome and repeated-helper multiplicity. Keys are encoded outside the timer using the captured packing. The replay calls the settled engine/plain/packed range readers; it does not invent a uniform shuffle or substitute one probe per distinct prefix.
- Root probes are stably source-ordered by snapshot, run and source ordinal. Root child records retain their captured completion order. Nested probes and children retain the instrumented atomic completion stream. Neither sequence is called uninstrumented hardware issue order.
- Actual child addresses, not returned-range lengths, drive enumeration. Every child checksum comes from the exact captured run row. Full-key membership and anti-join probes perform no child scans. Per-run Boolean answers are checked with the bounds; OR over the checked runs supplies membership, followed by the recorded negation for anti-join semantics.
- Host binary-search oracles first verify the recorded bounds against the captured state. GPU E/P/C results are then checked against those bounds, membership answers and child checksums before timing identical immutable inputs. Legacy directories/columns use their captured encodings. Scalar/carrier sites are checked separately.
- The primary execution model has **one timed device launch per nonempty recorded launch/site group**. Preparation, uploads, output allocation and oracle downloads are outside the timer. The E/P/C timing order rotates with launch and repetition. Device allocations use the pool.
- CUDA-event stream intervals and synchronized host intervals are both retained. The latter include event submission and synchronization overhead; the former are stream intervals, not a claim of exclusive decoder time. NVTX-scoped hardware counters are obtained separately.
- Per-kind measurements are **stratified diagnostics**: each launch/site's jobs are filtered by recorded range class, retaining their order and associated actual children. Their extra launches are not charged to the mixed total. These timings are not additive causal attribution of the fused mixed launch.
- The separately labelled batched model concatenates launches only when the kernel/site and immutable snapshot match. Each original root probe sequence and child sequence is retained within its chunk. Legacy/scalar states are never merged by pointer identity. It reduces the **19,324 nonempty site groups to 3,965 batches** and changes both scheduling and locality; it is not the engine's current execution model.

The replay deliberately does not regenerate the rule's joins, outputs or control dependencies. Nested keys are already captured. Child reads are independently addressable jobs rather than dependent expansion inside the rule kernel. Query-key encoding, carried parent fields, demanded-column selection, source/output traffic, concurrent rule streams and fused-kernel register pressure are not reproduced. Keeping the work mix and small launches does not make the replay kernel an engine rule kernel. In particular, an individual snapshot fitting cache is not proof that the engine's concurrent working set is resident.

## Measured engine memory reference

A separate local run of the uninstrumented read-workload baseline executable, with the engine's memory census enabled, passed the same 26-output exact comparison. Both compute-process checks were empty. The census observed **316,326,912 bytes peak allocated**, **535,822,336 bytes peak reserved**, and **275,147,264 bytes allocated at its final observation**. The census changes host execution time, so this run is a memory reference, not a replacement wall-time baseline. The read replay's simultaneous E/P/C pool peak includes its probe/output scratch and must not be mistaken for engine peak memory.

## Reproduction and artifacts

Build from `experiments/gpu`, after `source /dev/shm/gpu-env.sh`:

```sh
flock -s /tmp/mini_linq-timing.lock timeout 900 \
  cargo +1.99.1 oxide build --arch sm_89 --materialize-cubin -- \
  --release -p gpu-datalog-runtime --bin packed-replay \
  --features cq-gpu-runtime/nvtx
```

Never hold the build lock and GPU execution lock together. The local measurement driver takes `/tmp/mini_linq-gpu.lock` for each fresh process, checks `nvidia-smi --query-compute-apps=pid,process_name --format=csv,noheader` before and after it, and rejects contention or an incomplete run. Every process measures all three representations, including admission, so ratios are paired within a process.

```sh
ROOT=/data/design-notes/doop-memory/real-replay
python3 datalog-runtime/src/bin/packed_replay/prepare_real.py prepare \
  "$ROOT/admission-v2" "$ROOT/joint-v2" "$ROOT/join-v2.json"
# The batch destination must not already exist.
python3 datalog-runtime/src/bin/packed_replay/prepare_real.py batch \
  "$ROOT/joint-v2" "$ROOT/batched-v2"
python3 compare/real_packed_replay.py measure "$ROOT/packed-replay" \
  "$ROOT/admission-v2" "$ROOT/joint-v2" "$ROOT/launch-model"
python3 compare/real_packed_replay.py measure "$ROOT/packed-replay" \
  "$ROOT/admission-v2" "$ROOT/batched-v2" "$ROOT/batched-model"
python3 compare/real_packed_counters.py "$ROOT/packed-replay" \
  "$ROOT/joint-v2" "$ROOT/launch-model/process-0.json" "$ROOT/counters" \
  /opt/nvidia/hpc_sdk/Linux_x86_64/25.9/compilers/bin/ncu
```

The measurement driver defaults to five fresh processes and refuses fewer. Existing sample files are not overwritten. The counter driver selects the three heaviest sites by aggregate measured E stream time, then profiles each site's largest recorded probe-plus-child launch for E/P/C with `--cache-control none --clock-control none` and the `real_site/real_{engine,plain,packed}/` NVTX scope. Counter runs are not timing samples.

Capture provenance is retained as `capture.sha256`, `joint-doop-gpu-v2`, `instrumentation-tracked.patch`, `read_trace.rs` and `packed_trace.rs`. The tracked patch and the two trace modules belong to the throwaway worktree only. The intentionally uncommitted RCCL override in the packed-run Cargo manifest and lockfile is not part of this change.

To recapture, use the instrumented throwaway checkout and **fresh, empty output directories**, with `READ_TRACE_BUILD=1` on its locked `doop-gpu` build. The final capture used `READ_TRACE_CAPACITY=12000000`, `READ_TRACE="$ROOT/joint-v2"`, `PACKED_TRACE="$ROOT/admission-v2"`, `GPU_DATALOG_PHASES=1`, and the xalan input `/data/doop-bench/doop-data/xalan`, under the exclusive GPU lock with compute-process queries before and after. Its build was:

```sh
cd /data/mini_linq-wt/packed-trace/experiments/gpu
READ_TRACE_BUILD=1 flock -s /tmp/mini_linq-timing.lock timeout 900 \
  cargo +1.99.1 oxide build --arch sm_89 --materialize-cubin -- \
  --release -p doop-gpu --features cq-gpu-runtime/nvtx
```

The capture retains output-capacity retry launches, including unsuccessful capacity attempts. It does not deduplicate repeated helpers, remove misses, or discard nested probes. Overflow is a hard error, not truncation. Instrumentation affects scheduling, so helper multiplicities describe this captured execution rather than an asserted uninstrumented issue stream.

## Interpreting negative-filter opportunities

The engine E reader already checks a run's minimum and maximum before binary search. Consequently, an out-of-run-domain miss is **not** an opportunity to remove an E binary search that is already avoided. The P reader uses its settled plain binary-bound specialization; C uses fences followed by the selected block. Interior misses are the common negative-filter opportunity for both plain and packed.

**[INFERENCE: reader control-flow consequence]** A nonempty genuinely packed run does not currently terminate a miss from fences alone: the selected block's header is consulted by `bound`. An empty run is a separate zero-work case, not a successful fence rejection. A captured prefix strictly below the first fence is an opportunity for an earlier rejection using existing metadata; an upper-domain check would additionally need a usable run maximum. These statements are not cache-hit deductions from Nsight counters.

A hypothetical perfect per-run or per-block negative filter supplies only an upper bound on avoidable search work. Membership may return false immediately. Prefix-range callers that need only emptiness can also benefit, but a generic API promising the exact insertion bounds may still need to compute those bounds. Filter memory traffic, false positives, construction/admission costs and the retained launch/input/output overhead must be charged before translating rejected-probe counts into a speedup. No filter, reader fast path, format change or decoder change was implemented here.

## Paired timings

All tables below use five fresh processes per execution model, each measuring E/P/C in the same process. Times are medians; ratios are medians of within-process ratios, not ratios of independently chosen medians. Ratios above one mean slower. Primary tables use CUDA-event stream intervals.

Compact complete results under `/data/design-notes/doop-memory/`: launch-preserving classes (`REAL-READS-launch.json`), batched classes (`REAL-READS-batched.json`), counter captures (`REAL-READS-counters.json`), and exactness/opportunity evidence (`REAL-READS-exactness.json`). Each summary is below 200 KB. Full per-launch/site results remain outside git.

### Mixed totals

| Execution model | Probes M | Enumerated M | E ms | P ms | C ms | P/E | C/E | C/P |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Recorded launch/site groups | 222.417 | 69.171 | 152.019 | 145.264 | 181.981 | 0.956 | 1.198 | 1.253 |
| Separately batched | 222.417 | 69.171 | 44.352 | 42.857 | 48.306 | 0.968 | 1.090 | 1.128 |

Batching uses future captured probes from later launches; it is not an online schedule for the unchanged engine.

### Process variation and host intervals

| Model | E stream ms [min, max] | P stream ms [min, max] | C stream ms [min, max] |
|---|---:|---:|---:|
| Launch/site | 152.019 [151.598, 152.371] | 145.264 [145.060, 145.733] | 181.981 [181.624, 182.445] |
| Batched | 44.352 [44.254, 50.131] | 42.857 [42.697, 48.834] | 48.306 [48.241, 54.301] |

| Model | E host ms [min, max] | P host ms [min, max] | C host ms [min, max] |
|---|---:|---:|---:|
| Launch/site | 234.997 [234.464, 235.506] | 228.129 [227.854, 228.745] | 264.992 [264.345, 265.282] |
| Batched | 61.558 [61.363, 72.054] | 60.021 [59.875, 70.531] | 65.893 [65.422, 76.112] |

The host-interval paired C/E ratio is 1.127 for the launch-preserving model. The projection instead uses the stream ratio, since its supplied read partition is a device-kernel wall partition.

### Recorded kinds: stratified diagnostic launches

These preserve real keys, misses and associated child work, but separate classes into extra launches. Do not sum their times to infer the mixed total or interpret them as exclusive time attribution. Short means 2–32 returned rows; long means more than 32. One-row hits include membership hits; membership/anti-join probes never enumerate children.

| Kind | Probes M | Enumerated M | E ms | P ms | C ms | P/E | C/E | C/P |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Membership misses | 60.934 | 0.000 | 19.195 | 18.070 | 19.379 | 0.942 | 1.010 | 1.071 |
| Prefix-range misses | 92.353 | 0.000 | 69.689 | 67.509 | 86.184 | 0.969 | 1.236 | 1.275 |
| One-row hits | 49.664 | 34.416 | 119.060 | 113.761 | 139.227 | 0.956 | 1.170 | 1.224 |
| Short ranges | 7.884 | 8.522 | 40.147 | 38.323 | 48.848 | 0.954 | 1.215 | 1.274 |
| Long ranges | 11.583 | 26.232 | 13.599 | 13.038 | 16.490 | 0.958 | 1.214 | 1.268 |

Batched-model stratified diagnostics:

| Kind | Probes M | Enumerated M | E ms | P ms | C ms | P/E | C/E | C/P |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Membership misses | 60.934 | 0.000 | 11.473 | 10.839 | 11.499 | 0.943 | 1.002 | 1.061 |
| Prefix-range misses | 92.353 | 0.000 | 21.331 | 20.554 | 24.908 | 0.967 | 1.168 | 1.208 |
| One-row hits | 49.664 | 34.416 | 22.405 | 21.572 | 23.948 | 0.963 | 1.068 | 1.110 |
| Short ranges | 7.884 | 8.522 | 8.136 | 7.778 | 9.489 | 0.956 | 1.164 | 1.220 |
| Long ranges | 11.583 | 26.232 | 6.701 | 6.500 | 7.406 | 0.971 | 1.106 | 1.138 |

### Probe order

Sorted roots have at least 95% nondecreasing adjacent encoded prefixes within a run. “Unsorted” does not mean uniformly shuffled. Nested order means the captured instrumented completion stream, not an asserted uninstrumented order. Legacy/scalar root streams are separately labelled rather than assigned an invented sortedness.

| Class | Probes M | Enumerated M | E ms | P ms | C ms | P/E | C/E | C/P |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Root sorted | 80.362 | 23.328 | 32.287 | 31.017 | 40.824 | 0.961 | 1.265 | 1.315 |
| Nested completion | 114.499 | 41.873 | 95.709 | 91.301 | 110.674 | 0.955 | 1.157 | 1.212 |
| Root unsorted | 27.205 | 3.955 | 21.461 | 20.506 | 27.487 | 0.956 | 1.281 | 1.340 |
| Root insufficient comparisons | 0.000 | 0.008 | 2.487 | 2.358 | 2.902 | 0.950 | 1.165 | 1.231 |
| Root whole source | 0.007 | 0.007 | 0.027 | 0.028 | 0.028 | 1.029 | 1.041 | 1.002 |
| Root legacy | 0.343 | 0.000 | 0.064 | 0.066 | 0.067 | 1.052 | 1.036 | 1.008 |

### Probes per recorded site launch

| Class | Groups | Probes M | Enumerated M | E ms | P ms | C ms | P/E | C/E | C/P |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| <32 | 4,936 | 0.052 | 1.382 | 34.741 | 33.080 | 41.069 | 0.952 | 1.182 | 1.241 |
| 32–1,023 | 7,743 | 2.238 | 8.583 | 58.398 | 55.790 | 68.232 | 0.956 | 1.168 | 1.223 |
| 1,024–29,999 | 5,213 | 43.356 | 29.883 | 41.976 | 40.095 | 50.445 | 0.957 | 1.203 | 1.258 |
| ≥30,000 | 1,432 | 176.771 | 29.323 | 16.903 | 16.299 | 22.239 | 0.965 | 1.315 | 1.364 |

### Probes across the recorded rule launch

This classifies each replay site's time by the total observed probes across its original launch's sites; it does not pretend the split replay sites were one fused kernel.

| Class | Probes M | Enumerated M | E ms | P ms | C ms | P/E | C/E | C/P |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| <32 | 0.012 | 0.023 | 13.032 | 12.404 | 15.493 | 0.952 | 1.187 | 1.248 |
| 32–1,023 | 0.668 | 1.071 | 40.035 | 38.206 | 47.539 | 0.956 | 1.187 | 1.244 |
| 1,024–29,999 | 16.773 | 4.079 | 54.543 | 52.079 | 64.788 | 0.956 | 1.189 | 1.244 |
| ≥30,000 | 204.963 | 63.998 | 44.409 | 42.579 | 54.183 | 0.959 | 1.220 | 1.273 |

The raw capture has respectively 3,673 / 2,170 / 1,931 / 1,108 launches in those four bins, including 2,458 launches with no indexed/whole-source probes. Thus 7,774 captured launches have fewer than 30,000 probes. No read-only replay work is invented for zero-read launches.

### Captured plain state size

| Class | Probes M | Enumerated M | E ms | P ms | C ms | P/E | C/E | C/P |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| ≤128 KiB | 53.481 | 0.917 | 23.777 | 22.872 | 22.780 | 0.961 | 0.957 | 0.997 |
| 128 KiB–1 MiB | 70.953 | 29.275 | 52.870 | 50.422 | 65.696 | 0.954 | 1.243 | 1.302 |
| 1–8 MiB | 88.172 | 30.719 | 51.269 | 49.029 | 64.815 | 0.958 | 1.265 | 1.321 |
| >8 MiB | 9.810 | 8.260 | 24.095 | 22.937 | 28.672 | 0.952 | 1.192 | 1.251 |

Maximum captured plain state size is 22,973,084 bytes. There are no states above 96 MiB; the table does not assert cache residency.

### Actual storage of probed runs

| Class | Probes M | Enumerated M | E ms | P ms | C ms | P/E | C/E | C/P |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| All probed runs packed | 121.470 | 57.443 | 106.949 | 102.074 | 133.473 | 0.955 | 1.250 | 1.308 |
| All probed runs plain | 65.073 | 1.348 | 31.432 | 30.176 | 30.110 | 0.959 | 0.958 | 0.999 |
| Mixed probed runs | 35.520 | 10.372 | 13.479 | 12.890 | 18.213 | 0.957 | 1.354 | 1.414 |
| Unchanged legacy/scalar | 0.353 | 0.007 | 0.137 | 0.143 | 0.141 | 1.029 | 1.021 | 0.983 |

Exactly 131,597,793 probes, or 59.17%, address runs genuinely stored packed. Small-state C wins against E are the plain specialization, not compression wins. Both the all-packed and mixed cohorts lose against P.

### Ten heaviest measured sites

Ranked by the five-process median aggregate E replay stream time, not by duplicated parent-kernel weights. Site identity is hole/site; complete kernel and heading metadata is in the compact summary.

| Site / relation | Probes M | Enumerated M | E ms | P ms | C ms | P/E | C/E | C/P |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| 132/0 `loadinstancefield` | 20.533 | 9.643 | 2.553 | 2.500 | 4.538 | 0.980 | 1.778 | 1.815 |
| 121/0 `assignlocal` | 3.514 | 0.883 | 2.223 | 2.130 | 2.498 | 0.959 | 1.123 | 1.175 |
| 131/0 `loadinstancefield` | 3.076 | 0.142 | 2.037 | 1.970 | 3.314 | 0.968 | 1.629 | 1.684 |
| 136/6 `methodlookup` | 6.120 | 1.223 | 1.996 | 1.916 | 2.658 | 0.961 | 1.334 | 1.388 |
| 143/6 `methodlookup` | 5.738 | 1.146 | 1.957 | 1.877 | 2.638 | 0.959 | 1.351 | 1.407 |
| 145/6 `methodlookup` | 5.738 | 1.146 | 1.952 | 1.878 | 2.678 | 0.958 | 1.372 | 1.431 |
| 123/0 `assigncast` | 3.039 | 0.296 | 1.876 | 1.782 | 3.291 | 0.946 | 1.755 | 1.847 |
| 138/0 `storeinstancefield` | 3.057 | 0.101 | 1.799 | 1.730 | 3.174 | 0.958 | 1.764 | 1.840 |
| 119/0 `assign` | 12.952 | 5.764 | 1.787 | 1.711 | 2.072 | 0.953 | 1.159 | 1.217 |
| 136/0 `virtualmethodinvocation_base` | 3.413 | 1.225 | 1.782 | 1.693 | 2.083 | 0.953 | 1.172 | 1.230 |

## Hardware counters

The three heaviest sites in the first clean process are also the three heaviest by five-process median. Each site's largest probe-plus-child launch is profiled for E/P/C:

| Rank / site | Recorded launch | Probes | Enumerated rows |
|---|---:|---:|---:|
| 1: 132/0 | 2625 | 1,414,765 | 218,465 |
| 2: 121/0 | 3624 | 92,586 | 24,322 |
| 3: 131/0 | 3712 | 93,097 | 7,337 |

Counters include replay input/output traffic and actual children, not only bound search. Active-thread instructions/probe includes enumerated-row work. Long-scoreboard is the percentage per active warp. Cache control and clock control are both `none`.

The broad counter set reported some derived all-sector L2 percentages above 100. Those are retained in the raw evidence but **not interpreted as physical hit rates**. The displayed L2 column instead comes from separate, isolated `lts__t_sectors_op_read_lookup_hit.sum / lts__t_sectors_op_read.sum` captures of the same launch/site/representation. No percentage was clipped or invented.

| Rank / mode | Instructions/probe | Long scoreboard % | L1 hit % | L2 read hit % | Registers | Achieved occupancy % |
|---|---:|---:|---:|---:|---:|---:|
| 1 / E | 1074.78 | 70.79 | 53.23 | 6.47 | 34 | 91.41 |
| 1 / P | 956.32 | 73.25 | 51.70 | 6.73 | 30 | 90.88 |
| 1 / C | 3337.44 | 12.39 | 66.92 | 3.51 | 64 | 64.14 |
| 2 / E | 1114.24 | 62.67 | 73.85 | 99.27 | 34 | 41.32 |
| 2 / P | 996.13 | 64.15 | 72.45 | 99.52 | 30 | 40.95 |
| 2 / C | 1822.85 | 53.37 | 85.75 | 95.96 | 64 | 42.44 |
| 3 / E | 918.52 | 57.41 | 72.87 | 99.02 | 34 | 39.96 |
| 3 / P | 1004.17 | 58.54 | 72.02 | 99.44 | 30 | 39.83 |
| 3 / C | 2220.46 | 29.32 | 86.09 | 95.11 | 64 | 33.87 |

Findings:

- At 132/0, C uses 3,337 active-thread instructions/probe against P's 956, and 64 registers against 30. Occupancy falls from 90.88% to 64.14%, while the long-scoreboard share falls sharply. **[INFERENCE]** This is consistent with replacing load-wait time with decoder/control work and register pressure, not with a latency-saving speedup. The measured whole-site C/P ratio is 1.815.
- At 121/0, C's improved L1 hit rate does not overcome 1,823 instructions/probe versus P's 996; whole-site C/P is 1.175. Occupancy is similar here, so an occupancy-only explanation would be wrong.
- At 131/0, C again has more than twice P's instructions/probe and lower occupancy, despite a higher L1 hit rate; whole-site C/P is 1.684. The high L2 read hit rates at sites 2 and 3 do not remove instruction/dependency costs.
- The very low L2 read hit rate at site 1 is a whole-replay-kernel measurement. **[INFERENCE]** Its large streaming probe/output buffers contribute to that traffic; it is not evidence that the small relation state itself has that hit rate.

## Admission and projection

| Configuration | Admission ms [min, max] | Peak pool-allocated MiB | Final pool MiB |
|---|---:|---:|---:|
| E | 182.175 [181.557, 182.545] | 279.951 | 258.658 |
| P | 139.377 [139.144, 139.873] | 279.906 | 258.652 |
| C | 157.360 [157.218, 157.823] | 99.060 | 92.712 |

Paired admission ratios are **P/E = 0.76561**, **C/E = 0.86401**, **C/P = 1.12915**.

**Projection, not an integrated-engine measurement.** The supplied prior wall partition is an assumption: **[INFERENCE]** 262 ms reads, 74 ms admission and 333 ms no-kernel time. For representation X:

`wall_X = 333 + 262 × (read_X / read_E) + 74 × (admission_X / admission_E)`.

The primary projection uses only the launch-preserving real mix. Ratios are paired within each process.

| Configuration | Projected fixpoint ms [min, max] | Projected peak pool-allocated MiB |
|---|---:|---:|
| E | 669.000 [669.000, 669.000] | 301.673 |
| P | 640.230 [639.940, 640.413] | 301.628 |
| C | 710.825 [710.392, 711.029] | 120.782 |

The memory projection is `measured_engine_peak + replay_peak_X - replay_peak_E`, giving **126,649,088 bytes** for C. It holds other allocations constant, assumes peak phases coincide, unchanged run geometry/retention, and no additional fused-kernel scratch. It estimates pool-allocated application buffers, **not allocator-reserved or whole-process VRAM**. CUDA context/module memory is not counted and is assumed unchanged. The measured reserved-memory peak is not projected.

The wall projection assumes the replay read ratio applies to the complete prior read wall partition; admission ratios transfer unchanged; and host gaps, transfers, launch scheduling/overlap, source/output work and all other engine work remain unchanged. Admission ratios are synchronized host-operation ratios, so applying them to the supplied admission-kernel wall partition is an additional modelling assumption. Inputs are the captured sorted-unique candidates used by the existing admission replay, not reconstructed original unsorted output-buffer entropy.

Real rule kernels are not these replay kernels. Captured nested keys remove the real dependency chain; additional live decoded fields can change register pressure and occupancy in fused kernels. Site isolation and warming do not reproduce concurrent engine cache pressure. This is a sensitivity estimate, not a promised fixpoint speedup.

With the measured C admission ratio, break-even requires a read C/E ratio of at most **1.0384**, versus the observed **1.1980**. The projection favours P for time (4.30% below E) and C for memory, not C for time.

## Reader properties that matter on this mix

1. **Cheap misses, especially interior misses.** Misses are **68.92%** of probes: 60,933,839 membership misses and 92,352,951 range misses. There are **152,865,491 nonempty radix-run misses**, of which **33,964,200 (22.22%)** are outside the run's prefix domain. E already has its min/max fast rejection. The remaining **118,901,291 interior misses** are the common upper-bound opportunity for negative filters in both plain and packed, subject to the membership/empty-range contract caveat above.
2. **Use the first fence before touching a block.** There are **74,717,182 misses on genuinely packed runs**. **[INFERENCE: current control flow]** zero of these terminate from fences alone: a selected nonempty block is consulted. The trace-derived opportunity is concrete: **18,574,122 (24.86%)** of these misses have their entire prefix interval below the first fence. A further **4,495,852** are above the last row. Those are measured rejection opportunities, not measured speedups or implemented checks. Across plain and packed, the candidate-block min/max opportunity is 34,183,761 misses, only **219,561** more than the run-domain opportunity; a membership/prefix filter could address interior holes that those envelopes cannot.
3. **Low instruction cost for one-row/short reads.** One-row hits are **22.33%** of probes; **94.79%** of all probes return at most 32 rows. More decoder work is difficult to amortize here. The counter results show that higher L1 hit rates alone do not compensate for C's extra instructions and live registers.
4. **Keep helper probes distinct from child volume.** Raw returned cardinalities total **911,785,435 rows**, but only **69,170,748** are enumerated: **13.18×** less work than scanning every returned range. Root radix sites issue **107,567,323 probes**, including **22,634,031 repeated helper probes (21.04%)**. Their returned volume is 857,033,461 rows; source/run deduplication reduces that diagnostic returned volume to 27,517,045. The benchmark retains the repeated probes and never substitutes the deduplicated stream.
5. **Small launches and nested dependencies matter.** There are 17,892 nonempty site groups below 30,000 probes. Batching changes the result materially, but is a different execution model requiring future captured keys. **51.48%** of probes are nested-completion-order probes; their independent replay cannot establish the register pressure or latency hiding of fused nested lookups.
6. **Distinguish compression from the plain fallback.** Only **59.17%** of probes actually address packed runs. The all-plain cohort's C/P ratio is **0.999**; the all-packed and mixed cohorts are **1.308** and **1.414**. Reporting all C probes as compressed would hide the principal result.

The capture contains **74,032,110 membership probes**, including **717,578 anti-join probes**, and **zero out-of-domain encoding rejections**. The **89,195 empty-run misses** are separate from opportunities on nonempty runs.

The opportunity analysis is reproducible without a GPU:

```sh
python3 datalog-runtime/src/bin/packed_replay/analyze_real.py \
  "$ROOT/joint-v2" "$ROOT/capture-opportunities-final.json"
```

All bounds and child checksums passed in the complete replay and in every clean timed process. The original synthetic workloads and all engine/format/decoder/test files in the packed-run worktree were left unchanged. No paid GPU, push, or new filter was used.
