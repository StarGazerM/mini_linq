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

The resident H05 R and identity-chain callers author their Sequence programs
directly, then apply checked output/grade rewrites. They do not infer grades
from generated temporary names.

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

H05 A/B retain explicit TODO rejection paths and independently selectable
rejection fixtures (`h05-a-sequence-rejection`, `h05-b-sequence-rejection`).
Their reduced Nation provider lacks a recorded resident range layout. The
compiled triangle keeps its original `i64` relation and a
`triangle-sequence-rejection` fixture; range payloads require recorded
`u32`/`i32` dense codes. No narrowed schema, remapping, or alternate plan is
substituted. These unavailable programs have no Sequence timing claim.

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

All three rejection features were compiled independently and rejected by the
translator, not merely by their runtime TODO wrappers:

| Caller | Exact error |
|---|---|
| H05 A | `T-Gpu: r2 has no recorded resident range layout` |
| H05 B | `T-Gpu: r2 has no recorded resident range layout` |
| Signed-i64 compiled triangle | `T-Gpu: range payload needs a recorded u32/i32 dense code` |

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
