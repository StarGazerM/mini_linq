# Experimental GPU Sequence backend

The main workspace uses checked Sequence programs, resident compressed columns,
and explicit GPU grading. `cq-gpu-sequence` exposes `machine()` and the direct
`translate(&Program, &Analysis)` API alongside the pipeline macros
`machine_grade!` and `translate!`. The grading macro checks the authored grades
against `machine()`; it does not assign placements or choose a plan.

A GPU pipeline explicitly selects `sequence = ::mini_linq::passes::sequence`,
records its decomposition with `decompose = [...]`, then selects
`grade = ::cq_gpu_sequence::machine_grade` and
`print = ::cq_gpu_sequence::translate`. Sequence rewrites carry decisions:
`store_level` places storage, `tile` records tile size and items, `choose`
records guarded alternatives, `output` records publication protocol/capacity,
and `grade` records machine placements. Canonicalization remains a Sequence
`canon` term with its recorded implementation. Translator rejection is not a
license to substitute another query or plan.

## Build and run

From this directory, with the pinned cuda-oxide backend:

```sh
export CUDA_HOME=/opt/nvidia/hpc_sdk/Linux_x86_64/25.5/cuda/12.9
cargo oxide build --arch sm_89 -- --release --workspace
cargo oxide test --arch sm_89 -- --release --workspace
```

TPCH datasets are `/data/tpch-gpu/sf1` and `/data/tpch-gpu/sf10`.
`h05-reference` remains the independent H05 reference implementation.
The Sequence H05 example in `sequence/examples/h05.rs` is also available with
`--features cq-gpu-sequence/cuda-oracle`.

The executable H05 conformance test uses the checked-in
`sequence/tests/support/h05_program.rs` fixture for CUDA and translates that
fixture's **same Physical program** through default CPU structure selection,
Sequence lowering, and the Sequential translator for its reference:

```sh
cargo oxide test --arch sm_89 -- --release -p cq-gpu-sequence --test h05_conformance -- --nocapture
```

It loads `/data/tpch-gpu/sf1` once with the same catalog and resident bindings as
the H05 example, then downloads those exact encoded input rows for the CPU.
The shared `cq-testkit` answer comparator checks canonical answer sets in
`(order, supplier, nation)` order. This is conformance to the Physical reference,
not a claim that the CPU runs the GPU's provider types or identical Sequence
terms. It does not use `answer.tsv` or a handwritten join as its reference.
If the dataset directory is absent, it prints an explicit `SKIP` before CUDA
initialization; loading or CUDA failures with the directory present fail the
test. It also runs in the default `cargo oxide test --arch sm_89 -- --release -p cq-gpu-sequence`
suite without feature flags; only the standalone example requires `cuda-oracle`.

## Legacy Datalog boundary

`legacy-datalog/` is an excluded, independent workspace, not a compatibility
path in the Sequence pipeline. It contains the GPU Datalog bridge and its
oracle/ddisasm consumers, the old Structures printer, blocked-run indexes,
legacy runtime oracles, and the handwritten H05/triangle baselines that use
those indexes. Maintained frozen/leveled indexes have no Sequence term yet;
these consumers therefore have **not** migrated. Their original algorithms
and historical results are preserved rather than silently changed.

The shared column runtime exposes its old execution/index APIs only through
its optional `legacy-datalog` feature. Main-workspace callers do not enable it.
Build the old bridge separately, from `legacy-datalog/`, using the same CUDA
configuration and `cargo oxide build --arch sm_89 -- --release --workspace`.
Historical pre-cutover documentation is in
[`legacy-datalog/README.md`](legacy-datalog/README.md).

## Authored advice and rejection boundaries

The resident H05 A/B/R and identity-chain callers author their Sequence programs
directly, then apply checked store/output/grade rewrites. They do not infer
grades or provider choices from generated temporary names.

| Former decision | Recorded Sequence decision |
|---|---|
| Retained prefix/build | Explicit `store` at its authored level |
| Sorted/prefix lookup representation | `RangeProvider` with the same recorded key levels |
| Expansion batch and `tile(256,4)` | Explicit `tile 1024 items 4` and flatten boundaries |
| Device deduplication | `canon` with `PackedCanon` |
| Output allocation | Explicit `output(count)` for exact count publication; bounded/reserved protocols must be authored separately |
| Execution placement | `grade(gpu, ...)`, checked by `machine_grade` |

For the resident identity chain, the former `capacity(Input)` bound is
interpreted explicitly as `output count;`: the identity result length is
known from its input. No unsupported dynamic-capacity term is introduced.

`frontier(8)`, `directory(rank)`, and the legacy resume/retry policies have no
one-to-one Sequence advice mapping. They are not silently passed to the new
terminal or asserted to be equivalent to count publication. No size-guarded
alternative was authored for these callers, so none is invented with `choose`.

H05 A/B explicitly publish the reduced Nation semijoin `r2` with an authored
`R2 = store ...` graded at device (EXT-SeqStore), and a recorded
`RangeProvider [nation]` Structures choice. The store is present before source
contract checking; a later rewrite cannot establish scope for an earlier
`lookup r2`. The provider is built only after that store is published; the
translator neither chooses the representation nor substitutes R.

Recorded range providers can consume published device row stores or canonical
packed-key stores, not just base relations. Owning source descriptors project
row-major fields with a stride and packed fields with their recorded
base/shift/width; they retain the publication without an intermediate
row-to-column copy or full unpack. Selection, key sorting, and payload gathering
read those same views. Store publication must precede provider construction;
no provider layout or new Sequence term is inferred.

The compiled triangle keeps its original `i64` relation and now translates
normally, with positive translation and executable tuple-comparison tests in
place of the rejection fixture. Range payloads support `u32`, `i32`, and full
signed `i64`, including values outside the 32-bit domain. Their fused gather
and FOR packing preserves 64-bit extrema and decodes without narrowing.
`triangle-compiled EDGES --check` uploads native i64 IDs, executes the authored
Smallest/First plan, and compares actual GPU tuples with its CPU reference.
It reports one discarded warmup and five measured resident calls, including
per-call and median GPU build/kernel/dedup/total timings and query wall time.
`--check` separately times the CPU tuple reference and checks every GPU call.
Input loading/upload, answer download, and reference checks are excluded from
the query samples.
The recorded directory still requires dense nonnegative u32-range keys, and
canonical output fields must fit one packed u64; no schema narrowing,
ID remapping, or alternate plan is substituted.

## Device-store and i64 acceptance

On NVIDIA RTX 6000 Ada Generation (driver 580.178.04), the `sm_89` release
workspace build and test commands pass, including 22 tests and all doc tests.
The positive tests cover provider construction after device-store publication,
i64 triangle translation/execution, signed 64-bit payload extrema, and strided
and packed stored-column descriptors. `python3 doc/formal/check.py` reports
228 synchronized rules; no new translation decision or rule was introduced.

These H05 measurements use one discarded warmup and five measured resident
calls. Every A/B/R call and the reference compare complete tuples against
`answer.tsv`; all are exact. GPU time includes base provider construction,
dispatches, device-store provider construction, and final canonicalization.
The latter provider builds occur inside the reported kernel phase, after the
base-build timing boundary. Upload and answer download are excluded.

| Workload | Exact rows | Sequence GPU ms | Reference GPU ms | Sequence wall ms | Reference wall ms |
|---|---:|---:|---:|---:|---:|
| H05 A SF1 | 7,242 | 2.526368 | 1.922624 | 2.587867 | 1.931820 |
| H05 B SF1 | 7,242 | 1.796928 | 1.922624 | 1.855975 | 1.931820 |
| H05 R SF1 | 7,242 | 2.129568 | 1.922624 | 2.192349 | 1.931820 |
| H05 A SF10 | 72,983 | 18.664608 | 9.899072 | 18.731918 | 9.909329 |
| H05 B SF10 | 72,983 | 10.202752 | 9.899072 | 10.264289 | 9.909329 |
| H05 R SF10 | 72,983 | 10.323200 | 9.899072 | 10.388337 | 9.909329 |

`triangle-compiled /data/dataset/graph/com-dblp.ungraph.txt --check` retains
1,049,866 distinct native-i64 edges and produces exactly 2,224,385 tuples,
equal to its independent CPU reference on every call. One warmup is excluded
from five GPU samples; the CPU reference is measured once.

| Triangle workload | Build ms | Kernel ms | Dedup ms | GPU total ms | Query wall ms | CPU reference ms |
|---|---:|---:|---:|---:|---:|---:|
| DBLP, native i64 | 2.062880 | 3.302496 | 0.627648 | 6.020128 | 6.082209 | 244.443683 |

The H05 samples are retained in
`compare/gpu_sequence_device_stores.results.json` and
`compare/gpu_sequence_device_stores.reference.sf{1,10}.json`.

## Cutover validation

On NVIDIA RTX 6000 Ada, `sm_89`, both workspace build and test commands above
passed. The main workspace has 16 passing tests, including one test exercising
three schema UI cases. `python3 doc/formal/check.py` passed with a 226-rule
inventory; the GPU interpretation marker now lives in `sequence/core`.
The generated CUDA exports have expansion-specific names so multiple query
expansions in one binary do not collide.

H05 medians below use one discarded warmup plus five measured calls. Both
implementations compare full answer tuples against the independent
`answer.tsv`; the identity-chain answer is checked after explicit download.
GPU times include resident index build, query, and deduplication, not input
loading/upload or answer download. Chain time is separate, not added to R.

| Workload | Exact rows | Sequence GPU ms | Reference GPU ms | Sequence wall ms | Reference wall ms | Chain GPU ms |
|---|---:|---:|---:|---:|---:|---:|
| H05 R SF1 | 7,242 | 2.086272 | 1.895552 | 2.148636 | 1.906872 | 0.181888 |
| H05 R SF10 | 72,983 | 10.139392 | 9.759232 | 10.206828 | 9.769801 | 0.228640 |

`printer-smoke` passed its nonempty and empty cases. `printer-oracle` passed
resident projection for empty/sorted/unsorted/duplicate inputs and repeated
head projection. These do not claim the old runtime certificate, multiword
key, or resumable-provider coverage.

Historical cutover validation compiled the then-unsupported H05 A/B and
signed-i64 triangle rejection fixtures independently. Those fixtures have now
been removed in favor of positive device-store provider and i64 translation
tests; the historical rejection results in the archived cutover summary are
not current support boundaries.

The **legacy** workspace also built independently. Its Datalog oracle passed
eight scenarios, and its handwritten triangle oracle passed seven cases
(including 3,999,859 answers in the million-degree case), against independent
CPU count/materialization checks. The cvc5 ddisasm smoke run checked all eleven
outputs exactly against the CPU implementation: fixed point 8,482.319 ms,
upload 21.342 ms, CPU reference 28,605.944 ms, 878 checked rounds. That is one
smoke run, not a median or a Sequence performance claim.

Machine-readable evidence:
[`summary`](compare/gpu_sequence_cutover.summary.json),
[`R and chain samples`](compare/gpu_sequence_cutover.results.json), and
[`SF10 reference samples`](compare/gpu_sequence_cutover.reference.sf10.json).
SF1 reference medians/samples were captured from reference stdout in the
summary; its single-object output file was overwritten by the SF10 invocation.
