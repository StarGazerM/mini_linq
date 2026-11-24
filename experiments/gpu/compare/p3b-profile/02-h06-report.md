# Advised GPU TPC-H h06, SF1

> Point-in-time measurement: M0 binaries, 2026-10-31 (source revision not stated in this report; preserved by `1031adf6`, 2026-10-31). **Superseded implementation:** P3e's recorded grid fold in [PLANNING-REDESIGN](../../../../doc/PLANNING-REDESIGN.md); this exact profile has not been re-measured since in the retained evidence, and the one-CTA diagnosis below describes the measured baseline, not the current implementation.

Existing M0 `cq-gpu-tpch-advised OUTPUT --scale sf1 --mode advised --query h06`; no compilation. `h06.results.json` reports exact/conformance passed, one answer, 6,001,215 input rows, and median total **0.486912 ms** (build 0.001888, kernels 0.002368, dedup 0.479136 ms). The label `dedup` is broad: `sequence_trace` explicitly says **`group_without_canon`**. No radix sort or duplicate-removal sort occurs in the query timeline; this case is a grouped-fold/materialization problem, not expensive sorting.

## nsys evidence

Five measured executions, excluding the first warmup. The first group-argument launch through final reduction averages 465.418 us; active kernels total 388.733 us, leaving 76.685 us of intervening host/API/device-memory-operation time. The event-timed total also includes work outside this narrower kernel envelope, so it is not identical to the sum of these means.

| Kernel / work | Calls per query | Mean us | Share of active query kernels |
|---|---:|---:|---:|
| `reduce` | 1 | 198.824 | 51.15% |
| `__cq_group_arguments_...a3f16380a6168d5c_3` | 1 | 124.257 | 31.96% |
| `device_scan_lookback_u32` | 2 | 25.850 | 6.65% |
| `scatter_selected` | 2 | 20.794 | 5.35% |
| `mark_flagged` | 2 | 15.654 | 4.03% |
| `device_scan_lookback_init_u32` | 2 | ~2.14 | ~0.55% |
| `boundaries` | 1 | ~1.21 | ~0.31% |

Exact values: `h06.kernels.csv`. Full-process kernel list (including untimed input encoding/decoding): `h06.all-kernels.csv`. The additional input kernels are `headers`, `offsets`, `scan_offsets`, `finish_offsets`, `encode`, and `fastlanes_decode_column`. Raw timeline: `h06.nsys-rep`, `h06.sqlite`.

## ncu bandwidth / utilization

RTX 6000 Ada nominal memory bandwidth: **960.096 GB/s**. DRAM GB/s is measured bytes / measured duration; sustained-peak percentage is ncu's hardware metric. ncu replay/cache/clock behavior differs from warmed nsys runs; do not substitute its durations into the nsys cost table. Query metrics below are first matching launches, not warmed medians.

| Kernel | Grid x block | ncu duration us | DRAM MB | Achieved GB/s | % nominal peak | % ncu sustained peak | SM throughput % |
|---|---|---:|---:|---:|---:|---:|---:|
| `reduce` | 1 x 256 | 436.864 | 3.722 | **8.52** | **0.89%** | **0.94%** | **0.057%** |
| group arguments | 852 x 256 | 222.752 | 42.731 | 191.83 | 19.98% | 21.06% | 55.64% |
| `mark_flagged` | 23443 x 256 | 30.304 | 24.008 | 792.22 | 82.52% | 87.07% | 28.73% |
| `device_scan_lookback_u32` | 3126 x 128 | 53.600 | 24.043 | 448.56 | 46.72% | 49.31% | 40.58% |

All captured metrics, including input-codec kernels: `h06*.ncu.csv`; derived query bandwidth: `h06.bandwidth.json`. Binary reports: `h06.ncu.ncu-rep` (first 24 preprocessing launches), `h06-query.ncu.ncu-rep` (decode/argument/selection launches), `h06-reduce.ncu.ncu-rep` (explicitly filtered scalar reduction). The separate reduction capture is necessary because preprocessing consumed the initial launch limits.

## Three biggest attributable costs / why

1. **One-CTA scalar reduction: 198.824 us, 51.15% of kernel time.** Both nsys and ncu show exactly one 256-thread block, despite 142 available SMs. `experiments/gpu/sequence-runtime/src/group.rs:423–449` launches one CTA per group; h06 has one keyless group. Its lanes serially stride the entire selected order (`:565–578`) then perform shared-memory tree reduction with synchronization (`:580–604`). The measured 8.52 GB/s and 0.057% SM throughput rule out saturated device bandwidth as the main limit. [INFERENCE] Lack of cross-CTA parallelism is the primary reason a hand scalar reduction wins; a device-wide partial/final reduction would expose substantially more hardware.
2. **Materializing aggregate arguments instead of fusing filter+sum: 124.257 us, 31.96%.** The generated `__cq_group_arguments` kernel writes arguments/live flags over the source extent before folding; the fold subsequently accesses arguments through an order vector (`group.rs:257–292,565–567`). The observed argument kernel has 42.731 MB DRAM traffic and only ~20% nominal bandwidth, with 55.64% SM throughput. [INFERENCE] Predicate/decode/arithmetic/materialization work, rather than a simple streaming sum, explains this second cost; counters do not isolate each instruction subtype.
3. **Host-controlled selection/group-boundary pipeline: 76.685 us non-kernel envelope time, plus ~65.65 us selection/boundary kernels.** `group.rs:283–313` performs live-row selection, downloads its count and allocates/copies a compact order vector; `:385–410` then marks/selects group boundaries and downloads another count even for a keyless aggregate. These barriers and separate kernels are absent from a fused hand reduction. The selection kernels themselves can run at 47–83% nominal bandwidth, unlike `reduce`, but still create extra passes and dependencies. `:317–322` also allocates general grouped-fold scratch before launching reduction.

Code references are relative to `/data/mini_linq-wt/m0-baseline/`. No hand-kernel binary was supplied or executed here, so the historical 16–74x ratio is not remeasured. The profiles identify why this compiled path has overhead; they do not establish a new speedup figure.
