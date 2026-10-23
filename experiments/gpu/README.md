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

Release build dependencies also use optimization level 3: the build-time
compression catalog shares CUDA-containing crates with the runtime, and their
device address-space lowering requires optimized codegen.

### Planner-driven 11-query TPC-H suite

`tpch` is one crate with one binary, `cq-gpu-tpch`. It imports the CPU
`retail_kernel!` invocations directly with `include_str!`; no second query body
is duplicated. Build-time compilation uses the same CQ macro substitution,
checked CQ → Physical → Structures passes, generic catalog selection, Sequence
lowering, grading and translation. The default build generates only `advised`
executors, using explicit pipeline choices followed by decomposition-only
completion. Automatic is opt-in pending box 11b: build with
`CQ_GPU_TPCH_AUTOMATIC=1` to also generate `automatic` executors using
`cq_sequence_plan::plan`. No automatic lookup rewrite is required to compile or
measure the advised baseline.
The manifest loader records its integer-only `DeviceColumns` host adapters
through the checked retype rewrite; relations declaring `String` retain host
rows. Neither the loader nor translator chooses a plan.

The default loader reads only each imported query's declared columns. `String` columns
are decoded from `.bin` dictionary codes and `.dict.txt` into owned host strings;
integer/code declarations retain codes, and only their text literals are
resolved to dictionary IDs. Integer-only relations keep resident device uploads
and share columns between aliases. Dates use i32 epoch days and decimals use
scale-2 integers. Substring predicates receive real strings, not dictionary IDs.
This affects H03 (`c_mktsegment`), H05 (`r_name`), H07/H07-block (`n_name`),
H08 (`p_type`, `r_name`), H09 (`p_name`), H21-positive (`o_orderstatus`, `n_name`),
D19/D85 (`n_name`), and D27 (`p_brand`, `n_name`); D72-inner has no string columns.
Catalog provider tokens declare their accepted element types: resident range
providers accept scalar codes, while host providers retain unsupported rows.
String literal equality and opaque string predicates execute through the shared
host row printer. Once a host projection drops the strings, its numeric rows
are uploaded for subsequent GPU work; host evaluation and upload are included
in the build timing interval. The synthetic String-guard regression checks
this boundary against Sequential and explicit expected keys.

H05 uses the explicit `tpch_domains` loader profile in **both** modes: the shared
`gpu_compress_host::tpch::schema`/`gpu_compress::Catalog` stores join keys as dense u32 domain IDs
and the date/Region dictionary code as i32, exactly as the authored benchmark.
The imported query's types and dictionary literals are adapted to that schema;
GPU and Sequential outputs are decoded back to source keys before comparison.
The loader profile is recorded in the pipeline artifact, not selected by the
planner. H05's advised pipeline records Region → Nation → Customer → Orders →
Lineitem → Supplier, retains the Customer prefix, and uses the R schedule:
one 256-row input partition and 1024-row/items-4 segmented Orders and Lineitem
expansions. Its unchanged fully-bound Supplier probe receives the checked
one-level `membership(...)` Structures certificate, so Sequence lowers it to a
filtering lookup without expanding witnesses. It does not call the authored
H05 implementation. H09 advises Part → Lineitem → PartSupp → Orders → Supplier →
Nation and retains the filtered Part leaf using its checked numeric carrier;
the substring predicate remains at its authored source position.

The binary accepts repeatable `--query <name>`, `--scale sf1|sf10`, and
`--mode automatic|advised` filters. Omitted filters select every query, both
scales and all built modes (advised only by default). Every run writes its selected inventory to the optional
positional results path, including filtered runs. `--baseline FILE` retains
prior per-query best measurements and reports ratios; without it, an existing
output file is captured before replacement. `--authored FILE` reads the
`gpu-h05-compiled` R measurements and reports the same-scale comparison.
Advised H05 performance acceptance requires SF10 total ≤ 10.35 ms and, when
provided, no slower than the measured authored R. A failed timing gate keeps
the exactness/timing evidence and exits unsuccessfully.

`TPCH_GPU_ADVICE=/path/advice.json` selects build-time per-query overrides.
Each entry is a pipeline object, for example:

```json
{
  "h03": {
    "plan": "no_reduce, order(Customer, Orders, Lineitem)",
    "physical": [],
    "structures": [],
    "sequence": []
  }
}
```

`plan` uses the CPU `order(...)`, `no_reduce`, `reduce(T with S)` and
`skip(T with S)` conventions. Physical advice accepts `retain_streams(...)`,
`hoist(...)`, `build_host_reads(...)`, `canon_prefixes(...)` and
`smallest_intersections(...)`; Structures advice accepts `sorted_indexes(...)`,
`membership(...)`, and `retype(...)`. Sequence entries parse the planner's
public `Advice` syntax, including `store`, `store_level`, `tile`, `partition`,
`segment`, `output`, `grade`, and `evidence { ... }`. CPU workload
`advise plan(...)` hints are imported into the report's `cpu_plan_advice`
field for explicit selection. They do not automatically replace the measured
GPU baseline: CPU-specific reduction hints can produce unsupported GPU builds.

Each accepted report includes `sequence_input`, `sequence_trace`,
`sequence_planned`, symbolic cost, and a resolved `pipeline` object. Copy
`{ \"query_name\": report.pipeline }` into the override file to replay it;
`complete: true` means its Sequence trace already records the complete
schedule and is not replanned. Manifest row counts are recorded as evidence,
never fabricated selectivities. Replay coverage checks exact program equality.

```sh
export CUDA_HOME=/opt/nvidia/hpc_sdk/Linux_x86_64/25.5/cuda/12.9
cargo oxide build --arch sm_89 -- --release -p cq-gpu-tpch
cargo oxide test --arch sm_89 -- --release -p cq-gpu-tpch -- --nocapture
./target/release/cq-gpu-tpch
```

The binary visits both SF1 and SF10 and writes
[`compare/gpu_tpch.results.json`](compare/gpu_tpch.results.json). For accepted
translations it performs one warmup plus five measured iterations, checks every
iteration exactly against `answer.retail_<query>.tsv` and its `.count`, and
reports median CUDA-event total/build/kernels/dedup milliseconds. Input loading,
answer download, and comparison are outside those event intervals. SF1 also
compares the GPU answer to independent Sequential translation of the identical
Physical using `cq_testkit::conformance::assert_answers_equal`, the external
target API used by H05 (`assert_conformant` itself exposes only CPU targets).
The eleven conformance tests cover both modes and reject unexpected planning
failures; H07's existing unsupported opaque country disjunction remains an
explicit reported rejection. `TPCH_GPU_DATA_ROOT` changes the fixture root at
build time. The new advised acceptance results must be measured; the schedules
and timing ceilings above are requirements, not claimed measurements.

Historical initial inventory, before the subsequent translation fixes: all eleven queries were rejected at both scales, so **no GPU
exactness, conformance agreement, or timing is claimed**. The release build and
13 tests passed (two source/encoding tests plus eleven explicit rejection
reports). No compiler gap was patched around. The table abbreviates only the
large `syn::Type` debug payload in catalog errors; the JSON preserves each full
error, failed phase, complete term, and imported CPU query. DuckDB values are
the existing per-query t24 medians, converted from seconds to milliseconds.

| query | SF1 exact | SF10 exact | GPU total ms SF10 (build/kernels/dedup) | DuckDB t24 SF10 ms | status |
|---|---|---|---|---|---|
| h03 | not run | not run | — | 167 | translate rejected: Sequence lookup lacks checked Physical row provenance |
| h05 | not run | not run | — | 172 | planner rejected: P-Decompose: unmet catalog obligation Provider; `r13`, `[nation_key, supplier_key]`, Range, `DeviceColumns` |
| h07 | not run | not run | — | 137 | translate rejected: T-Gpu: provider predicate requires a recorded contiguous range selection |
| h07_block | not run | not run | — | 139 | planner rejected: BE-Boundary: a dispatch must have one unambiguous outer stored space (definition `r3`) |
| h08 | not run | not run | — | 116 | planner rejected: P-Decompose: unmet catalog obligation Provider; `r11`, `[supplier_key]`, Range, `DeviceColumns` |
| h09 | not run | not run | — | 4472 | planner rejected: BE-Partial: opaque Physical Rust must be evaluated at host before dispatch |
| h21_positive | not run | not run | — | 3942 | translate rejected: T-Gpu: provider bound must be a constant or host parameter |
| d19 | not run | not run | — | 204 | planner rejected: P-Decompose: unmet catalog obligation Provider; `r8`, `[part_key]`, Range, `DeviceColumns` |
| d27 | not run | not run | — | 79 | translate rejected: Sequence lookup lacks checked Physical row provenance |
| d72_inner | not run | not run | — | 292 | planner rejected: P-Decompose: unmet catalog obligation Provider; `r9`, `[supplier_key]`, Range, `DeviceColumns` |
| d85 | not run | not run | — | 191 | planner rejected: P-Decompose: unmet catalog obligation Provider; `r14`, `[part_key]`, Range, `DeviceColumns` |

### TPC-H data

TPC-H datasets are `/data/tpch-gpu/sf1` and `/data/tpch-gpu/sf10` (ignored,
never committed). Regenerate all authored CQ projections and reference answers
from each directory's existing `dbgen.duckdb`, from the repository root:

```sh
python3 experiments/gpu/compare/export_tpch.py --root /data/tpch-gpu --sf 1 10
```

The exporter reads every `features/workloads/src/queries/retail/tpch/*.rs`
relation declaration and its exact `sql r#"..."#` reference, including guards.
These are positive CQ kernels, not the original aggregate TPC-H/TPC-DS queries.
The required union of columns is:

| Table | Exported columns |
|---|---|
| customer | c_custkey, c_nationkey, c_mktsegment |
| orders | o_orderkey, o_custkey, o_orderdate, o_orderstatus |
| lineitem | l_orderkey, l_partkey, l_suppkey, l_shipdate, l_quantity, l_receiptdate, l_commitdate |
| supplier | s_suppkey, s_nationkey |
| nation | n_nationkey, n_regionkey, n_name |
| region | r_regionkey, r_name |
| part | p_partkey, p_brand, p_type, p_name |
| partsupp | ps_partkey, ps_suppkey, ps_availqty |

`manifest.json` schema version 2 contains `sf`, `byte_order: "little"`,
`binary_bytes`, `constants` (the original H05 date bounds and ASIA ID),
`relations`, and `queries`. Each relation has `rows` and a `columns` array;
each column has `name`, physical `type` (`i32`/`i64`), `logical_type`,
`encoding`, and `file`. Binary files are headerless aligned columns in original
database row order. Encodings:

* `plain`: signed integer keys/quantities, with the declared Rust width.
* `days_since_epoch`: signed i32 dates, `epoch: "1970-01-01"`, `unit: "day"`.
* `dictionary`: i32 indices into the named UTF-8 `dictionary` (`*.dict.txt`),
  one lexicographically sorted value per line, `dictionary_index_base: 0`;
  `dictionary_entries` records cardinality. Dictionaries are per column.
* `scaled_integer`: i64 decimal values with `scale: 2`; divide by 100 to decode.
  Thus the D72 `l_quantity > 20.00` guard compares encoded values to 2000.

Every `queries.<query>` entry records source path, exact SQL,
`required_columns` grouped by table (the complete per-query input inventory),
answer filename, distinct answer count, and timing filename. Answers are
`answer.retail_<query>.tsv` plus `.count`, sorted by all output columns.
`duckdb_timing.retail_<query>.json` retains the original timing schema:
engine/version, SQL, seconds, one warmup and five measured full SELECT DISTINCT
runs, and thread-keyed warmup/samples/median at 1 and 24 threads. Raw CLI logs
are retained per query/thread count; timings include normal result emission,
not a COUNT wrapper or the separate sorted answer export. H05 additionally
retains `answer.tsv`, `answer.count`, and `duckdb_timing.json`.

All pre-existing `.bin` files are SHA-256 checked before and after export;
`binary_checksums.json` records both maps and requires equality, preserving
the H05 binary inputs byte-for-byte. The manifest is published only after all
columns, checksums, answers, and timings complete. Temporary projected TSVs
are removed after conversion.

The completed SF1/SF10 export has the following distinct counts and DuckDB
median wall times in seconds (five samples after one warmup):

| Query (`retail_` prefix) | SF1 count | SF1 1 thread | SF1 24 threads | SF10 count | SF10 1 thread | SF10 24 threads |
|---|---:|---:|---:|---:|---:|---:|
| d19 | 8,962 | 0.086 | 0.096 | 90,015 | 0.600 | 0.204 |
| d27 | 357 | 0.078 | 0.023 | 3,575 | 0.806 | 0.079 |
| d72_inner | 5,344 | 0.232 | 0.119 | 53,966 | 1.648 | 0.292 |
| d85 | 8,962 | 0.058 | 0.035 | 90,015 | 0.655 | 0.191 |
| h03 | 11,620 | 0.054 | 0.034 | 114,003 | 0.735 | 0.167 |
| h05 | 7,242 | 0.062 | 0.024 | 72,983 | 0.681 | 0.172 |
| h07 | 5,501 | 0.064 | 0.030 | 54,142 | 0.724 | 0.137 |
| h07_block | 5,501 | 0.062 | 0.029 | 54,142 | 0.724 | 0.139 |
| h08 | 2,603 | 0.069 | 0.018 | 24,254 | 0.918 | 0.116 |
| h09 | 319,402 | 0.694 | 0.466 | 3,261,612 | 7.327 | 4.472 |
| h21_positive | 302,284 | 0.514 | 0.418 | 2,930,274 | 5.438 | 3.942 |

| Bytes (logical file sizes) | SF1 | SF10 |
|---|---:|---:|
| Binary columns | 325,773,800 | 3,256,586,628 |
| String dictionaries | 6,753,882 | 67,493,549 |
| Named query answer TSVs | 12,386,099 | 143,352,887 |
| Entire directory, including database, logs, and H05 copies | 757,972,236 | 7,876,436,632 |

All 13 pre-existing H05 `.bin` files at each scale passed before/after SHA-256
equality. The manifest includes all 11 query inventories and all 8 tables.

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

Checked `compact` membership reads retain their Physical row provenance through
later maps, projections, and payload lookups, even when one provider serves
both existence and payload reads. Row-valued bounds in a recorded device
`compact` are evaluated in each lane against that lane's row; they are not
converted into host provider-build intervals. Host provider selections still
require the recorded contiguous range supported by their adapter.

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
The recorded directory still requires dense nonnegative u32-range keys.
`PackedCanon` advertises a 128-bit packed-key capability. At setup, the sum
of the recorded codec-bound widths selects one u64 lane (up to 64 bits) or
two lanes (up to 128 bits); larger headings return a setup error before
packing or dispatch. Narrow keys retain radix sorting, while wide keys use
exact row sorting and adjacent deduplication. Cross-lane fields remain
zero-copy projections for later providers. This is provider-internal storage
selection, not a translator or planner choice; no schema narrowing, ID
remapping, or alternate query plan is substituted.

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
