# Experimental GPU compiler and runtime

This standalone workspace contains the checked Structures-to-Rust GPU printer,
resident compressed columns, and hand-written comparison kernels described in
`../../doc/GPU-DESIGN.md`. GPU execution is an explicit terminal-printer choice;
the core default pipeline is unchanged.
All kernels are Rust compiled by cuda-oxide. The approved de-risk workaround
uses dense counting-sort builds and exact atomic-CAS dedup instead of RCCL's
device algorithms, whose merged PTX hit the compiler bug documented below.
RCCL's host-only `rccl-memory` allocator is used without a device bundle.
No CUDA C, NVRTC, OptiX or CPU join is in the measured path.

## Resident Datalog

`cq_gpu_datalog::datalog!` reuses the source contract, stratification,
Boolean semi-naive cover, layout-demand analysis, and independently checked
fork in `features/datalog`. It does not introduce a second scheduler.
The scheduler explicitly selects `insert_new device_set_all_columns` and
`negation_keys device_set_owned`; authored CPU HashSet schedules are rejected
by the GPU bridge rather than silently reinterpreted. The CPU default remains
`hash_set_all_columns` / `hash_set_borrowed`.
The entry accepts a default named `pipeline { ... }` with the same phase syntax
as `mini_linq::compile!`, followed by optional
`hole(rule N, delta K) pipeline { ... }` or `hole(rule N, seed)` overrides.
Missing phases inherit the default; an explicitly empty pass list does not.
Selectors resolve only against recorded schedule identities. Unknown/non-delta
occurrences and duplicate overrides fail. Structures queues may omit default
optimizers; the selected checked passes and GPU printer validate device providers,
not the bridge's membership test against a preferred optimization pipeline.
`__CQ_DATALOG_HOLE_SELECTORS` prints copyable selectors beside variant numbers.

The default pipeline uses `order(le)`, sorted/hashed/prefix providers,
`retain_lookups`, `expand_lookups(256)`, and the ordinary
`retype(dedup => DeviceSet<_, Plain, Compacting>)` pass argument. Its printer
uses `tile(256,4), frontier(8), directory(rank)` with resumable Compacting.
An override can select `Fused` through that same `retype` argument and
`output(retry)` through its own printer invocation. The bridge never parses
either argument. It appends only `completion(round)`, dictionary, pool and
negative-key schemas; the consuming printer rejects duplicates rather than
silently overriding caller advice.

Scheduling advice is routed separately, for example
`schedule = ::cq_gpu_datalog::schedule(delta_first, hoist_invariants);`.
Only that pass interprets these arguments. Without advice it retains written
drivers; GPU consumers explicitly request delta-first. Each variant records
`driver N` and its source `rule`/`delta` identity. Moving a delta never crosses
negation, a guard or a binding clause. Independent checks validate driver and
delta occurrence, complete body erasure and layout demands.
`__CQ_DATALOG_SCHEDULE_IR` retains the checked schedule.

Caller plan/IX/lower/select macros execute once through ordinary continuations.
The bridge gathers their checked Structures for maintained-index demands, then
splices each hole's remaining opaque Structures/represent/stage/print queue.

The fork records `MaintainedFrozenDeviceIndex` plus an exact `frozen_layouts`
list for each GPU stratum. Keyed full layouts of EDBs and completed lower strata
are projected and indexed once before the seed/round loop, then passed as
`FrozenDeviceIndex` host reads. Changing keyed full layouts instead record
`MaintainedLeveledDeviceIndex` and an exact `leveled_layouts` list. They are
constructed once per SCC and receive each round's admitted delta after all
readers and admission checks. `leveled_ratio(4);` is explicit bridge/fork advice
(default four, minimum two), recorded with SCC lifetime and layout in the
emitted `__CQ_DATALOG_LEVELED_LAYOUT_IR` certificate. Unkeyed full scans and delta
layouts keep contiguous resident projections. Frozen certificates retain
`__CQ_DATALOG_FROZEN_LAYOUT_IR`.
Actual checked Structures demand sets are partitioned into a minimum
subset-chain cover, independently checked for exact prefix coverage.
For example, `{varr,varp} ⊂ {nxtBlk,varr,varp}` becomes one sorted run with levels
`[varr,varp] → [nxtBlk]`, serving the early semi and later full probe. Incomparable
demands use separate runs. Each Host provider type records its complete run
levels, including levels after a shorter read's requested prefix; printers read
those certificates without selecting layouts or rebuilding frozen providers.
`leveled_hosts(Relation => [[1,2],[0]], ...)` records the corresponding exact
`LeveledDeviceIndex` host type. Lookup, membership, and cooperative expansion
iterate its immutable snapshot runs using the existing per-run prefix search.
Kernel arguments carry `*const LeveledRunsView` from `snapshot_ptr(levels)`;
`leveled_count` and `leveled_run` load the device-resident descriptor on demand.
The fixed-capacity run array is not copied by value into each kernel/helper call.
Membership succeeds if any run matches. Cooperative expansion streams one run
at a time through a bounded 256-lane descriptor: its header retains the uniform
run id and the existing per-input frame retains that run's exact range. An
in-device run loop skips empty runs with `leveled_any`; it does not launch a
kernel per run or multiply shared descriptor storage by the maximum run count.
The root `tile(256,4)` decode remains unchanged; each root item streams its runs
through the stage, including run-id and row restoration on resume.
`retain_lookups` retains packed range sets (17 u64 words per lane) between an
eager semi and same-key force over the identical snapshot layout; the force
does not search again. Unwitnessed staged reads do not allocate range arrays.
These snapshot-only hosts bind index
metadata, not a fabricated contiguous table. Leveled `Smallest` intersections
and actual scan roots (including constant Point/Range roots) are rejected;
ordinary unkeyed scans must bind admission columns instead.
For frozen positive factors with locally bound negation filters, advise the
schedule pass with `hoist_invariants` (the ddisasm consumer's `hoist-invariants`
feature supplies precisely that pass argument). The existing checked invariant
rewrite materializes these factors before their consumer stratum, allowing
derived indexes to use the same frozen-layout path. Without that advice, LE may
retain a filtered right-side provider inside a round despite an immutable EDB.

### ddisasm scale baseline

`features/datalog/ddisasm` implements the SRDatalog ddisasm slice (23 rules,
39 declared relations, 28 CSV inputs, 11 outputs) in MiniLinq and independent
Ascent engines. It loads only the mapped CSVs and resolves LOAD/STORE/NONE/
PCRelative from each dataset's `str2num.json` (or explicit `--meta JSON`).
The local z3 directory requires `--meta` because its dictionary is absent.
All columns are lossless `i64`
interned identities; apparent offsets are not interpreted arithmetically.
Only LiveVarUsed2 reorders a negative atom to bind its register first.
The unused declared BlockLastInstr input is loaded; SRDatalog's Python Program
auto-discovery omits that input and reports 38 relations/27 inputs.

```bash
DATA=/home/stargazermiao/workspace/SRDatalog/benchmarks/data/ddisasm
cargo run --release --manifest-path ../../features/datalog/Cargo.toml \
  -p datalog-ddisasm --bin ddisasm -- "$DATA/cvc5" --engine both --check
META=/home/stargazermiao/workspace/SRDatalog/integration_tests/examples/ddisasm
cargo run --release --manifest-path ../../features/datalog/Cargo.toml \
  -p datalog-ddisasm --bin ddisasm -- "$DATA/z3" --engine both --check \
  --meta "$META/z3_consts.json"
export CUDA_HOME=/opt/nvidia/hpc_sdk/Linux_x86_64/25.5/cuda/12.9
cargo oxide build --arch sm_89 -- --release \
  -p gpu-datalog-ddisasm --bin gpu-datalog-ddisasm --features cq-gpu-runtime/nvtx
target/release/gpu-datalog-ddisasm "$DATA/cvc5" --check --meta "$META/cvc5_consts.json" --repeat 3
uv run --project /home/stargazermiao/workspace/srdatalog-python \
  python datalog-ddisasm/measure_srdatalog.py cvc5
uv run --project /home/stargazermiao/workspace/srdatalog-python \
  python datalog-ddisasm/measure_srdatalog.py z3
```

`--repeat N` requires at least three timed repetitions. It first runs one
discarded warmup, then reuses the same process, CUDA runtime and pool for all
timed runs. JSON retains the cold report as `warmup`, individual timed reports
as `runs`, and median/minimum values under `summary`; top-level `fixedpoint_ms`
is the timed median and `fixedpoint_min_ms` the minimum. With `--check`, every
execution is fully tuple-compared against one shared CPU reference outside the
GPU timer. Retry capacity history survives repetitions and retry diagnostics are
explicitly marked process-cumulative. NVTX `cold-run` and `timed-run` ranges
enclose individual executions; their nested `fixed-point` ranges isolate kernel
work from uploads and comparison. Without `--repeat`, one execution is reported.

The GPU consumer preserves the same program and uploads all EDBs before its
fixed-point timer. The checked `hashed_runs` rewrite realizes composite equality
indexes, and closed typed scalar captures lower `meta.*` values once on the host.
All Datalog holes default to `DeviceSet<_, Plain, Compacting>`. The ddisasm
consumer selects the written default planner, hashed providers and resumable
Fused output for `hole(rule 3, delta 0|1|2)` (variants 4–6) and
`hole(rule 19, delta 1)` (variant 22), through separate per-hole pipelines.
The frozen providers in variant 22 use explicitly recorded
`expand_lookups(128)` stages; dynamic hashed providers use ordinary automatic
batch-128 staging. Maintained frozen indexes remain available to either planner.
These use ordinary `plan`, `structures` and `print` pass arguments, not bridge
policy. The checked schedule retains its independent delta-first advice.
Other holes use Compacting with the default pipeline's `output(retry)`.
A default pipeline can select
Fused for all holes through the same pass argument; there is no dedup macro
prefix or Cargo feature channel. These choices never change the Datalog rules.

After stopping the fixed-point timer and popping its allocator counters, the
consumer reads `fused_retry_reports()` from the same pool-keyed cached runtime
used by the CQ holes. It does not initialize or reset that cache before the timed
run. The existing `fixedpoint_ms`, allocator peaks and `exact_cpu_check` outputs
are preserved. Additional JSON keys are:

- `fused_retry_attempts` and `fused_retry_relaunches`: totals across reported sites.
- `fused_retry_wasted_kernel_ms`: failed-attempt GPU event time (`timings` feature; otherwise zero).
- `fused_retry_peak_attempt_bytes`: maximum attempt-owned requested allocation
  size across reported sites, not a sum of their individual peaks.
- `fused_retry_reports`: stably sorted per-hole/site reports containing `hole`,
  `site`, `calls`, `attempts`, `relaunches`, `max_attempts`, `attempts_per_call`,
  `wasted_kernel_ms`, `kernel_ms`, `peak_capacity`, `peak_attempt_bytes`, and
  `last_distinct` (the last successful distinct cardinality).

With `--features cq-gpu-runtime/timings`, the report's GPU events surround the launch callback, including its frontier
reset and in-kernel unique-output compaction, rather than just the query kernel.
They exclude attempt allocation, summary/control readback and later operators.
`kernel_ms` includes
all attempts; `wasted_kernel_ms` includes only failed attempts. Neither is the
wall-clock fixed-point duration. Attempt bytes cover output and ticket-arena
columns, rounded hash slots, metadata/counters, dependency-pointer buffers and
device-pointer arrays.
They exclude resident inputs, frontier storage, other live allocations, pool
reservation and CUDA driver overhead. The fixed-point allocator peak instead
includes resident EDBs and the measured region's requested live bytes; it is not
driver-reserved VRAM. These are reporting scopes, not new measurement results.

**cvc5 GPU equals CPU on all full tuples in all 11 outputs.** The single measured
run took **11.444806 s** for synchronized resident computation, plus **13.844 ms**
upload, with **6,504,636,136 bytes (6.058 GiB)** allocator peak, **878 checked
rounds / 871 nonempty publications**, and 773 prefix compactions. Compared with
SRDatalog's **0.609062 s** prepared synchronized run, this is **18.791× longer**.
Both exclude CSV parsing, H2D upload, result download and answer checking;
MiniLinq's interval includes resident EDB canonicalization, first-use query module
loads, all indexes, admission, compaction and publication. This is a prepared-run
comparison, not an end-to-end speedup. Raw metrics are in
`datalog-ddisasm/gpu-cvc5.json`; merged CPU/SRD/GPU evidence is in
`datalog-ddisasm/results.json`, and detailed traces are summarized in
[`profile/README.md`](profile/README.md). The historical rejection logs are
retained as pre-unblock evidence, not current status.

The z3 GPU full-check was started but stopped when wrap-up was requested, before
it produced a completed report. Its GPU exactness, timings and memory-fit outcome
are therefore **unavailable**, not inferred from the successful CPU baseline.

CPU compute includes index construction and output collection, excluding CSV
loading, input cloning and sorted full-tuple comparison. SRDatalog's prepared
run timer excludes parsing, upload and JIT compilation. These preliminary timings
are not equal-boundary end-to-end speedups; SRDatalog count equality is not an
independent full-tuple oracle.

Single-run preliminary compute seconds on this shared machine:

| Dataset | CPU load | MiniLinq CPU | Ascent CPU | SRDatalog prepared GPU |
|---|---:|---:|---:|---:|
| cvc5 | 5.498 | 36.013 | 50.151 | 0.609 |
| z3 | 9.071 | 2085.226 | 119.720 | 1.947 |

MiniLinq and Ascent matched **all full tuples in all 11 outputs on both datasets**;
every output count also matched SRDatalog. Exact counts and unrounded timings are
in `datalog-ddisasm/results.json`. The large z3 MiniLinq CPU time is measured,
not extrapolated; this baseline does not diagnose its cause.

For an already loaded `edges: DeviceColumns<'_, (i64, i64)>`:

```rust,ignore
let paths = cq_gpu_datalog::datalog! {
    struct Closure;
    relation Edge(x: i64, y: i64) = (&edges);
    relation Path(x: i64, y: i64);
    output Path;
    Path(*x, *y) :- Edge(x, y);
    Path(*x, *z) :- Path(x, y), Edge(y, z);
};
// `paths` owns a resident device relation; download only when requested.
let rows = paths.to_host()?;
```

The consumer package must use the GPU workspace's pinned toolchain and depend
on `cq-gpu-datalog` plus the generated CQ/runtime crates, as demonstrated by
`datalog-oracle/Cargo.toml`.

Every EDB must be resident `DeviceColumns`, share one pool/stream/context, and
use compatible dictionary IDs. Grouped CPU hosts are not supported.
Before any stratum, the bridge merges compatible EDB dictionaries and interns
every authored rule string literal in stable source order, including negative
constant filters. Every EDB/empty IDB is bound to this common dictionary without
remapping device IDs. Every CQ hole receives explicit dictionary and pool
seeds before host binding or literal encoding. Distinct literal-producing
rules therefore cannot independently assign conflicting IDs. Incompatible
input dictionary prefixes remain errors; no CPU tuple remap is attempted.
At least one EDB is needed to supply the device context; that EDB may be empty.
A single output returns one owning resident `DeviceColumns`; multiple outputs
return a tuple in authored order, matching the CPU entry's shape. An empty IDB
and a canonicalized EDB are valid outputs.
Explicit user `to_host()` calls are the only tuple-download boundary.

The initial device fragment consists of positive relational joins/projections,
typed copied/literal heads and closed typed integral/bool captures supported
by the GPU printer, supported total
device comparisons, and stratified exact negative membership. Negative keys
are owned projected device values held by `FrozenDeviceMembership`; their exact
membership index is built once per stratum and borrowed by every round's holes.
Nullary negation retains a witness row but indexes the zero-column existence key.
`__CQ_DATALOG_FROZEN_KEY_IR` records this lifetime. The checked fork's dedicated
membership intrinsic is not arbitrary Rust method execution. Query-dependent
arbitrary Rust heads and body bindings,
generators, and observers are not supported. Printer restrictions on scalar types,
integral run keys, and representable key domains still apply.

Each SCC evaluates every hole against immutable full/current-delta snapshots.
Other recursive operands read FULL, including the current delta: nonlinear
variants form a Boolean cover, not a disjoint positional partition.
Admission inserts every hole's candidates into the member's persistent exact
all-column device hash set. Full-word equality (including zero-column existence)
admits each new tuple once across holes, duplicates, and prior full rows.
The set grows and rehashes geometrically; it is not rebuilt each round.
Full columns are append-only native `Plain` buffers with geometric reserve.
`Relation::prepare` queues the new delta into a shared `RoundStatus`; holes never
read the mutable admission set, so preparing one member cannot affect another
member's old FULL/current-delta inputs.
The single checked D2H batch contains one ancestor-error slot and one 16-byte
count/status slot per member. `PreparedRelation::finish` reserves from the exact
distinct count and copies only the new delta into the full tail, copying old full
rows only on capacity growth or explicit branch detachment. Full/delta snapshots
are owner-retaining prefix/suffix ranges with immutable version-local counts and
exact logical capacities; later appends cannot change any published prefix.
A cloned or reused old relation detaches its own storage before appending.
All successors finish before any publication, including empty deltas; convergence
requires every delta to be empty, and ancestor errors remain checked rather than
being interpreted as emptiness. Normal admission work is candidate insertion
plus delta copying, not a full scan; geometric growth is amortized.
Admission and join maintenance remain separate: full keyed changing layouts
receive only disjoint admitted deltas, while their leveled compactions preserve
the complete row multiset. Snapshot publication and buffer retirement are
stream-ordered after prior readers; no historical timestamp compaction is
promised. Unkeyed full scans still use contiguous admission columns, and
non-maintained derived/delta layouts retain ordinary per-hole construction.
When `timings` is enabled, timing events are retained until a round readback, with one final program
completion covering trailing metadata work. There is no per-hole completion
wait or per-member cardinality wait.

`host_wait_count`, `control_readback_count`, `resume_readback_count`, and `compaction_readback_count`
are thread-local diagnostics (reset with `reset_host_wait_count`). They count
explicit runtime waits, not allocator-internal synchronization, uploads, or
caller-requested tuple downloads. Publication/completion costs one readback per
round, independent of hole/member count, plus EDB checks and one final program
wait. General grow/resume attempts still read a four-byte suspension flag before
dependent operators consume their output; even success cannot be deferred under
that protocol without speculative replay or a different scheduling/storage
protocol. Those waits are reported separately, not excluded from total waits.
Recorded `Compacting` adds one distinct-count check per suspended-prefix
compaction; that count is reported separately and remains in total waits.
The runtime oracle varies 1/2/8 holes per member and 1/2/8/32 members and asserts
one wait per bounded-output round, including a terminal empty round.

The `gpu-datalog-oracle` consumer compares complete answers against independent
naive closure/parity oracles for empty inputs, self-loops, chains with alternating
member deltas, cycles, disconnected graphs, duplicates, empty seeds, linear and
nonlinear closure, mutual recursion, lower-stratum negation, multiple outputs,
empty IDBs, EDB outputs, literal heads, and a resident result consumed by a
second compiled GPU query. A separate per-hole Fused `output(retry)` case first
turns 131,072 candidates into 2,048 distinct rows, checking growth from 1,024 to
4,096 slots rather than candidate-sized allocation. The next invocation emits
another 2,048 distinct rows without retry, checking its warm estimate. The test
asserts all 4,096 final answers and accounts for retry metadata readbacks
separately from SCC-round, resume, and compaction feedback.
`--bench` additionally selects a 192-node chain and six connected branching
32-node cycles, with duplicate EDBs and integer values above three billion.
Printed execution time includes EDB canonicalization, all recursive CQ/index
builds, admission, and publication, separately from input upload; there is no
excluded compaction phase. These are reproduction commands:

```sh
cargo oxide build --arch sm_89 -- --release -p gpu-datalog-oracle
./target/release/gpu-datalog-oracle
./target/release/string-dictionary
./target/release/gpu-datalog-oracle --bench
```

The string-dictionary regression first reproduced the incompatible-ID admission
failure for two rules emitting `"a"` and `"b"`. It now verifies those complete
answers, a join of separate literal-producing relations, a negative string
constant filter, a total string equality guard, constant-only projected heads,
compatible EDB dictionary prefixes of different lengths, and owned input text
sharing an ID with a borrowed literal. Both GPU regression programs pass with
the common-dictionary initialization policy.

Compiler-policy regression gate:
`cargo test -p cq-datalog -p cq-lang-datalog -p cq-datalog-schedule -p cq-datalog-fork -p datalog-bridge`
from `features/datalog` passed 98 tests after the device-policy/fork extension.
The separate sequential Ascent/naive CPU differential gate
`cargo test -p datalog-oracle` passed another 17 tests. The GPU macro package
passed four unit tests plus all four compile-fail fixtures (CPU-policy
reinterpretation, grouped host, arbitrary Rust head, and unsafe negation).
The GPU oracle reports a lifetime-wide allocator peak across all scenarios
(including existing pool allocations), allocation count, upload/setup time,
and fixed-point time separately. `nonempty_publications` counts successful
nonempty SCC batches, including seed publications and excluding terminal empty
batches; it is not a per-rule iteration count. Final answer downloads, resident
replay, and independent CPU-oracle work are excluded from fixed-point time.
Run compile-fails with the pinned nightly, not stable:
`cargo test -p cq-gpu-datalog`.

Recorded single-run evidence (`datalog-oracle/results.json`, RTX 6000 Ada,
driver 580.178.04, release `sm_89`): all seven small scenarios and both
moderate inputs matched complete independent oracle answers. The 192-node
chain (191 edges) produced 18,336 TC pairs in 213.342 ms fixed-point time,
0.125 ms upload/setup, with 590 aggregate nonempty SCC publications.
The branching/cyclic input has six 32-node SCCs with local offsets 1 and 5,
five forward inter-SCC edges, and every seventh edge duplicated: 445 input
rows / 389 distinct edges, values `3_000_000_000 + 17*i`. It produced 21,504
TC pairs in 91.526 ms, 0.144 ms upload/setup, with 163 nonempty publications.
Both timings cover the complete 15-output program, not an isolated TC kernel:
linear `TC(x,z) :- TC(x,y), Edge(y,z)` and nonlinear
`TC(x,z) :- TC(x,y), TC(y,z)` run alongside reachability, parity, negation,
and copied/literal-output rules. Lifetime-wide peak pooled device allocation
was 196,559,600 bytes across all nine scenarios (62,298 allocations).
This is correctness-backed reproduction evidence, not a speedup claim.

## Evidence retention

CSV/JSON summaries and historical commands under `compare/` retain their
original measurement scope and paths. Binary profiler captures and regenerated
answer exports are ignored. `compare/srdatalog_sf1_fact.answer.tsv` is the
canonical SF1 answer retained from eleven byte-identical exports (SHA-256
`868d84a39c966787f4b42868a02cc3a7ac6ef8dfbbe75b58a3e57b193ee6cf7b`).
The integration cleanup archived the removed copies locally under
`/data/artifacts/mini-linq-gpu-cleanup`; scripts regenerate them for new runs.
Historical timings are not measurements of the current tree.

## Reproduce

Run from `experiments/gpu` in this checkout. Use an installed `cargo oxide`
matching cuda-oxide revision `5f7c34fddadcf250f2e45d0cffe83a3d27f2bea3`
(install if needed with `cargo install --git https://github.com/StarGazerM/cuda-oxide.git --rev 5f7c34fddadcf250f2e45d0cffe83a3d27f2bea3 cargo-oxide --locked`).
No RCCL checkout or setup script is required.

```sh
export CUDA_HOME="${CUDA_HOME:-/usr/local/cuda}"
export PATH="$CUDA_HOME/bin:$PATH"
export TPCH_DATA_ROOT="${TPCH_DATA_ROOT:-$PWD/data/tpch-gpu}"
cargo oxide build --arch sm_89 -- --release -p gpu-h05 --bin smoke
./target/release/smoke
cargo oxide build --arch sm_89 -- --release -p gpu-tiles --bin tiles-oracle
./target/release/tiles-oracle
cargo oxide build --arch sm_89 -- --release -p gpu-h05 --bin blocked-run-oracle
./target/release/blocked-run-oracle
cargo oxide build --arch sm_89 -- --release -p gpu-h05 --bin dedup-oracle
./target/release/dedup-oracle
cargo oxide build --arch sm_89 -- --release -p gpu-h05 --bin gpu-h05
./target/release/gpu-h05 --synthetic "$TPCH_DATA_ROOT/synthetic"
./target/release/gpu-h05 "$TPCH_DATA_ROOT/synthetic" both
./target/release/gpu-h05 "$TPCH_DATA_ROOT/sf1" both
./target/release/gpu-h05 "$TPCH_DATA_ROOT/sf10" both
python3 audit.py
```

Set `CUDA_HOME` to your toolkit (including an HPC SDK CUDA directory) before
running setup. Nonstandard installations may also export `CUDA_TOOLKIT_PATH`,
`LIBNVVM_PATH`, `CUDA_OXIDE_LIBDEVICE`, or `LIBCLANG_PATH`; none is forced by
workspace configuration. `sm_89` is the measured GPU architecture; change build
`--arch` and audit `--arch` for your GPU. `audit.py` finds `ptxas` via
`--ptxas`, `PTXAS`, `CUDA_HOME/bin`, or PATH, in that order.
If sharing a Cargo build cache with `CARGO_TARGET_DIR`, invoke binaries under
that directory and set a distinct absolute `CUDA_OXIDE_PTX_DIR` per checkout/build.

Export benchmark inputs with `python3 compare/export_tpch.py --sf 1 10`.
`--duckdb` / `DUCKDB` selects the CLI (default `duckdb` on PATH);
`--source-sf1` / `TPCH_SOURCE_SF1` selects an optional existing SF1 database
(default this checkout's `features/workloads/target/bench/data/tpch-1/dbgen.duckdb`).
Absent that database, the DuckDB `tpch` extension generates it.
Exporter `--root` and H05 reference `--data-root` use `TPCH_DATA_ROOT`, default
`data/tpch-gpu` relative to the current directory, and refer to the parent of
`sf1/` and `sf10/`, not an individual scale-factor directory.

### General H05 reference (`h05-reference`)

`gpu-h05` above relies on unchecked shortcuts (first-match probes, a
direct-address nation array, value-range packed dedup, an unchecked
`|Lineitem|` output bound) and builds its indexes inside every query. The
`h05-reference` binary is the general reference instead:

- **Compression (load, untimed):** columns read at their declared types; every
  join key dictionary-encoded to a dense u32 ID per domain (orderkey, custkey,
  suppkey, nationkey, regionkey; order-preserving); every base column FOR
  bit-packed per 1024-row vector and made resident.
- **Program:** the one the compiler records for R N C O L S in written order
  (`order(written, coarse)` with default lowering), not hand-picked choices.
  The constant atom `Region(region_key, ASIA)` reads an index holding only the
  ASIA rows: a selection `r_name = ASIA` (`Catalog::text_id`) before the build,
  keyed by region ID, and `segments` starts from its rows with no in-query
  constant test. The query's date `if` is a selection on Orders before its build. The
  closing `Supplier(supplier_key, nation_key)` probe is full-key: both
  columns are bound, so it is an existence test.
- **Index build (GPU, timed):** six sorted indexes: ASIA Region by region, Nation by
  region, Customer by nation, date-selected Orders by customer, Lineitem by
  order, Supplier by (nation, supplier). One kernel decodes the packed key
  column(s) into (key, row) pairs. A two-column ID key is packed as
  `outer << inner_bits | inner` (rejected above 31 bits). RCCL's stable radix
  pair sort orders the pairs over only the
  key's bits (`device_radix_sort_pairs_buffer`, result read from the current
  double buffer). RCCL `lower_bound` of every outer ID gives dense offsets:
  over `0..D+1`, or `arithmetic_range(0, 1 << inner_bits, D+1)` for a pair
  key.
  `EncodedColumn::from_gather` gathers each payload column by sorted row and
  FOR-packs it in one pass. The Region (`r_name = ASIA`) and Orders (date)
  selections are one flag kernel (`range_flags`), then RCCL
  `select_flagged` over row numbers; its count, read back once, sizes that
  build. RCCL's radix controls are allocated once at setup, sized for each
  relation's full row count.
- **Plan R N C O L S**, chosen by counted prefix cardinalities over all 164
  connected left-deep orders. No key is assumed unique: every lookup is a range
  and every range is expanded; Supplier is the existence test above (binary
  search of the supplier within its nation's range). Counts and offsets are
  u32; any sum that would exceed u32 fails the run through a device flag.
  Block scans in `expand` are RCCL's `block_scan_sum_256`.
- **Query and dedup (GPU, timed):** `segments`, `expand` (emits the answer as
  one packed key per row, widths from the dictionary sizes), one metadata
  readback (emitted count; overflow reruns with the exact count), then RCCL
  radix key sort over the answer key's bits and `unique_copy_by` into the
  resident distinct answer.
- **Reported figure:** host wall clock from compressed resident base columns to
  the resident distinct answer (index build + query + dedup); GPU events give
  the breakdown. Stream-ordered temporaries come from the device's default
  memory pool, whose release threshold is raised once at setup.

```sh
cargo oxide build --arch sm_89 -- --release -p gpu-h05-reference --bin h05-reference
./target/release/h05-reference "$TPCH_DATA_ROOT/sf10" compare/gpu_h05_reference.sf10.results.json
python3 compare/duckdb_h05_exact.py
python3 compare/srdatalog_h05_orders.py "$TPCH_DATA_ROOT/sf1/dbgen.duckdb" /tmp/srdatalog-orders-sf1.json
uv run --project "$SRDATALOG" python compare/srdatalog_h05.py --sf 1 10 \
  --var-order c,n,r,d,o,s --dimension-filters --output compare/srdatalog_h05.exact.results.json
```

It checks the decoded result against `answer.tsv`. Resident at SF10: 280.2 MB
compressed base plus 193.8 MB indexes, for 1,279 MB of declared source columns.

**Same-scope comparison** (2026-10-20, RTX 6000 Ada; DuckDB on the host CPU;
median of five after one warmup; every result checked against `answer.tsv`).
Each system starts from its own resident representation and ends with the
resident distinct answer; load/compression/interning is excluded, index or hash
construction is included.

| System, scope | SF1 ms | SF10 ms |
|---|---:|---:|
| `h05-reference`: GPU index build + query + dedup, wall clock | **1.80** | **9.60** |
| srdatalog-python, `run` (device index build + query), cardinality-chosen order | 10.90 | 165.26 |
| srdatalog-python, `run`, its default plan | 68.15 | 843.41 |
| DuckDB 1.5.6 in-process, 24 threads, `CREATE TEMP TABLE AS SELECT DISTINCT` | 15.25 | 65.83 |
| DuckDB 1.5.6 in-process, 1 thread | 46.43 | 516.05 |

SF10 breakdown for `h05-reference` (Nsight, one iteration, 9.69 ms span,
9.40 ms kernel time): radix onesweep passes 3.90 ms, `gather_encode` 1.69,
`lower_bound` 0.85, `keys_and_rows` 0.72, `expand` 0.48, offsets packing 0.45,
flagged select 0.17, `date_flags` 0.15. Per index: lineitem_by_order 7.32 ms,
orders_by_customer 1.11 ms (was 2.86 ms before the date selection moved before
the build); dedup 0.18 ms. The previous program (all Orders indexed, date
checked during expansion; Region scanned; Supplier by supplier with nation
scanned) measured 1.91 / 11.45 ms.

**Sequence build-resource reuse** (2026-10-21, RTX 6000 Ada, one warmup
and five measured runs, exact against `answer.tsv` at both scales):

| Scale / implementation | Build ms | Query kernels ms | Dedup ms | Total ms |
|---|---:|---:|---:|---:|
| SF1 reference | 1.574 | 0.170 | 0.132 | 1.873 |
| SF1 Sequence | 1.501 | 0.194 | 0.138 | 1.834 |
| SF10 reference | 9.001 | 0.502 | 0.178 | 9.679 |
| SF10 Sequence | 8.996 | 1.170 | 0.197 | 10.365 |

These are CUDA-event phase medians, including device index construction but
excluding input loading/encoding, setup-time count owners and module loading,
and answer download/validation. Total is independently medianed. The generated
program's recorded layouts, capacities, sorts and launch decisions are unchanged.
`RangeWorkspace` retains immutable count owners and exclusively reusable payload
view tables; `PackedColumnsAdapter` does the same for output metadata/views.
Retained results force fresh owners rather than being overwritten. Host uploads
use pool-owned pinned staging, while native range/canon scratch retains exact
live extents across calls.

Nsight, warm timed regions only: median GPU kernel sum 1.625 ms at SF1 and
10.047 ms at SF10. Both scales have 14 `cuStreamSynchronize`,
13 copies (6 H2D, 5 D2H, 2 D2D), 7
`cuMemAllocAsync`, 6 `cuMemFreeAsync`, zero synchronous `cuMemFree`, and zero
module loads per run. All five process-wide module loads finish before the
first timed region; dividing that startup total by six does not measure
per-iteration loading. Remaining stream synchronizations are five RCCL
`lower_bound` completions, two flagged-selection completions, five explicit
count readbacks, one adjacent-unique completion, and final completion.
They include waiting for useful GPU work: the SF10 warm API wait sum is 8.443 ms,
not an additional 8.443 ms beyond the GPU span. In this build-resource baseline,
query expansion (1.170 versus 0.502 ms) was the main remaining gap, not index build.

The Sequence row templates now derive live columns backwards from checked
Physical headings, row predicates, and provider payloads. Dense codec columns
with unsigned 32-bit-compatible Physical types use `u32` device bindings and
shared arrays; other columns retain `i64`. Children read inherited fields
directly through their owner's shared column arrays. Local stores no longer
copy a complete row into shared memory and immediately reload it. In a terminal
`items m` stripe, each lane gathers its live results in fixed-size register
arrays, then the block performs one checked scan and one output reservation
for the entire stripe. Output bounds and scan/owner synchronization remain in
place, as do the recorded partitions, provider choices, dispatches, and host
output protocol. Collective reductions use the same live-column bindings but
retain their recorded reduction scopes and `i64` results, including when their
input is a dense `u32` code. The scalar-fold packing template also parenthesizes
literal bounds in `match` scrutinees so its generated host expression parses.

The live-column template measurement (same GPU, one warmup/five measured runs;
all full answers exact) is:

| Scale / implementation | Build ms | Query kernels ms | Dedup ms | Total ms |
|---|---:|---:|---:|---:|
| SF1 Sequence before | 1.501 | 0.194 | 0.138 | 1.834 |
| SF1 Sequence live columns | 1.502 | 0.252 | 0.138 | 1.892 |
| SF1 reference, same session | 1.578 | 0.170 | 0.134 | 1.881 |
| SF10 Sequence before | 8.996 | 1.170 | 0.197 | 10.365 |
| SF10 Sequence live columns | 8.961 | 0.565 | 0.186 | 9.710 |
| SF10 reference, same session | 9.008 | 0.501 | 0.179 | 9.691 |

SF10 query time is 0.564704 ms versus the reference's 0.500736 ms (+12.8%),
within the 20% target; build/dedup remain near their baseline and total falls
with query time. SF1 query time regresses by 0.058 ms; this is not hidden in the
SF10 improvement. Both `cuda-oracle` (SF1/SF10) and `cuda-retry-oracle` (SF1,
capacity 1024) match every answer, including the no-duplicates check.
The complete datasets on this machine are `/data/tpch-gpu/sf1` and
`/data/tpch-gpu/sf10`; the workspace-relative data directories only contain
DuckDB files. The reference comparison command was
`./target/release/h05-reference /data/tpch-gpu/sf10 /tmp/ref.json`.

The remaining phase gap was profiled, not tuned away. Nsight Compute reports
the main generated kernel at 48 registers/thread, 7,232 dynamic shared bytes,
83.33% theoretical / 73.83% achieved occupancy, and 464.70 us under replay;
reference `expand` uses 62 registers, 19,552 shared bytes, 66.67% / 61.22%
occupancy, and 484.38 us under the same replay settings. These replay durations
are not substituted for the unprofiled phase medians. Nsight Systems' five warm
generated calls have a median 491.14-us main expansion, approximately 27 us
across the two executions of the first query kernel, a 518.116-us summed kernel
duration, and 55.329 us of inter-kernel gaps. The trace shows the preserved
count/readback protocol (`cuMemcpyDtoHAsync_v2`, stream synchronization, metadata
reset, and launch) in those gaps. Shared-memory occupancy is no longer the
limiter; the extra first-dispatch execution and host-controlled publication
remain outside this template-only change.

srdatalog's scope was checked on an Nsight timeline: `prepare` ends with the
host-to-device copies and a device synchronize; the index sorts
(`DeviceMergeSort*`) and the derived ASIA rules run inside `run`.

srdatalog evaluates variable-at-a-time: the root variable's distinct values are
a `ParallelFor`, deeper variables are nested loops. `srdatalog_h05_orders.py`
counts, for every variable order, the distinct prefix bindings per level (a
filter prunes only once its variable is bound) and the work under the heaviest
root value. The cheapest orders start from dimensions but have 1–25 root values
(serial on a GPU). With the derived ASIA filter, `c,n,d,o,s` is the cheapest
order with real root parallelism at SF1: 219,145 bindings, 99,996 roots, at most
29 under one root; the earlier `c,n,o,d,s` costs 475,010 because `o` is bound
before the date filter. Without the derived filter the cheapest such order,
`c,n,r,d,o,s`, makes srdatalog return a wrong answer (0 rows at SF1, 4 at SF10;
`compare/srdatalog_sf{1,10}_cnrdos.json`), so it is excluded.

For triangle references, `--dblp` / `DBLP_DATASET` and `--livejournal` /
`LIVEJOURNAL_DATASET` select source files (defaults `data/graph/com-dblp.ungraph.txt`
and `data/graph/soc-LiveJournal/edge.facts`). Triangle `--data-root` /
`TRIANGLES_DATA_ROOT` instead selects the **generated deduplicated edge output**
directory (default `data/triangles-reference`), not a source directory.
`--srdatalog` / `SRDATALOG` selects the external SRDatalog checkout (default
`../srdatalog-python`); use the same checkout with `uv run --project "$SRDATALOG"`.
All relative CLI paths are relative to the invocation directory.
Archived JSON/log/SQL and historical profiling commands below retain the original
machine paths as provenance; they are not setup requirements or new measurements.

The blocked-run oracle source stays under `blocked-run/src/bin`, but its Cargo
target belongs to the consuming `gpu-h05` package. This avoids the pinned
non-generic module loader's package-name ambiguity between a library and a
binary built from that same package; the initial arrangement produced CUDA
error 500 (`named symbol not found`) while loading the run kernels.
The input directory contains the column files described by `manifest.json`:
raw little-endian i64 keys and i32 dates/nation/region/name dictionary IDs.
`answer.tsv` is the sorted distinct three-column DuckDB answer. Without it, the
runner computes an independent CPU reference. `--synthetic DIR` creates a small,
deterministic, near-uniform dataset with dense primary keys and date boundaries,
and writes its CPU answer in the same format. The benchmark validates both the
device-deduplicated key set and the fully materialized three-column result.

One warmup and five measured runs rebuild all runs, execute the fused kernel,
and deduplicate. Inputs upload once, before timing. CUDA events delimit build,
kernel, dedup and total. Build includes pool suballocation, GPU filtering,
construction, clustering and one batched metadata readback. Dedup includes the
output-count readback, pool suballocation and exact GPU hash insertion.
Event intervals include GPU idle time caused by host synchronization and launch
boundaries. Final answer
transfers and CPU comparison are outside timing. The runner updates matching
SF/plan entries in `compare/gpu_h05.results.json`; `audit.py` fills resource fields
from actual `ptxas -v -arch=sm_89` output and saves that output verbatim.

### Stored representation (`gpu-compress`)

Compression is pre-query storage code in its own crate, `compress`
(`gpu-compress`), never a compiler decision. `Source` reads manifest columns at
their declared types (plus `<table>.<column>.dict.txt` for exported interned
text). `Catalog::compress(pool, source, schema)` builds one order-preserving
dictionary per join domain (`Domain`: size, ID bits, bounds, `value`/`id`)
and FOR-packs every stored column. `tpch::schema()` is H05's schema: five
domains, join keys stored as `u32` IDs, and `r_name`/`o_orderdate` as `i32`.
`Catalog::device_columns::<Row>(table)` binds a stored table as a resident
`DeviceColumns` for compiled queries. Both `h05-reference` and
`gpu-h05-compiled` use it.

Compiled H05 declares relations at the stored types
(`Orders(o_orderkey:u32,o_custkey:u32,o_orderdate:i32)`, ...) and decodes its
answer IDs through the domains before comparing them with `answer.tsv`. Query
constants must have the stored type. `"ASIA"` is the literal `2`; before any run
the host checks it against `catalog.text_id("region.r_name", "ASIA")` and the
manifest. Every plan writes `Region(region_key, 2)`: the constant atom reads a
relation holding only the ASIA rows, built before the query like any other
index, and the query has no ASIA test. The sorted-run rewrite gates (`sorted_runs`, the physical
stream retention gate, `prefix_runs`) and `BorrowedI64TupleCodec`/`I64Codec`
now admit `u32` keys alongside `i32`/`i64`.

Results (`--features cq-gpu-runtime/timings`, median of 5,
`compare/gpu_h05_compiled.resident.results.json`, all exact):

| Plan | SF1 total ms | SF10 total ms | SF10 build ms | before (i64 rows) SF10 total ms |
|---|---:|---:|---:|---:|
| A | 1.18 | 4.89 | 3.01 | 36.64 |
| B | 1.97 | 11.64 | 11.07 | 71.22 |

The device path is still i64: the runtime's column access, run views,
registers and outputs widen every value. These are not same-program numbers as
the reference: the build uses blocked runs, and the Nsight trace shows
`sorted_check`, the blocked-run sorted-input skip. Order-preserving IDs make
`o_orderkey` arrive sorted, so that check skips the sort. The PTX
profile is unchanged from the i64 version: `cq_gpu_stream_1_r19` uses 104 B of
stack, and `cq_gpu_stream_0_r21` has a 16 B frame with 24 B of spills.

**Plan R** (`queries::plan_r`, runner argument `R`) is the reference
traversal in written order (`order(written, coarse)`) with ASIA as the literal
`2`. Physical `[build_host_reads, retain_streams(r4)]`; Structures
`[sorted_indexes, sorted_runs, prefix_runs, retype(dedup => DeviceSet<_, For>)]`;
staging `expand_lookups(256, depth 5)`. The ASIA Region rows are a build-phase
table. `retain_streams(r4)` stores Region ⋈ Nation ⋈ Customer, so the second
kernel's grid spans every ASIA customer; without it the stream starts from the
single ASIA region row and the printer launches one block (SF10 query kernel
143 ms). Exact at SF1/SF10; median total 4.52 ms (SF1) and 24.78 ms (SF10),
query kernel 2.65 / 13.26 ms, against `h05-reference` 1.89 / 9.70 ms. The
remaining gap is the printer's staged kernel and i64 builds.

## Implementation

- `tiles`: cross-crate `#[device]` helpers for 256-thread exclusive scan, stable
  shared-memory compaction, upper-bound owner search, shared bitonic sorting of
  up to 1024 u32 keys, and stable merge-path partition. Oracle kernels invoke
  the library across a crate boundary, including repeated offsets, empty inputs,
  partial tiles, random masks, MAX keys and every merge diagonal.
- `blocked-run`: a device sortedness test selects adjacent-difference
  construction or dense counting sort with bitmap-word prefix scans, atomic
  provenance scatter and per-bin heapsort for stable duplicates.
  Payloads are gathered in stable input-row order. Runs expose CSR offsets, unique
  keys, 2048-row fences, a dense-domain u64 bitmap and u32 popcount prefixes.
  `point` returns a half-open `(begin,end)` range.
- Plan A: dynamically scheduled 1024-row lineitem tiles (256 threads, four
  striped items each), order rank probe after date filtering, customer rank
  probe, nation direct-array/region bitmap check, supplier rank probe and
  nation equality. One atomic reservation per nonempty tile, shared packing,
  then coalesced columnar stores.
- Plan B: customer tiles restricted to ASIA; CSR expansion into date-filtered
  orders by customer, then into lineitems by order. Each stage retains begins,
  exclusive offsets and parent nation in shared memory. Output-parallel owner
  search resolves every virtual row. Nested in-tile loops consume all `M/T`
  chunks without any global frontier or cross-block stealing. Counters record
  the maximum expansion at each stage. No frontier spill is necessary for
  correctness, regardless of the observed expansion ratio.
- Both plans allocate output capacity equal to lineitem cardinality, a strict
  bound under the TPC-H PK/FK schema. Thus overflow/relaunch is unnecessary for
  this query, rather than silently dropping output. Dedup packs all three fields
  losslessly into u64 (26-bit order, 17-bit supplier, 5-bit nation); actual
  manifest-column domains are checked before upload. Zero marks an empty slot,
  so stored keys are encoding+1. AtomicCAS linear probing compares the complete
  encoded key; successful insertions count distinct triples. Capacity is at
  least four times the emitted row count, rounded to a power of two.

## Contracts and measurement caveats

The toolchain is pinned to nightly-2026-08-28 and cuda-oxide commit
`5f7c34fddadcf250f2e45d0cffe83a3d27f2bea3`. `rccl-memory` is a workspace git
dependency pinned to `https://github.com/StarGazerM/rccl.git` revision
`745f53cc8d447ec58033d97f4d13f2d7e2f7463a`; it uses the same cuda-oxide
revision and does not introduce device PTX.
Device artifacts are workspace-local under `target/oxide-ir`.
The release profile disables LTO.

The first tile build failed MIR verification when passing the pointer returned
by `DynamicSharedArray::<u32>::get()` into a helper: the call carried address
space 3 while the helper signature carried a generic pointer. Moving helpers
into a `#[path]` module did not fix it. The supported RCCL pattern,
`DynamicSharedArray::<u8>::get_raw() as *mut u32`, fixes this type-lowering
boundary without modifying the compiler. The module fallback is not retained.
Rust also warns that `#[inline(always)]` is ignored on exported `#[device]`
functions; the device backend's documented inline behavior and resource audit
remain the relevant evidence, rather than suppressing that host warning.
The resident integration also exposed a host-linker collision when two device
libraries exported a kernel named `decode_column`: both emitted
`cuda_oxide_codegen_v1_cuda_oxide_kernel_246e25db_decode_column`.
The FastLanes consumer kernel is therefore named `fastlanes_decode_column`;
kernel names must be distinct across these linked libraries. No toolchain
source was changed.

Cooperative tile helpers require exactly 256 converged threads. Scan needs eight
shared u32 words and uses wrapping arithmetic; callers ensure expansion totals
fit u32. Compaction's public helper accepts four consecutive indices per thread;
the query's striped TileStore uses the same scan with its own row-ID packing.
Bitonic sort is key-only and not stable. Merge-path is stable A-before-B.
Pointer validity and shared-memory ownership are explicit caller obligations.
Rank and row positions use u32; unsupported large domains are rejected rather
than truncated. The query's direct nation array rejects negative nation keys;
region keys must fit its 64-bit bitmap. These are workload representation
contracts, not general-purpose SQL limitations.

`scan_gbps` is the logical 16-byte-per-lineitem input divided by Plan A kernel
time, compared with the 960 GB/s model; it is not a hardware DRAM counter and
includes probe/compaction work. Plan B reports null for that metric because its
access pattern is not a fact-table scan. Device memory uses CUDA Driver
`cuMemGetInfo` (the counterpart of runtime `cudaMemGetInfo`), sampled while run
construction scratch is live, after build, and after dedup against a pre-upload
baseline. It includes persistent inputs/outputs, run data, and sampled build
scratch. This is sampled device usage, not a hardware allocator high-water mark.
Tiny nation/region representations are prepared once outside timed rebuilds.

Scratch, run columns and output buffers use one `rccl-memory`
`PoolMemoryResource<CudaMemoryResource>` wrapped in `StatisticsMemoryResource`.
The fixed-capacity pool is primed from input cardinalities before warmup;
stream-ordered suballocation and return never invoke synchronous device free.
Only one-time input uploads remain on `cuda_core::DeviceBuffer`. Per-iteration
allocation counts are logical suballocations, not CUDA allocation calls, and
peak bytes count live requested bytes rather than reserved pool capacity.
Partially initialized histogram, cursor and permutation scratch retains
`DeviceUninitBuffer` ownership; kernels receive raw pointers to only the regions
they initialize and consume, rather than claiming the entire capacity is valid.

Key-domain bounds are prepared once from manifest-validated input columns.
Filtered cardinalities, scan totals and run metadata remain on device; run
buffers have bounded capacity rather than requiring a downloaded unique count.
`read_stats` combines all run records into one end-of-build DtoH transfer.
The query output count is read once to size the dedup table, whose kernel reads
the device counter. Dedup status and exact-answer downloads happen after the
final timing event. No extrema or per-stage total readback remains in a build.

The histogram sets a presence bit only on the atomic count's 0→1 transition.
A coalesced warp per 64-key word computes local bin offsets, its row total,
its bitmap popcount and its maximum multiplicity. Only the two word-sized
arrays (row totals and popcounts) are globally scanned. A bin's final row
offset is its local offset plus the word's row prefix; its CSR rank is the
word's popcount prefix plus the lower bits' popcount. Row totals are necessary
in addition to popcounts because duplicate keys contribute multiple rows.
This removes the full-domain multi-level scan and the old serial directory
kernel. Oracles cover partial words and recursive word-prefix boundaries
through a 4,194,305-position sparse domain, with stable duplicate payloads.

A one-pass adjacent-key check records sortedness on device (warp ballots reduce
flag updates). Nondecreasing keys use adjacent run starts/ends to build the
bitmap, CSR offsets, unique keys and maximum multiplicity; copying the already
ordered columns yields fences directly. Histogram initialization, counting,
scatter and provenance heapsort return without work on this path. Their
device-gated launches remain, avoiding an extra sortedness readback.
SF10 takes this path for **customer and supplier in both plans, plus lineitem
in Plan B**, in warmup and all measured iterations. The stable date-filter
compaction now also preserves the input order for `orders_by_order`;
`orders_by_customer` remains unsorted. The optimization checks device data
rather than assuming dbgen ordering. See [the H05 measurements](h05/README.md)
for the newer stable-compaction and merge-mode results.

## RCCL compiler bug and approved workaround

The initial RCCL-based blocked-run oracle built and passed its two empty-input
cases. Its first nonempty case reached RCCL's program loader and failed with
CUDA error 218 (`a PTX JIT compilation failed`). Assembling the concatenated
emitted PTX identified the actual cause:

```text
ptxas target/diagnostic-merged.ptx, line 6987; error   : Duplicate definition of variable '__shared_mem_0'
ptxas fatal   : Ptx assembly aborted due to errors
```

The initial `gpu_blocked_run.ptx` contained RCCL radix kernels monomorphized into
our library: their histogram had `.visible .shared ... __shared_mem_0[8192]`.
`rccl_cub.ptx` also declared `__shared_mem_0[8192]` for its nongeneric u32 scan.
RCCL's generic program loader merges these modules, and duplicate visible
definitions are invalid PTX. All handwritten kernels in this workspace already
use only dynamic shared memory; changing our shared allocations therefore
cannot remove this collision in dependency-generated kernels.

The compiler source confirms the prerequisite fix:
`mir-lower/src/convert/ops/memory/shared.rs` creates `__shared_mem_{counter}`
with a module-local counter, while `cuda-host/src/embedded.rs` concatenates
PTX bundles without renaming these visible globals. The compiler must namespace
generated shared globals by module, or the merger must safely namespace them.
Neither RCCL nor the cuda-oxide checkout has been modified.

The experiment was explicitly redirected to use handwritten dense counting
sort, dynamic hierarchical scans and an exact u64 CAS set. RCCL is removed
from the device dependency graph rather than silently patched, and the original
diagnostic is retained in `compare/rccl-ptx-collision.txt`. Consequently the
results measure the fused execution design, **not RCCL sort/scan performance**.

## Allocation and build optimization measurements (2026-10-18)

Measured on the same RTX 6000 Ada / sm_89 machine with the pinned toolchain.
`rccl-memory` built unchanged against the pin; no RCCL or cuda-oxide source
was modified. All four stages were applied and measured in order. At every
stage, SF1 and SF10, both plans, the warmup and all five measured iterations
reported `exact=true` (7,242 and 72,983 distinct triples respectively).
The final implementation is `bf34e9f`; the staged commits and complete samples
are retained in `compare/gpu_h05.results.json` under `optimization_history`.

### Before / after

Build time is the median unprofiled CUDA-event interval, including host-caused
idle time. Kernel sum is the median **build-only** sum from a separate nsys
capture, including filtering but excluding query and dedup kernels. It is not
the existing `kernel_ms` field, which measures the fused query alone.
All arrows compare the fresh original-build baseline to the final code.

| SF | Plan | Build ms | Build kernel sum ms | cuMemAlloc / cuMemFree per iteration | DtoH timed / whole iteration | Scan-pipeline DRAM MB |
|---:|:---:|---:|---:|---:|---:|---:|
| 1 | A | 3.044064 → 1.426496 | 0.242370 → 0.287329 | 60/60 → 0/0 | 12/17 → 2/7 | 50.440 → 26.416 |
| 1 | B | 4.937056 → 2.199872 | 1.088487 → 0.674179 | 77/77 → 0/0 | 15/20 → 2/7 | 51.687 → 3.084 |
| 10 | A | 13.001888 → 4.884064 | 3.683762 → 3.567031 | 63/63 → 0/0 | 12/17 → 2/7 | 864.474 → 460.751 |
| 10 | B | 22.511841 → 9.477088 | 12.061921 → 7.181522 | 80/80 → 0/0 | 15/20 → 2/7 | 876.934 → 29.904 |

Allocation/free counts include iteration-local verification and teardown, but
exclude process startup/shutdown. The final **timed** counts are also 0/0 for
every measured run. The final whole-process SF10 `both` trace has 15 allocations
and 15 frees: fourteen input uploads plus one pre-warmup pool reservation, not
timed rebuilding. The original process had eighteen persistent allocations;
its fresh SF10 A iteration count is 63, rather than the earlier reported 66.
The scope here explicitly excludes startup allocations; whole-process totals
must not be presented as timed per-run counts.

Build DtoH calls decrease from **10 (A) / 13 (B) to one**, independent of scale.
One additional timed readback obtains the query output count. The five other
DtoH transfers validate dedup metadata, the hash table and three materialized
columns **after** the final event: seven whole-iteration readbacks, not seven
build synchronizations. The final SF10 `both` trace has 84 DtoH calls for twelve
iterations. These counts come from nsys CUDA API records, not source estimates.

Scan traffic is ncu `dram__bytes.sum`, summed over the first complete build
iteration under kernel replay, in decimal MB. Before includes `scan_blocks`
and `add_prefix`; after additionally includes the necessary `word_offsets`
local-bin/directory pass, rather than hiding that traffic. Histogram and sorted
adjacency passes are not part of this scan metric. Exact byte counts are in
the JSON. On this SF10 dataset, sparse order keys span approximately **60 M
positions** despite having 15 M order rows; the dense u32 array is approximately
240 MB, not a 15 M-position array.

### Measurement after each change

| SF | Plan | Original build ms | 1: pool | 2: device sizes | 3: word scan | 4: sorted path |
|---:|:---:|---:|---:|---:|---:|---:|
| 1 | A | 3.044064 | 1.409280 | 1.291168 | 1.193888 | 1.426496 |
| 1 | B | 4.937056 | 2.595488 | 2.342464 | 2.247040 | 2.199872 |
| 10 | A | 13.001888 | 5.557216 | 5.486016 | 4.233536 | 4.884064 |
| 10 | B | 22.511841 | 14.608064 | 13.343232 | 12.388096 | 9.477088 |

The sortedness check is not free: it regresses Plan A relative to step 3
because filtered orders are not ordered. It improves SF10 B by bypassing the
lineitem histogram/scatter/provenance sort. This is measured behavior, not an
assumption that all dbgen-derived columns remain sorted after compaction.

Before enabling the sorted path, step 3 alone reduced SF10 scan-pipeline
traffic from 864,474,496 to 466,995,072 bytes (A), and from 876,934,144 to
473,732,224 bytes (B). Of those after totals, the global word scans moved only
15,568,128 / 15,977,472 bytes; the remainder is the fused local-offset pass.
Thus the reduction does not depend on sorted input.

| SF | Plan | Logical allocations per run | Peak live requested bytes |
|---:|:---:|---:|---:|
| 1 | A | 57 | 270,048,720 |
| 1 | B | 73 | 439,924,608 |
| 10 | A | 59 | 2,699,859,640 |
| 10 | B | 75 | 4,397,928,900 |

These `StatisticsMemoryResource` counters include iteration-local exactness
copies; peaks include persistent outputs but exclude uploaded inputs. The
fixed pools reserve 926,003,712 bytes at SF1 and 8,721,138,688 bytes at SF10.
Logical suballocation is still performed; **CUDA allocation/free is absent
from the timed region**. No allocator-growth fallback is allowed.

### Profiling and correctness evidence

Run unprofiled timings separately from profiling:

```sh
./target/release/gpu-h05 /data/tpch-gpu/sf10 both /tmp/h05-timings.json
nsys profile -o /tmp/h05 --force-overwrite true \
  ./target/release/gpu-h05 /data/tpch-gpu/sf10 both /tmp/h05-nsys.json
nsys stats --report cuda_api_sum,cuda_gpu_kern_sum /tmp/h05.nsys-rep
nsys export --type sqlite --force-overwrite true -o /tmp/h05.sqlite /tmp/h05.nsys-rep
ncu --kernel-name 'regex:word_offsets|scan_blocks|add_prefix' --launch-count 48 \
  --metrics dram__bytes.sum --csv --log-file /tmp/h05-ncu.csv \
  ./target/release/gpu-h05 /data/tpch-gpu/sf10 A /tmp/h05-ncu.json
```

Repeat for SF1 and Plan B. For the original scan, omit `word_offsets` from the
kernel regex. Use CPU launch timestamps/correlation IDs to identify iterations
at `filter_orders`, the build end at `plan_a`/`plan_b`, and the final timing
event after `dedup_insert`; do not confuse pool free-range events with timing
events. The ncu first-iteration scan launch counts are 22/30 (SF1 A/B) and
28/36 (SF10 A/B) before, and 21/28 and 25/32 after. Extra sampled iterations
are excluded. Captures are `/tmp/gpu-h05-before-*`, `/tmp/gpu-h05-after-*` and
the intermediate `pool`, `device-sizes`, `word-scan` variants on this host.
Persisted JSON includes the capture paths and metric scopes.

Executed final checks: pinned release build of all h05 binaries; blocked-run
oracle (40 data cases plus invalid-input checks); dedup oracle (12 cases);
all SF1/SF10 A/B iterations, including under nsys/ncu. New-kernel coverage
includes device-count tails and zero active rows, invalid counts/domains,
word and recursive prefix boundaries, sorted duplicates and provenance,
and single inversions at warp/block/large-domain boundaries. No
compute-sanitizer result is claimed.

## Historical original measurements (2026-10-16)

GPU: NVIDIA RTX 6000 Ada Generation, sm_89, driver 580.178.04, 49140 MiB
reported device memory. Clocks were not locked and exclusive-GPU ownership was
not established; these are local event measurements, not a controlled hardware
bandwidth study. Upload and final downloads are excluded as described above.

| SF | Plan | Build median ms | Kernel median / min ms | Dedup median ms | Total median / min ms | Distinct answers | Sampled peak MiB |
|---:|:---:|---:|---:|---:|---:|---:|---:|
| 1 | A | 2.937024 | 0.088064 / 0.088032 | 0.315744 | 3.338592 / 3.326016 | 7,242 | 360 |
| 1 | B | 4.932288 | 0.034816 / 0.033920 | 0.320704 | 5.288992 / 5.260768 | 7,242 | 490 |
| 10 | A | 12.337152 | 0.905216 / 0.903424 | 1.375360 | 14.613600 / 14.597376 | 72,983 | 3,526 |
| 10 | B | 22.318176 | 0.302240 / 0.300032 | 0.932320 | 23.552608 / 23.508928 | 72,983 | 4,820 |

Both plans matched the complete DuckDB answer set at SF1 **and SF10**, in the
warmup and all five measured runs. Synthetic input produced 579 exact triples
for both plans. The table and discussion below describe the original allocator
and full-domain scan; current samples are in `compare/gpu_h05.results.json`.

The SF10 Plan A stream is below the 10 ms design gate. Plan B is 0.334 times
Plan A's stream time and 1.612 times its build+stream+dedup time. Plan B has no
global frontier: its descriptor/working storage stays at 33,808 bytes per block.
The sampled global-memory increase is for input/output buffers, indexes and
counting-sort scratch, not a fanout-sized intermediate.

Maximum virtual expansion totals were:

| Input | Customer→orders M | Orders→lineitem M | Maximum M/1024 |
|---|---:|---:|---:|
| synthetic | 1,192 | 6,146 | 6.001953 |
| SF1 | 413 | 1,660 | 1.621094 |
| SF10 | 407 | 1,643 | 1.604492 |

All were consumed by in-tile loops; no frontier spill was needed. Cross-block
stealing remains deliberately outside this experiment.

SF10 Plan A's nominal 16-byte-per-lineitem rate is **1,060.274 GB/s** against the
960 GB/s model. This is not measured DRAM bandwidth: supplier loads are
predicate-gated and caches can serve probes. Counting only the obligatorily
scanned 8-byte order-key column gives **530.137 GB/s**. Do not interpret the
nominal rate above the model as exceeding the GPU's physical memory bandwidth.

`ptxas -v -arch=sm_89` reports:

| Kernel | Registers | Stack bytes | Spill stores / loads | Dynamic shared bytes |
|---|---:|---:|---:|---:|
| Plan A | 32 | 0 | 0 / 0 | 9,216 |
| Plan B | 34 | 0 | 0 / 0 | 33,808 |
| Dedup insertion | 18 | 0 | 0 / 0 | 0 |
| Run scan | 18 | 0 | 0 / 0 | 32 |

Static shared memory is zero for these kernels. Full assembly output is in
`compare/ptxas.txt`, with all parsed entries in `compare/ptxas.resources.json`.
The initial fixed-four-item loops produced 40-byte local stack frames despite
zero register spills; explicit `#[unroll]` on those loops removed the frames.
No Segment split is needed for the measured 32/34-register streams.

Executed correctness checks: vector-add smoke (1,025 elements); tile oracle
(82 scan, 92 compact, 246 sort, 65 owner, 114 merge-path cases); blocked-run
oracle (24 data cases plus invalid-input checks, including 131,073-row skew and
recursive scan boundaries); dedup oracle (12 cases including all-zero and
maximum encodings); complete synthetic/SF1/SF10 results for both plans.
No compute-sanitizer result is claimed.

## SRDatalog variable/clause-order sweep (2026-10-18)

SRDatalog is a variable-order/WCOJ-style engine, not a fixed binary-join
baseline. On RTX 6000 Ada, driver 580.178.04, engine revision
`fab849f672d43af2a6a1e8a1727ec065201e3720`, the **best exact measured**
configuration is `cnrods_clauses341205_filters`: **5.169698 ms at SF1,
176.812205 ms at SF10**, versus rerun `fact` medians of 38.139930 and
574.001685 ms. This is a bounded sweep, not a claim of global optimality.

Each variant compiles once (SF10 reuses the SF1 native build cache), then
uses a fresh initialized/loaded/prepared DB for one warmup and five timed
runs. Load and prepare are excluded; `run(0)` plus synchronization is timed.
Dimension-filter derivation and any materialized semijoins are **inside**
the timed query, not precomputed on the CPU. Whole-device VRAM is sampled
at 1 Hz: these peaks are lower bounds, not allocator high-water marks.
Clocks were not locked and exclusive device ownership was not established.

| SF1 requested order / variant | Median ms | Exactness |
|---|---:|---|
| `o,d,c,n,r,s` | rejected | 125 rows, expected 7,242 |
| `o,c,d,n,r,s` | 19.387561 | count + tuple diff |
| `o,d,s,c,n,r` | rejected | 125 rows |
| `d,o,c,n,r,s` | rejected | 0 rows |
| `c,n,r,o,d,s` | 11.226063 | count + tuple diff |
| `n,r,c,o,d,s` | 697.755019 | count + tuple diff |
| `s,n,r,o,d,c` | rejected | 125 rows |
| engine default | 37.974203 | count + tuple diff |
| `fact`: `o,s,c,d,n,r` | 38.139930 | count + tuple diff |
| `c,n,r,o,d,s` + `.with_semi_join()` | 11.279425 | count + tuple diff |
| `c,n,r,o,d,s` + clauses `3,4,5,6,1,2,0,7` | 10.552553 | count + tuple diff |
| `c,n,r,o,d,s` + dimension filters | 5.490582 | count + tuple diff |
| `c,n,r,o,d,s` + dimension filters + semijoin rewrite | 5.221426 | count + tuple diff |
| **`c,n,r,o,d,s` + dimension filters + clauses `3,4,1,2,0,5`** | **5.169698** | **count + tuple diff** |

All accepted SF1 variants return 7,242 rows in all six runs and match the
complete reference tuple set in the warmup export. **Not all attempted
orders are correct:** four requested orders compile but fail the first
count check, so their timed medians are null and they are excluded from
ranking. In particular, the planner accepts filtered-only `d` first but
the resulting query returns zero rows. No engine patch or count-check
suppression was used. The “all variants exact” criterion therefore remains
unmet for these rejected engine configurations.

The planner's own default chooses `o,s,c,n,r,d,_c0` (the last variable is
introduced for the ASIA constant); its heuristic is structural, not
cardinality/cost-based. Chosen HIR plans and emitted MIR are retained.
The plain-rule semijoin opt-in leaves its plan unchanged; unary dimension
filters create bound-key membership probes, while adding `.with_semi_join()`
also materializes filtered Customer/Supplier relations. Further exploration
of families exceeding 3× the best was stopped; the historically slow `dim`
order was not rerun. `fact` was retained as the required SF10 control.

The three fastest exact SF1 variants and `fact` were run at SF10:

| SF10 variant | Median ms | Min ms | Sampled peak MiB | Exact count, all six runs |
|---|---:|---:|---:|---:|
| **dimension filters + clauses `3,4,1,2,0,5` (best)** | **176.812205** | **176.762880** | **3,230** | **72,983** |
| dimension filters + semijoin rewrite | 192.410878 | 192.069845 | 3,230 | 72,983 |
| dimension filters, derived clause order | 198.356330 | 198.161938 | 3,230 | 72,983 |
| `fact` | 574.001685 | 573.087575 | 3,230 | 72,983 |

SF1 accepted variants all sampled 2,206 MiB. SF10 correctness here is a
count check, **not** a full tuple comparison. The winning DSL is below
(using the runner's input declarations and unary `AsiaRegion`/`AsiaNation`
relations). Removing dimension joins from H05 removes `r` from its actual
variable order; the CLI's six-variable order is explicitly projected to
`c,n,o,d,s`. No materialized semijoin opt-in is enabled for the winner.

```python
Program(rules=[
    (AsiaRegion(r) <= Region(r, 2)).named('AsiaRegion'),
    (AsiaNation(n) <= Nation(n, r) & AsiaRegion(r)).named('AsiaNation'),
    (Answer(o, s, n) <= Lineitem(o, s) & Orders(o, c, d)
     & Filter(('d',), 'return d >= 8766 && d < 9131;')
     & Customer(c, n) & AsiaNation(n) & Supplier(s, n))
    .named('H05')
    .with_plan(var_order=['c', 'n', 'o', 'd', 's'],
               clause_order=[3, 4, 1, 2, 0, 5]),
])
```

Reproduce the winner from this fork's root:

```sh
uv run --project "$SRDATALOG" python -u \
  experiments/gpu/compare/srdatalog_h05.py --sf 1 10 \
  --var-order c,n,r,o,d,s --dimension-filters --clause-order 3,4,1,2,0,5 \
  --output experiments/gpu/compare/srdatalog_h05.winner-rerun.json
```

`--var-order` is repeatable and can be combined with named `--order`
variants. Clause indices address the body after constant rewriting:
ordinary H05 uses eight clauses (`0` Lineitem, `1` Orders, `2` date filter,
`3` Customer, `4` Nation, `5` Region, `6` ASIA filter, `7` Supplier);
dimension-filter H05 uses six (`4` AsiaNation, `5` Supplier).
`compare/srdatalog_h05.sweep.json` retains all 18 measured/attempted entries,
including errors, requested/chosen plans, five samples for each accepted
variant, counts, and paths to logs/MIR. `srdatalog_h05.results.json` labels
the winners `best` and retains the named historical baselines.

## Directed triangle lookup-expand experiment

`triangles` is a separate device-library package. Its benchmark and oracle are
in the sibling `triangles-runner` package, avoiding cuda-oxide's same-package
library/binary module-name ambiguity. It depends on `tiles` and `blocked-run`;
neither library is forked or copied.

The query is exactly `Triangle(a,b,c) <= E(a,b) & E(b,c) & E(a,c)`.
Input ingestion sorts and deduplicates directed edges once on the host; it
does not add reverse edges, remove self-loops, or impose `a < b < c`. This
canonicalization and upload are outside CUDA-event timing, and ingestion time
is recorded separately. The GPU builds the source-keyed blocked run with
sorted neighbors. The rank-domain budget includes histogram scratch and is
64 MiB; domains exceeding that budget or the density bound use 2048-row fences
and equal-range binary search instead. Sparse signed-i64 domains, including
both extrema and repeated fences, are covered by the oracle.

Each 1024-edge tile builds exclusive expansion offsets with four cooperative
256-thread scans. Intersection expands the smaller of `N(a)` and `N(b)` and
binary-searches the other sorted range. Parent bindings retain the original
`(a,b)`, including when the expansion side is swapped. T-sized virtual chunks
are consumed immediately; counting uses a block reduction and one u64 atomic
per consumed chunk. Optional materialization emits exact triples and reports
insufficient output capacity instead of silently dropping rows.

The 14,340-byte descriptor contains 1024 edge-row bindings, 1024 range begins,
1025 exclusive offsets, and 1024 packed u16 parent slots/side flags. Pool-on
uses `K=8`, 64 slots, and a persistent 568-block grid. Owners publish when
`M > K*T`, consume head chunks, and free the slot after all chunks have been
claimed. Blocks without root tiles steal tail chunks. A slot's acquire/release
lock protects its cursors and descriptor lifetime: a thief copies the complete
descriptor to its own shared memory before releasing the lock. Consequently
slot reuse cannot invalidate an in-flight chunk. A full pool falls back to the
owner's in-tile loop. A global root cursor and remaining-owner counter terminate
the grid; stolen chunks finish before their blocks test termination. `K=0`
disables publication. Both modes use the same fixed RCCL allocation reserve,
primed before warmup, with no CUDA allocation/free in timed regions.

The descriptor uses u32 offsets. Expansion overflow is detected rather than
wrapped into an answer; each range is currently limited to `u32::MAX/256`
items so each block scan is exact, and each descriptor total must fit u32.
Triangle totals and pool statistics use u64. These representation limits do
not constrain either measured input or the degree-1,000,000 oracle.

The BFS baseline first materializes **all** `E(a,b) & E(b,c)` wedges as
three-i64 tuples, then launches the membership filter. It does not use the
smaller-range optimization. Its required allocation is `24 * wedge_count`
bytes; results also report the `12 * wedge_count` lower demand for a possible
u32-column implementation. An allocation exceeding available VRAM is recorded
as `oom_preflight`, not as an executed BFS timing.

Reproduce from `experiments/gpu`:

```sh
# Use the CUDA_HOME/PATH setup in Reproduce above.
cargo oxide build --arch sm_89 -- --release -p gpu-triangles-runner --bins
./target/release/triangles-oracle
# Supply the independently obtained DuckDB count:
./target/release/gpu-triangles EDGE_FILE DUCKDB_COUNT RESULT_JSON
"$CUDA_HOME/bin/ptxas" \
  -v -arch=sm_89 target/oxide-ir/gpu_triangles.ptx -o /dev/null
```

The oracle compares pool capacities 0, 1, and 64, thresholds 0, 1, and 8,
with three repetitions of each combination. Cases include empty graphs,
duplicate input edges, self-loops, random partial tiles, a clique, sparse
repeated fences, output-capacity exhaustion, and a geometric power-law graph
whose first node has exactly 1,000,000 outgoing neighbors. It checks counts,
materialized triple membership/uniqueness, termination, released slots,
actual stealing, and pool-full fallback.

### Triangle measurements (2026-10-18)

Both inputs were exact against DuckDB on every warmup and measured GPU run:
**DBLP: 2,224,385 triangles from 1,049,866 edges**;
**LiveJournal: 946,400,853 triangles from 68,993,773 edges**.
Neither input contained duplicate edges. Both source domains fit the rank
budget. Results and the comparison table are retained in
[`compare/gpu_triangles.results.json`](compare/gpu_triangles.results.json);
per-run samples, reference SQL, console logs, and the independent comparison
runner remain alongside it.

These are medians of five runs after one warmup on an RTX 6000 Ada (sm_89).
GPU columns separate the fused query from run-build plus query. SRDatalog's
run includes `KnowsLoad` and its index builds; DuckDB's 24-thread self-join runs
on an already loaded distinct-edge table. All exclude file loading/H2D.
Our host ingestion additionally sorts the neighbor lists before upload:
67.013 ms for DBLP and 4,859.487 ms for LiveJournal, excluded from these GPU
event intervals. Thus this table is **not an end-to-end ingestion comparison**.

| Dataset | Method | Query/kernel ms | Build + query / engine run ms | Device memory / BFS demand | Stolen chunks |
|---|---|---:|---:|---|---:|
| DBLP | GPU pool off | 0.715 | 1.877 | 128 MiB sampled delta | 0 |
| DBLP | GPU pool on (`K=8`, 64 slots) | 0.711 | 1.866 | 128 MiB sampled delta | 153–171 |
| DBLP | BFS materialize, then filter | 1.293 | — | 161.699 MiB wedge allocation; 398 MiB sampled delta | — |
| DBLP | SRDatalog best: `b,a,c` | — | 5.857 | 2,206 MiB whole-device sampled lower bound | — |
| DBLP | DuckDB, 24 threads | 77 | — | CPU | — |
| LiveJournal | GPU pool off | 88.255 | 95.885 | 5,412 MiB sampled delta | 0 |
| LiveJournal | GPU pool on (`K=8`, 64 slots) | 89.431 | 101.773 | 5,412 MiB sampled delta | 1,740–3,607 |
| LiveJournal | BFS materialize, then filter | OOM preflight | — | **132.225 GiB** i64 wedges; **66.112 GiB** even with u32 triples | — |
| LiveJournal | SRDatalog best: `b,a,c` | — | 1,760.952 | 9,374 MiB whole-device sampled lower bound | — |
| LiveJournal | DuckDB, 24 threads | 35,368 | — | CPU | — |

The GPU memory delta is measured with `cuMemGetInfo` relative to the
post-module/pre-upload baseline. It includes inputs and the entire fixed RCCL
pool reserve, not just live logical suballocations. The same reserve is used
with publication disabled and enabled. DBLP's BFS peak also includes the
retained lazy-query pool, explicitly recorded in its JSON. SRDatalog's 1 Hz
whole-device sampling is a different metric and must not be treated as a
comparable allocator high-water mark.

LiveJournal has 5,915,633,945 wedges: 141,975,214,680 bytes for the implemented
three-i64 BFS layout, or 70,987,607,340 bytes for three u32 columns before any
index/scratch storage. Available device memory at preflight was
43,942,019,072 bytes; the baseline was rejected without allocating or launching.
DBLP's 7,064,738 wedges were actually materialized and filtered, with exact
counts on all six runs. The host-only wedge-demand diagnostic is separate
from query execution (112.362 ms DBLP, 48,809.696 ms LiveJournal).

Maximum `M/T` was **18.782** on DBLP and **388.771** on LiveJournal. On
LiveJournal the 64-slot pool published 7,158–7,316 descriptors per measured run
and fell back depth-first 56,863–57,021 times. Pool stealing is exercised and
correct, but **does not improve this LiveJournal measurement**: its kernel is
about 1.3% slower than the in-tile-only loop. Build medians also varied
(7.600 ms pool-off versus 11.887 ms pool-on), so total-time differences cannot
all be attributed to descriptor stealing.

Final GPU timings were taken after the reference and H05 GPU runs finished;
there was no intentional concurrent GPU benchmark. The machine was **not
quiet**: recorded 1/5/15-minute CPU load averages were `21.94/21.39/12.85`
(DBLP) and `22.31/21.20/13.29` (LiveJournal). Concurrent CPU gates and host
scheduling can affect CUDA-event intervals that span host submissions.

SRDatalog's actual `examples/lsqb_triangle_count.py` is imported by
`compare/srdatalog_triangles.py`; only the requested variable plan is replaced.
Its default selected `a,b,c`. All six dataset/order combinations completed and
matched DuckDB on the warmup and every measured run:

| SRDatalog order | DBLP ms | LiveJournal ms |
|---|---:|---:|
| engine default (`a,b,c`) | 6.618 | 2,576.524 |
| explicit `a,b,c` | 6.547 | 2,575.829 |
| explicit `b,a,c` | **5.857** | **1,760.952** |

```sh
uv run --project "$SRDATALOG" python \
  compare/srdatalog_triangles.py --dataset dblp livejournal \
  --order default abc bac
./target/release/gpu-triangles "${DBLP_DATASET:-data/graph/com-dblp.ungraph.txt}" \
  2224385 compare/triangles_dblp.gpu.json
./target/release/gpu-triangles "${LIVEJOURNAL_DATASET:-data/graph/soc-LiveJournal/edge.facts}" \
  946400853 compare/triangles_livejournal.gpu.json
```

`compare/triangles.oracle.txt` records the passing oracle, including 32,394
stolen chunks across the degree-1,000,000 graph's protocol variants and
3,999,859 exact triangles. `compare/triangles.ptxas.txt` records the actual
resource audit: the persistent kernel uses **40 registers**, **14,468 bytes
dynamic shared memory**, a **104-byte stack frame**, and **zero spill stores
or loads**. The BFS filter uses 18 registers, 32 bytes dynamic shared memory,
and zero stack/spills. No CUDA C, NVRTC, or upstream checkout changes were used.

### Compiled triangle with checked host sets (2026-10-19)

`triangle-compiled` records `cq_rewrites::host_sets(E)` after the loader sorts
and deduplicates directed edges. The emitted triangle has no canon/dedup:
the full-heading host keys prove the returned `(a,b,c)` heading unique.
It still certifies the actual resident input on every invocation. The expanded
triangle and both certificate kernels were reviewed before GPU execution.
`printer-oracle --host-sets` passed sorted/unsorted inputs, duplicate rejection
with the named error, exact hash collisions, projected duplicates, resident
inputs, and nullary runtime tables.

Measured on RTX 6000 Ada (49,140 MiB), sm_89, driver 580.178.04:

| Dataset | Answer rows | Cold query ms | Warm median query ms | Certificate median ms | Output column bytes |
|---|---:|---:|---:|---:|---:|
| DBLP | 2,224,385 | 3.553 | 0.881 | 0.041 | 53,385,240 |
| LiveJournal | 946,400,853 | 267.781 | 160.690 | 1.653 | 22,713,620,472 |

The cold query needed two direct-output attempts; each of five warm runs needed
one. Query wall time includes the input certificate, index builds, allocation,
and kernels; input parsing/sorting/upload and result download are excluded.
Separate certificate-only samples run afterward on the resident input (one
discarded sample, then median of five), including count/status readbacks.
DBLP compares every output tuple with the independent CPU reference using
`--check`. LiveJournal matches the requested count; it is not claimed to have
undergone a full-tuple comparison. Its output uses 21.154 GiB of column storage,
not counting the input, indexes, or allocator reserve.

The initial full-download LiveJournal benchmark returned a VRAM error. The
runner now uses checked metadata counts except for the first `--check` sample,
and drops each answer before the next query. Direct retry history retains the
previous exact count rather than doubling a materialization; oversized estimates
use a zero-capacity counting attempt before exact allocation. All six final
LiveJournal runs completed without dedup or allocation OOM. Raw samples, the
initial failed run, and validation scope are retained in
[`compare/host_sets.results.json`](compare/host_sets.results.json) and its logs.

## Structures GPU terminal printer

`printer` is the external proc-macro crate `cq-gpu`; `printer-runtime` owns
typed resident hosts, pooled blocked-run builds, output allocation and device
canonicalization. `device-columns` (`gpu-device-columns`, re-exported through
`cq_gpu_runtime`) owns the pooled column buffers and device codecs.
`h05-compiled` authors the retail clauses twice, with fact-first and
customer-first written orders. It does not call the hand-written H05 kernels.

```rust,ignore
pipeline {
    plan = ::cq_planners::order(le);
    physical = [::cq_rewrites::build_host_reads];
    structures = [
        ::cq_rewrites::sorted_runs,
        ::cq_rewrites::hashed_runs,
        ::cq_gpu::prefix_runs,
        ::cq_rewrites::retain_lookups,
        ::cq_rewrites::retype(dedup => ::cq_gpu_runtime::DeviceSet<_>),
    ];
    print = ::cq_gpu::print(tile(256,4), frontier(8), directory(rank));
}
```
`prefix_runs` also replaces integral composite-key `HashedRun` payload builds
consumed by probes with recorded lexicographic `PrefixRun` levels. Membership-only
and nonintegral hashed builds keep their representation. `retain_lookups` carries
the exact full-prefix range from an eager semi to its matching probe, and
`expand_lookups(256)` can then balance that range across the tile instead of
serially scanning hash-collision buckets per driver row. These are checked
Structures decisions; neither the printer nor the runtime chooses the plan.
`expand_lookups(N, depth D)` explicitly bounds nested expansion frames per stream;
the default depth is two. Eligible probes are staged in stream order up to that
limit; deeper probes keep their recorded provider and traverse it lane-locally.
The bound is not a byte-budget proof: carried payloads and frontier claim space
also occupy shared memory, and the printer/device launch contract remains final.
The ddisasm consumer opts into `output(retry)` for its default hole pipeline,
using direct bounded kernels and exact post-kernel dedup for Compacting outputs.


The displayed printer arguments are optional; these are their defaults.
`frontier(0)` disables descriptor publication. The supported tile shape is
256 threads × four items, and the supported directory is `rank`; other
shapes/directories are rejected rather than silently ignored.
Without `output(retry)`, `capacity(Relation)` additionally records an explicit
output allocation bound from a named input relation. H05 records
`capacity(Lineitem)`; its chained identity query records `capacity(Input)`.
These are allocation contracts, not inferred functional dependencies, uniqueness
or sortedness.

`output(retry)` explicitly selects a bounded, no-resume kernel instead of the
default resumable execution policy. `capacity(Relation)` supplies the named
input's allocation size as an initial hint, not a hard output bound. Generated
output records `__CQ_GPU_OUTPUT_POLICY = "retry"` and
`__CQ_GPU_RETRY_DEFAULT_CAPACITY = 1024`. Only `output(retry)` is accepted;
other values and duplicate output options are rejected. Omitting the option
preserves the existing execution policy.

Retry requires immutable input snapshots and a deterministic exact candidate
total across every attempt. Each attempt traverses the entire stream and keeps
counting every valid pre-admission candidate past capacity while skipping
excess writes or insertions. Only capacity overflow permits a retry: the runtime
discards the partial output and any partial set, resets the frontier inside the
launch callback, and relaunches the entire kernel without saved lane or
shared-memory checkpoints. The candidate count is a safe upper bound on distinct
rows, not an allocation target for fused execution. A changed candidate count,
counter wrap, CUDA error, allocation failure or other non-overflow error is
terminal. Failed attempts never publish partial results.

For fused retry, `execute_retry_fused(site, bounds, capacity_hint, launch)` uses
the generated `concat!(module_path!(), "::", stringify!(kernel))` site and an
optional capacity hint. A thread-local `fused_retry_scope(label)` guard labels
Datalog rule/key holes, restores the previous label on drop, and cannot move
between threads. The runtime retains estimates per `(hole, site, row width)`.
The initial capacity is `max(1024, hint.unwrap_or(0))` on the first call and
`max(1024, 2 * previous_nonempty_distinct + hint.unwrap_or(0))` thereafter.
Only successful nonempty distinct cardinalities update the estimate, never
candidate counts or failed partial rows. Empty successful rounds still report
`last_distinct = 0`, but do not erase the size estimate for the next nonempty wave.
Bounded retry likewise retains the last **nonempty** successful candidate count:
an intervening empty round cannot force the next wave to rediscover its size.

Overflow growth targets **4 times the observed distinct occupancy**, capped by
the immutable candidate upper bound, the u32 row-index domain and the available
VRAM/pool budget. It never allocates the candidate count merely because that
count was observed; that cap applies only when below the geometric target.
Budgeting includes reusable cached pool bytes as well as CUDA free memory,
reserves safety headroom, and accounts for allocator growth requests rather
than assuming allocation can fall back to an exact-sized request. Capacity must
strictly increase; exhausted budget or lack of progress is terminal. There are
at most **12 total attempts** (the 1024 floor with 4x growth spans the u32 domain).

A recorded `DeviceSet<_, _, Fused>` stays genuinely fused under `output(retry)`:
`execute_retry_fused` supplies a `FusedRetryView`, and `fused_retry_insert`
performs exact admission while candidates stream through the direct bounded
kernel. A separate ticketed exact-key arena lets the set publish complete keys
with the CAS/BUSY/publish-row protocol independently of final output compaction.
Only new admissions become output hits; duplicate and capacity-overflow results
emit nothing. BUSY collisions use warp-only reconvergence/ballot retries, never
a pending block scan or a BUSY lock held while waiting. The overflow flag is
checked once per candidate chunk, not once per resumable program counter.
Even after that flag is set, all original valid candidates contribute to the
count needed for a safe retry allocation.

For a 256-thread, four-item chunk (at most 1024 candidates), one packed u32
block scan combines new-admission count with candidate count:
`__tile_count | (__tile_candidates << 11)`. The low 11 bits give unique-output
offsets/count and the high bits give the full valid-candidate count. Thread zero
updates the checked u64 candidate counter before at most one output reservation;
only new keys are scattered. Staged Smallest and nested expansions retain their
bounded descriptor footprint, with no resumable PCs, register checkpoints or
extra pending-work block scan. Successful fused retry output needs no
post-execution dedup for that canon.

Other recorded policies (`Staged` and `Compacting`) use `execute_retry` and
staged canonicalization only after successful materialization, rather than
suspension-time compaction. Their first call starts at 1024 rows or the explicit
hint; subsequent calls use `max(1024, previous_successful_candidates, hint)`
clamped to the supported row limit, isolated by hole, kernel site and tuple width.
Unlike fused distinct estimates, candidate history has no multiplicative slack.
The initial output must fit reusable pool bytes plus free VRAM minus the greater
of 64 MiB or 5% VRAM, respecting the pool maximum. An oversized estimate selects
a zero-capacity count-only attempt, then allocates the exact checked count if it
fits the refreshed budget; an exact count that cannot fit is a resource error.
Only successful checked counts update this history. One overflow relaunch is
allocated at the exact candidate count; a second failure is terminal. Both bounded
policies retain the already-checked successful row count as the table's capacity
bound, without copying or reallocating its buffers, so downstream operators do not
launch over unused output slack. A successful synchronized summary also discharges
all checked immutable ancestor statuses: retained output keeps its own status,
not an already-validated ancestor list. Dependency failures still terminate before
publication or history updates. Without `output(retry)`, recorded
`Fused` continues to use resumable fused execution. Compared with direct bounded
execution, retry adds a checked host control/count readback for each attempt,
even when the initial allocation fits; it does not download tuples.

Runtime bookkeeping initializes each column-view/pointer/count table with one
asynchronous copy from a reusable pinned arena, rather than one CUDA launch per
element. The arena is rewound only after an existing checked readback or explicit
finish has completed the stream, and is retained through error-path teardown.
Immutable column descriptor tables are cached by their column owners across
executions; expired owners retire the cache entry at the next checked boundary.
The per-round check gathers all member counts and distinct ancestor errors in
one kernel and one readback, without a summary memset. Private admission deltas
defer capacity validation to that check; other bounded/resumable outputs retain
their validation before dependent operators consume them. `CheckedRound::rows`
returns a checked count only for the exact immutable metadata owner (including
projections), without a new readback.

CUDA stage and fused-attempt event timings require the runtime `timings` feature.
Without it, timing fields are zero and no timing events are created or recorded.
Allocator completion events remain enabled and pooled per stream; their ordering
is required for safe cross-stream reuse. Frontier reset copies both counters in
one upload and skips the duplicate pool memset on the first reset.

The extra recorded passes are necessary. The shorter
`order(le)` + `sorted_runs` + `retain_lookups` pipeline still records host
fallback indexes for unfiltered Supplier/Lineitem and a CPU HashSet at
canon. `build_host_reads` is a CPU-neutral Physical mode rewrite: eligible
integral-key host renames become retained `let` results, without changing
the graph, keys, predicates or their order. `prefix_runs` explicitly retypes
an existing multi-level Trie to `cq_gpu_runtime::PrefixRun`. Without arguments
it records the existing first singleton integral level. An explicit
`prefix_runs(Supplier => supplier_key)` selects a singleton level by base
relation or retained-result name and moves it to the directory position.
The provider's fourth const argument records that column's Structures layout
index; the printer rejects disagreement with the first level. All recorded key
columns, in level order, form a stable lexicographic trie, with original row ID
as the final tie-break. A device-only blocked counting sort bins the first key,
then heapsorts each bin lexicographically by the remaining recorded keys and
original row ID; payloads are gathered once. Two-level prefixes pass their sole
secondary-key descriptor inline as a copyable kernel argument, with no metadata
allocation, copy or event. For three or more levels, the runtime selects
`build_prefix_stream_ordered`: descriptor initialization is stream-ordered on
device, so retiring the run needs no pinned-copy completion event or host wait.
The standalone `Run::build_prefix` API retains its pinned-upload behavior.
One all-level sortedness/domain check
supplies the final build status, without host readbacks.
Fully lexicographically sorted inputs take the ordinary
adjacent-difference directory path: histogram clearing, histogram, scatter and
in-bin ordering return immediately on sorted or invalid status. Prefix builds
replace only `sorted_check` and `order_bins` with their prefix variants; all
other launches and their order match the single-key pipeline. Both enqueue
12 fixed launches plus one gather per payload and two exclusive scans (whose
recursive launch counts depend on directory size), regardless of sortedness.
There are no per-key passes, permutation composition, next-key gathers or merge
launches. A one-level PrefixRun uses the ordinary single-key build.
The host-only `build_sequence` test target checks this source-level launch and
gating contract without loading CUDA or running the device oracle.
Level one retains the blocked-run rank directory;
deeper payload reads narrow that range with lower/upper-bound binary searches.
A fully bound probe in a pure stream ending in recorded canon is realized as
lane-local membership: duplicate witnesses cannot add an output value. Its final
recorded level uses lower-bound plus equality, not a second expansion or an
upper-bound search. This does not assume provider-key uniqueness; a supplier
directory can still contain several distinct nations. Recorded levels never use
a residual linear scan.
Displaced singleton-prefix reads receive
additional recorded one-level SortedRuns and retained Vec rows, rather than
silently losing semijoins. B therefore preserves its nation-key semijoin while
its full Supplier lookup uses the supplier-key directory.

Recorded Physical `probe smallest` binary intersections require **both** candidate
providers to be explicitly recorded `PrefixRun`s with levels `[prefix][v]`,
where `v` is the singleton candidate column. Run structure selection and
`prefix_runs` to record those sorted tries; a plain `SortedRun` is not an
implicit fallback. Missing prerequisites produce `CQ_GPU_SMALLEST_TRIE`,
naming the affected side and provider.

For an adjacent supported probe/check pair (the second read may be `probe`
or eager `semi`), generated kernels compare the two prefix-range lengths,
expand the shorter range, and check each candidate in the other range with
a boolean lower-bound/equality binary search. The second Physical read is
not emitted as another expansion. Ordinary recorded 256-row expansion stages
also honor `tile(256,4)`: root tiles hold 1024 rows, each nested descriptor holds
1024 parents, and candidate chunks hold 1024 candidates. Smaller explicit stage
batches retain their existing scalar-item shape. A certified membership-only
probe with recorded constants is a total lane-local step, so it no longer blocks
`expand_lookups` from recording subsequent range-expansion boundaries.

When the staged intersection retains driver-row identity, its compact descriptor
stores u32 driver row IDs and packed u16 parent slots: the low 15 bits select the
parent and the high bit selects the shorter adjacency. Candidate execution
reloads demanded driver columns from the resident table; copied driver-column
bindings also reload, rather than adding i64 carrier arrays. Only demanded
non-driver values (such as a literal binding used by the result) need i64
carriers; unused bindings do not enlarge the descriptor. The other range's
begin/end bounds are not stored: one selected-branch rank lookup recovers that
prefix range per candidate before binary-search membership. Generated source
therefore has four rank-point call sites for the minimal staged pair (two root
lookups and two alternative candidate reprobe branches), not four executed
lookups per candidate. Recorded witnesses can supply the root lookups; they do
not eliminate this compact candidate-range reprobe. Unstaged serial traversal
retains the original ranges and does not add the candidate reprobe.

For the minimal two-driver-column intersection without extra carried values or
witnesses, the descriptor occupies 3588 u32 words: two header words, 1024 range
starts, 1025 exclusive offsets, one alignment word, 1024 driver row IDs, and
512 words of packed parents. With 16 scratch words, generated dynamic shared
memory is **14,416 bytes**, rather than 73,832 bytes for the former full-value,
full-range layout with a separate stolen-task descriptor. A kernel with exactly
one expansion descriptor uses the same scratch-plus-16 location for root and
stolen tasks, without a second descriptor or claim-dispatch descriptor copy.
This applies to bounded, retry and resumable execution; resumable execution still
checkpoints shared state when suspending, while retry has no checkpoint state.
Each additional demanded non-driver i64 value adds 8192 bytes at this tile size.
These are generated-layout sizes,
not new GPU resource measurements; the handwritten triangle kernel's measured
14,468-byte footprint above includes its own scratch layout.

The compact layout depends on recoverable driver-row identity and also applies
to ordinary nested range expansions. Exact join-key bindings are reloaded from
the selected run row, while residual/hash-collision keys retain their original
bindings. Other demanded non-driver values and witness cursors remain carriers.
An ordinary frame with only root identity occupies 3076 u32 words (12,304 bytes);
Smallest additionally uses its packed parent/side array. Nested expansions keep
separate live frames. If the root frame is the largest, stolen work reuses that
area; later resumable frames receive disjoint copies rather than allocating
another shared claim area. Otherwise the separate claim area remains necessary.
Candidate-count scans reject any
per-item count above `u32::MAX / 256` before their 256-lane scan; stripe totals
accumulate in `u64` and descriptor totals must fit u32. Compact driver row IDs
are checked to fit u32 rather than silently truncated. Each candidate chunk
uses one output scan and reservation, with no second overflow scan. Default
execution checkpoints the cooperative chunk for bounded grow-and-resume;
explicit capacity retains its allocation contract unless `output(retry)` makes
it an initial hint. Retry uses the same compact **14,416-byte** bounded shape
without resume parameters or local checkpoint registers. Host token tests cover
this generated control flow; these checks make no GPU timing claim.

`hashed_runs` records one-level composite integral/bool/dictionary-key HashIndex
providers as `cq_gpu_runtime::HashedRun<_, _>`. It follows `sorted_runs`, leaving
single-key selection unchanged. The device hashes every recorded key into a
capacity-sized power-of-two directory (capped at 2^24 buckets), then uses the
same blocked counting-sort build without host cardinality readback. Every probe
compares **all** key columns within the bucket; collisions never imply equality.
Standalone `cq_gpu::print` accepts this recorded type and rejects an unretyped
HashIndex. `Runtime::with_hashed_run_bucket_limit` can cap directories further
for memory control or deliberate collision testing without changing answers.

| Checked Structures/Physical fact | GPU realization |
|---|---|
| Host rename and relational select/point/range | Resident column scan and device comparisons; string equality uses the host's shared dictionary IDs. Ordinary tuple bindings explicitly take the upload path. |
| `let` with one-level `SortedRun` | One blocked run, built on device using the recorded integral key. |
| Explicit one-level composite `HashedRun` | Blocked run of deterministic hash buckets, with exact residual comparison of every key column. |
| Explicit `PrefixRun` key levels | Stable lexicographic device build over all recorded keys; rank lookup on level one, binary-search range narrowing on deeper levels, then non-empty membership or narrowed-range expansion. |
| Recorded `smallest` binary intersection with two explicit `[prefix][v]` `PrefixRun`s | Cooperative 1024-candidate chunks (four items per thread), shorter-range expansion and binary-search membership in the other range, rather than a second expansion. |
| Integral membership-only provider | Budgeted exact bitmap, not an inferred payload run. |
| Recorded `join_filter` | Budgeted bitmap prefilter on its recorded scan column, retaining the covering exact keyed read. |
| Eager semi without a witness | Exact membership test at the recorded keyed read. |
| `share` plus matching `repr witness` | Retained `(begin,end)` descriptor, consumed at the recorded forcing probe without repeating its root lookup; compact staged `smallest` candidates separately reprobe the other prefix range as described above. |
| Probe | Unstaged exhaustive lane-local range traversal; recorded batch/expand streams retain block scans, owner search and bounded cooperative expansion. `first` never implies uniqueness. |
| `stage = batch N` / `expand` | Bounds cooperative work by the recorded batch; only explicit expand stages publish large descriptors to the bounded frontier. |
| Project | Carries only demanded values to output. |
| Recorded `DeviceSet<_>` canon | Exact device dedup; hash collisions never establish equality. |
| Returned stream | Owned resident device columns, with an explicitly recorded capacity contract, bounded grow-and-resume execution, or explicit `output(retry)` count-and-relaunch execution. Tuple transfer requires `.to_host()`. |

`DeviceSet<Row, Codec = Plain, Policy = Staged>` records both storage encoding
and dedup scheduling. `Staged` retains standalone behavior: materialize all
candidates, then canonicalize. `Compacting` exact-deduplicates a completed prefix
when a resumable kernel suspends, preserves its query checkpoints, and resumes
into the compacted output; growth uses the checked distinct cardinality instead
of raw candidate multiplicity. The printer also supplies the maximum pending
reservation: one row for serial output or `256 * items` for cooperative output.
After compaction, capacity is at least the distinct prefix plus that bound, so
a four-item, 1,024-row append cannot livelock behind a scalar-block allowance.
An append that cannot fit within the u32 row domain is rejected. This policy
adds a checked count transfer at compaction boundaries, but never transfers
tuples. The Datalog pipeline records
`DeviceSet<_, Plain, Compacting>` by default through `retype`; default or per-hole
pipelines select `Fused` with the same ordinary consuming-pass argument.
The printer reads the recorded representation, never query or dataset names.
`Fused` records exact insertion during
execution; unknown policies are rejected. Explicit `output(retry)`
preserves fused admission with distinct-based growth for `Fused`, while
`Staged` and `Compacting` canonicalize after successful materialization.

Each generated kernel records its Structures keyed-read provenance. In addition
to one-level payload providers, the existing `share` / `repr witness` contract
admits an exact first-level prefix of an explicitly recorded `PrefixRun`.
`retain_lookups` can retain both earlier eager prefix semis feeding a `smallest`
intersection: each share names its own provider and prefix, and both force at
the introducing `smallest` probe. The witnesses keep the existing
`RunCursor<'_, _>` representation; the printer selects each by forcing probe,
provider and prefix, not by forcing probe alone. No deeper-prefix or unrecorded
witness is inferred. Inputs must remain immutable during execution.
Resident `DeviceColumns<'data, Row, C = Plain>` hosts own their encoded columns,
row metadata and string dictionary domain. Column allocations retain the
shared pool, CUDA stream and context through reference-counted ownership:
neither inputs nor answers borrow a temporary execution `Runtime`. Passing
`&DeviceColumns` to another compiled query shares the allocation rather than
columnizing or uploading it again. The terminal returns an owned device host,
not a host iterator; `.to_host()?` is the explicit tuple-transfer boundary.
The H05 loader deliberately drops its construction runtime before executing
either plan. Ordinary tuple bindings still upload for that execution; they do
not create a persistent cache keyed by host pointer.

### Resident host codecs and recorded derived codecs

Host compression is a property of the supplied value, not a printer decision
and not an extra IR host-codec entry. The loader supplies `Codec::Plain`,
`Codec::For`, `Codec::Delta` or `Codec::Dictionary` to `Runtime::upload_owned_rows`;
device encoders prepare 1024-value FastLanes vectors. Generated loads consume
the column/vector metadata through the device decoder. They do not first
download or expand the whole relation to host tuples. H05 calls
`detect_integer_codec` on each numeric column during loading; this estimates
the actual vector widths and selects delta only when at most 1/16 adjacent
transitions descend and delta is smaller including its base metadata.
Region names use dictionary IDs. All six relations are loaded once using
one `Dictionary` and runtime, so string equality has one compatible ID domain.
`upload_owned_rows` uses `OwnedRowCodec` tuples and an owning string dictionary,
returning `DeviceColumns<'static, Row>` without retaining a borrow of the
source tuples. The separate `upload_rows` path preserves existing borrowed
string hosts. The `'data` parameter tracks host string/binding compatibility,
never an execution runtime lifetime.

Physical `host_set Edges;` is an opt-in, checked fact about the complete
declared heading, never a projected key. The GPU printer certifies the bound
host before any query consumer runs: a device pass checks every adjacent
full-row pair for both sortedness and duplicates. Sorted distinct inputs need
no hash allocation; unsorted inputs use an exact device hash table of immutable
row IDs, resolving every collision by comparing all columns. Only count/status
metadata is read back, not tuples, including for resident and frozen hosts.
Duplicate rows fail with `CQ_GPU_HOST_SET_DUPLICATE` naming the host relation.
No fact means no certificate work; projecting away columns still requires its
own set proof or canonicalization.
The Physical rewrite is selected as `physical = [cq_rewrites::host_sets(Edges)]`;
its relation-name advice is consumed there, not by the printer.
`DeviceColumns::checked_len()` validates all retained statuses and reads only
the logical count. The compiled triangle runner uses it for count/timing runs
and calls `to_host()` only for the first `--check` full-tuple comparison, avoiding
unnecessary decoded-device scratch and host tuple allocation for large outputs.

The 64-bit FastLanes layout has sixteen lanes and the `04261537` unified
transposed ordering. A packed vector of width W consumes W × 128 data bytes.
Each vector has a 32-byte `VectorHeader` recording its word offset, base,
width, codec and valid-row count. Delta uses sixteen independent 64-value
sequences with sixteen 64-bit restart bases, avoiding a whole-vector serial
prefix dependency; its signed deltas are frame-of-reference packed.
Host loading obtains the exact encoded extent before allocation. Derived
outputs retain a conservative device allocation rather than reading their
extent back merely to resize the answer.

Resident execution retains allocation capacity separately from the device live
count. Result columns and decode scratch are allocated without clearing unused
tails; every consumer is count-gated. Plain results adopt their native column
buffers and count owner without encoding or copying. Recorded compressed results
build headers, offsets and payloads only for live vectors, including the partial
last vector. A runtime retains dedup scratch capacity across calls, but clears
the device-count-sized active set on each call; no relation data is reused.
Membership bitmap allocations are capped by the recorded key bounds as well as
the recorded byte budget. Runtime and generated code-object caches retain the
actual context owner, not just a potentially recycled context identifier.
Sorted-run starts/directory construction and per-bin ordering use bounded grids
with device-count/domain grid strides, avoiding capacity-sized grids whose
threads immediately exit on sparse filtered relations.

Derived columns carry conservative source-variable bounds through joins,
projection, deduplication and re-encoding. `Bounds::Empty` represents a known
empty input without invented extrema; a filtered result may retain nonempty
source bounds while its live count is zero. Both bounded and grow/resume
outputs retain this metadata, allowing the next resident query to build a
sorted run over the answer. Dense runs still reject domains beyond 2^32 keys.
Membership builds use the known domain directly when it fits the recorded
budget; otherwise a bounded parallel signed-extrema reduction checks the
actual live domain on-device. Codec word offsets use hierarchical 256-thread
u64 prefix scans, not a single-thread walk over every vector.

With `cq-gpu-runtime/timings`, events are recorded during submission. Standalone `finish` completes
and collects them once; Datalog holes retain them for the shared checked round
readback and final program completion. Stages do not synchronize to accumulate
elapsed time. Both boundaries propagate asynchronous CUDA errors; explicit
downloads and grow/resume control-word reads retain their required waits.
Timing intervals still include GPU idle gaps within each recorded stage.

Derived storage is different: its codec is an explicit Structures provider
fact, selected by a rewrite rather than guessed from a query or Rust host
value. The default recorded device canon is plain:

```rust,ignore
::cq_rewrites::retype(dedup => ::cq_gpu_runtime::DeviceSet<_>)
```

Both H05 plans instead record packed frame-of-reference answers:

```rust,ignore
::cq_rewrites::retype(dedup =>
    ::cq_gpu_runtime::DeviceSet<_, ::cq_gpu_runtime::For>)
```

Their return type is `DeviceColumns<'_, (i64, i64, i64), For>`. A second compiled
identity query binds this device host and records an ordinary plain
`DeviceSet<_>` answer. Only that second result is downloaded. This makes the
packed-output-to-resident-input boundary part of every H05 exactness run,
without changing either H05 clause order or its keyed-read facts.

The design follows the register/tile decoding motivation in Afroozeh, Felius
and Boncz, [*Accelerating GPU Data Processing using FastLanes Compression*,
DaMoN 2024](https://doi.org/10.1145/3662010.3663450)
([paper](https://ir.cwi.nl/pub/34260/34260.pdf)): keep compressed columns in
VRAM and decode values where the query consumes them, rather than writing
an entire decompressed relation back to global memory. The paper's speedups
are not measurements of this Rust implementation. The 1024-value storage
vector is not a requirement to keep 1024 decoded values live per thread.

Root decoding keeps two named scalar values at a time rather than an indexed
local-memory array. Each warp owns paired positions in two 64-value Delta sequences; one
warp-prefix sum per sequence replaces duplicate lower-half decoding. Narrow
residuals use exact 32-bit sums with the signed frame base added separately;
wider residuals retain wrapping 64-bit arithmetic. Selection kernels decode
convergently before validity filtering. These paths use lane shifts and warp
shuffles, not shared-memory decode buffers or block barriers.
Root column descriptors are passed as immutable kernel parameters, retaining
their device count pointers; there is no per-tile descriptor-table round trip.
Serial scan kernels request four resident blocks per multiprocessor to limit
register pressure. Packed payload accesses remain scalar/coalesced UTL loads:
paired logical values are not contiguous packed words, so blindly substituting
vector loads would fetch unrelated lanes.

### Lifetime, VRAM and capacity errors

All device buffers, including codec headers, payloads and scratch space, use
the `rccl-memory` pool. No RCCL device algorithm, managed-memory spill, CPU
fallback or out-of-core partitioner is introduced. The working set must fit
VRAM: resident inputs plus indexes, transient encoding/build storage and
output capacity all count. Compression of final columns alone does not bound
peak memory. Allocation failure is reported as `CQ_GPU_VRAM_EXCEEDED`;
reducing the dataset or explicitly chosen capacity is a caller decision,
not an implicit change of query semantics.

Pool/stream/context ownership follows the columns, so a device result can
outlive the runtime that produced it and feed another query. Explicit capacity
overflow is `CQ_GPU_OUTPUT_CAPACITY`, retained as device status and surfaced
at the explicit download boundary; it is not a truncated successful answer.
No tuple readback is required between the two compiled H05 calls.
Every packed or native column retains its own logical-count allocation owner,
including a column cloned out of a relation before the relation is dropped.
Raw column descriptors require unsafe access with an explicit owner/stream
lifetime contract; safe `to_host` keeps the metadata alive itself.

The current runtime regressions and targeted membership measurement are
independent of the historical H05 baselines:

```sh
cargo oxide build --arch sm_89 -- --release -p gpu-device-columns-oracle --bins
./target/release/gpu-device-columns-oracle
./target/release/runtime-oracle
./target/release/runtime-oracle --membership
```

The codec oracle passed 599 cases, including multi-block offsets and retained
packed-count owners. The runtime oracle covers escaped-column lifetime,
resident outputs indexed by the next execution, empty/negative domains,
conservative-domain membership reduction and retained capacity errors.
On RTX 6000 Ada, five unprofiled four-million-row membership builds measured
403.801–403.816 ms before versus 0.103–0.134 ms after using lawful loader
bounds. This is the membership build only, not an H05 speedup. Exact inputs,
samples, original bug reproductions and a separate Nsight trace summary are
retained in `compare/gpu_runtime_membership.audit.results.json`; its six
profiled query-build intervals contained no HtoD/DtoH copies.
The six-relation synthetic resident H05 smoke also passed both plans and their
chained query on all six calls per plan (579 exact rows each); its separate
record is `compare/gpu_h05_compiled.runtime-audit.smoke.results.json`.

### Running the compiled pipeline

```sh
# Use the CUDA_HOME/PATH setup in Reproduce above.
cargo oxide build --arch sm_89 -- --release -p gpu-h05-compiled --bin printer-smoke
./target/release/printer-smoke
cargo oxide build --arch sm_89 -- --release -p gpu-h05-compiled --bin gpu-h05-compiled
./target/release/gpu-h05-compiled "$TPCH_DATA_ROOT/sf1" both compare/gpu_h05_compiled.resident.optimized.results.json
./target/release/gpu-h05-compiled "$TPCH_DATA_ROOT/sf10" both compare/gpu_h05_compiled.resident.optimized.results.json
```

The standalone smoke compares GPU answers with the pull printer, including
empty input. Each H05 plan and its chained compiled identity query check exact
sorted answers against independent `answer.tsv` after one warmup and five
measured runs. Region names are decoded from the input file dictionary once
and assigned IDs in the shared resident dictionary during loading. Dates are
integral days; the manifest must match the authored 1994 window.

The resident JSON records, separately:

- `one_time.host_load_ms`: projected-file and oracle loading.
- `one_time.load_encode_ms`: runtime initialization, row preparation, upload
  and device encoding, once for all six relations. Each relation also records
  its `load_encode_ms` around `upload_owned_rows`, chosen codecs, row count,
  logical `raw_bytes` and `fastlanes_bytes` including vector headers and
  extent metadata. These byte counts are not allocator high-water marks,
  PCIe counters or the size of host string allocations. Codec selection and
  tuple preparation are in the all-relations load interval, not the narrower
  per-relation `upload_owned_rows` interval.
- `build_ms`, `kernel_ms`, `dedup_ms`, `total_ms`: runtime GPU timings (zero unless `cq-gpu-runtime/timings` is enabled) for
  the first compiled query; `wall_ms` is that resident call's host elapsed
  time, excluding initial loading, the chained call and final download.
- `calls`: individual warmup and measured call facts, including separate
  wall/build/kernel/dedup/total values for the second compiled query, packed
  answer allocation bytes, exactness and the final `download_ms`.
  Resident result `raw_bytes` describes uncompressed capacity, not an implicit
  readback of live row count; encoded answer storage may be conservative.

The repaired resident results are in `gpu_h05_compiled.resident.optimized.results.json`;
the before/after comparison and evidence ledger are in
`gpu_h05_compiled.resident.summary.json`. Profile separately from unprofiled samples:

```sh
CQ_GPU_PROFILE=1 nsys profile --trace=cuda --capture-range=cudaProfilerApi \
  --capture-range-end=stop --force-overwrite=true -o compare/compiled_resident_optimized_A \
  ./target/release/gpu-h05-compiled /data/tpch-gpu/sf10 A \
  compare/gpu_h05_compiled.resident.optimized.profile.results.json
nsys stats --report cuda_gpu_kern_sum,cuda_api_sum --format csv \
  compare/compiled_resident_optimized_A.nsys-rep
ncu --target-processes all --kernel-name 'regex:cq_gpu_stream_0_r21' \
  --launch-skip 1 --launch-count 1 \
  --metrics dram__bytes.sum,gpu__time_duration.sum,dram__throughput.avg.pct_of_peak_sustained_elapsed \
  --force-overwrite -o compare/compiled_resident_optimized \
  ./target/release/gpu-h05-compiled /data/tpch-gpu/sf10 A \
  compare/gpu_h05_compiled.resident.optimized.ncu.results.json
/opt/nvidia/hpc_sdk/Linux_x86_64/25.5/cuda/12.9/bin/ptxas \
  -v -arch=sm_89 target/oxide-ir/gpu_h05_compiled.ptx -o /dev/null
```

`CQ_GPU_PROFILE` brackets the first measured resident call and its chained
query with CUDA profiler control APIs. Loading and the final explicit download
are outside that capture range, so transfer absence can be checked directly
rather than inferred from whole-process transfer totals.

The codec oracle is a separate binary-only package to avoid the pinned
cuda-oxide package-loader ambiguity:

```sh
cargo oxide build --arch sm_89 -- --release -p gpu-device-columns-oracle
./target/release/gpu-device-columns-oracle
```

It compares independently constructed paper-layout bits and headers, not only
encode/decode round trips, and exercises empty/partial vectors, signed extrema,
all widths, per-lane delta restarts, and device-count-below-capacity encoding.

#### Resident phase repair measurements (2026-10-18)

RTX 6000 Ada, sm_89; one warmup and five unprofiled measured calls. Both plans
and their chained queries were exact in every SF1/SF10 call (7,242 / 72,983
answers). The final codec oracle passed **595** cases, the blocked-run oracle
passed **42** data cases plus invalid-input checks, printer smoke/expansion
oracles passed, and **32** printer tests including UI fixtures passed.

SF10 A medians, from `gpu_h05_compiled.results.json`,
`gpu_h05_compiled.resident.before.results.json` and
`gpu_h05_compiled.resident.optimized.results.json`, respectively:

| Per-call median | Pre-resident re-upload | Initial resident | Repaired resident |
|---|---:|---:|---:|
| Build | 1.972 ms | 14.512 ms | **1.880 ms** |
| Kernel phase | 1.044 ms | 11.338 ms | **1.004 ms** |
| Dedup phase | 0.069 ms | 12.459 ms | **0.039 ms** |
| Total | 3.087 ms | 38.319 ms | **2.924 ms** |
| Wall | 894.274 ms | 39.554 ms | **3.054 ms** |

All five repaired A totals are below 3.5 ms; the largest wall-minus-total
difference is **0.143 ms**. Phase medians need not sum to the median total.
Warmup is retained in JSON but excluded from these medians; code loading and
initial pool growth are not steady-state query costs.

| Repaired plan | SF | Build ms | Kernel ms | Dedup ms | Total ms | Wall ms | Exact |
|---|---:|---:|---:|---:|---:|---:|---|
| A | 1 | 0.491 | 0.130 | 0.033 | 0.656 | 0.778 | yes |
| B | 1 | 1.408 | 0.115 | 0.035 | 1.558 | 1.699 | yes |
| B | 10 | 15.090 | 0.488 | 0.037 | 15.620 | 15.776 | yes |

Fresh nsys captures bracket the first measured compiled query and its compiled
consumer, not just the main kernel. The following complete operation list sums
the first query's build/kernel/dedup work, excluding the consumer. Full per-phase
lists, launch counts, memset bytes, timelines and event residuals are recorded
in `gpu_h05_compiled.resident.before.attribution.json` and
`gpu_h05_compiled.resident.optimized.{A,B}.attribution.json`.

| SF10 A operation | Before repair GPU ms | After repair GPU ms |
|---|---:|---:|
| `memset` | 13.403 | 0.203 |
| `offsets` | 11.951 | 0.009 |
| `encode` | 5.044 | 0.071 |
| `cq_gpu_stream_0_r8` (Orders selection, two passes) | 2.940 | 0.568 |
| `headers` | 1.797 | 0.020 |
| `cq_gpu_stream_0_r21` (main) | 1.790 | **0.988** |
| `gather` | 0.258 | 0.262 |
| `sorted_starts` | 0.156 | 0.121 |
| `sorted_directory` | 0.142 | 0.104 |
| `order_bins` | 0.128 | 0.005 |
| `word_offsets` | 0.074 | 0.074 |
| `decode_column` | 0.065 | 0.006 |
| `sorted_check` | 0.041 | 0.041 |
| `set_insert` | 0.040 | 0.013 |
| `scatter` | 0.038 | 0.038 |
| `histogram` | 0.038 | 0.038 |
| `cq_gpu_stream_0_r5` | 0.036 | 0.038 |
| `scan_blocks` | 0.035 | 0.035 |
| `initialize_pointer` | 0.022 | 0.021 |
| `initialize_view` | 0.020 | 0.019 |
| `add_prefix` | 0.015 | 0.015 |
| `initialize_count` | 0.010 | 0.010 |
| `pack` | 0.010 | 0.004 |
| `fences` | 0.008 | 0.007 |
| `validate_output` | 0.007 | 0.006 |
| `cq_gpu_stream_0_r3` | 0.006 | 0.006 |
| `cq_gpu_stream_0_r2` | 0.005 | 0.005 |
| `clear_histogram` | 0.005 | 0.005 |
| `complete_stats` | 0.005 | 0.005 |
| `initialize` | 0.004 | 0.004 |
| `count_u32` | 0.004 | 0.004 |
| `run_status` | 0.004 | 0.004 |
| `membership_insert` | 0.002 | 0.002 |
| `membership_bounds` | 0.002 | 0.002 |
| `clear_set` | 0.000 | 0.003 |

There was **no individual 10-ms kernel**. The regressions were aggregates:
initialization plus codec work consumed **10.271 ms in build**, **9.549 ms in
the kernel phase**, and **12.373 ms in dedup**. The main kernel added 1.790 ms
to the kernel phase; actual set insertion took only 0.040 ms. Clearing 5.398 GB
and encoding capacity-sized Plain columns, not set probing, caused the dedup
regression. The repaired set clears only its device-count-sized active prefix;
recorded For result encoding processes 72,983 rows, not 59,986,052 capacity rows.

| nsys A phase | Before GPU work ms | After GPU work ms | Before event residual ms | After event residual ms |
|---|---:|---:|---:|---:|
| Build | 14.322 | 1.729 | 0.577 | 0.541 |
| Kernel | 11.348 | 0.996 | 0.031 | 0.019 |
| Dedup | 12.431 | 0.027 | 0.037 | 0.028 |

Residuals are event-interval time not occupied by the listed device operations:
launch gaps and host/profiler overhead, not attributed to invented kernels.
Profiled timings are not substituted for the unprofiled table. Plan B's separate
trace reports a 0.475-ms main kernel; its larger build includes 5.773 ms of
per-bin stable ordering and 1.532 ms of Lineitem decoding, retained explicitly
in the B attribution rather than hidden in A's improvement.

One-time SF10 load/encode is **1,893.642 ms**, including runtime setup,
host column preparation, upload and completed GPU encoding; file/oracle loading
is separately **1,578.936 ms**. Input storage remains:

| Relation | Raw column bytes | FastLanes bytes |
|---|---:|---:|
| customer | 24,000,000 | 1,218,896 |
| orders | 360,000,000 | 74,534,136 |
| lineitem | 959,776,832 | 176,211,664 |
| supplier | 1,600,000 | 81,552 |
| nation | 400 | 592 |
| region | 80 | 592 |

Small relations expand because vectors retain headers, lane bases and padded
payloads. Byte accounting includes encoded data, per-vector headers and extent
metadata, not pool reserve or host dictionary strings.

Separate ncu kernel-replay captures measure main-kernel DRAM traffic falling
from **532,209,024** pre-resident bytes to **202,497,920 bytes** (**61.95%**
lower). The replay duration is **2.360 ms**, not the unprofiled timing.
`compiled_resident_optimized.ncu.csv` and `.resources.json` retain the evidence:
64 registers/thread, no block barriers in the main kernel, a 64-byte stack,
88 bytes of spill stores and 104 bytes of spill loads. The occupancy bound
deliberately trades these spills for the measured 0.988-ms nsys main kernel;
this is not a zero-spill claim.

Both `compiled_resident_optimized_A` and `_B` nsys captures include the first
query and its compiled consumer, excluding loading and final `.to_host()`.
Each records **zero HtoD and zero DtoH calls**, no memcpy activity and no module
loads; A/B contain 30/33 device memsets respectively. Their `_cuda_api_sum.csv`,
`_cuda_gpu_kern_sum.csv` and attribution JSON retain the measured evidence.
The explicit-capacity path measured here does not use `output(retry)` and has
no control readback; generic grow-and-resume's permitted four-byte control flag
and retry's checked control/count reads are separate execution contracts.


The following tables and `gpu_h05_compiled.results.json`, `.profile.json`,
`.resources.json` and `compiled_final.nsys.csv` are **historical,
pre-resident** evidence, not results for the current resident implementation.
The historical JSON entries are labeled accordingly; their recorded samples
are preserved. The original hand-written SF10 totals (A 6.2 ms, B 10.1 ms)
remain in `compare/gpu_h05.results.json`; the subsequent hand-written
comparison is `compare/gpu_h05_compiled.handwritten.results.json`.

Historical measurements on RTX 6000 Ada, 2026-10-18: every warmup and measured
run was exact at SF1 (7,242 answers) and SF10 (72,983). Medians of five
unprofiled runs after one warmup; phase medians need not sum to median total:

| SF | Plan | Implementation | Build ms | Kernel ms | Dedup ms | Total ms |
|---:|:---:|---|---:|---:|---:|---:|
| 1 | A | Compiled | 0.512 | 0.114 | 0.058 | 0.686 |
| 1 | A | Hand-written | 0.274 | 0.093 | 0.461 | 0.830 |
| 1 | B | Compiled | 1.328 | 0.109 | 0.059 | 1.495 |
| 1 | B | Hand-written | 0.831 | 0.043 | 0.463 | 1.337 |
| 10 | A | Compiled | 1.972 | 1.044 | 0.069 | 3.087 |
| 10 | A | Hand-written | 2.336 | 0.943 | 0.475 | 3.760 |
| 10 | B | Compiled | 14.201 | 0.506 | 0.069 | 14.767 |
| 10 | B | Hand-written | 8.417 | 0.304 | 0.480 | 9.202 |

Separate final `nsys` per-kernel medians (six launches, including warmup):

| SF10 kernel | Compiled ms | Hand-written ms |
|---|---:|---:|
| A | 1.031799 | 0.942022 |
| B | 0.503507 | 0.300995 |

Complete build/query/dedup kernel summaries and CUDA API counts are retained in
`compare/compiled_final.nsys.csv` and `handwritten_final.nsys.csv`.

Compiled SF10 A is **1.11×** the fresh hand-written kernel, below the requested
1.3× bound against 0.94 ms, with zero spills. Its build is below both the fresh
hand-written build and the original 4.9 ms reference. B's kernel falls from
92.602 to 0.506 ms; its remaining build and traversal overhead is not hidden.
In these historical captures, host columnization and upload occurred on every
execution; only allocator capacity and kernel code objects were reused.

#### A diagnosis and emitter changes

The old A kernel emitted a cooperative descriptor, two block scans (including
overflow checking), shared parent/witness save/restore and owner search at every
probe. Its four items per thread were four serial 256-row stripes, each paying
the nested synchronization costs. PTX contained **37 barrier sites**, versus
eight in the hand-written implementation. There was no uninlined generic trait
dispatch: generated A PTX contained no calls and no explicit local-memory
instructions. `ptxas` introduced eleven 32-bit spill stores and nine loads
(44/36 bytes), with a 40-byte frame, while allocating the live nested scheduler
state. SASS additionally outlines warp-sync/shuffle sequences as local calls;
these are not Rust trait dispatch.

Unstaged probes now exhaust their full ranges in lane-local nested loops.
This is a cardinality-independent lowering, not a PK inference from `first`.
Recorded batch/expand streams retain cooperative descriptors and frontier
publication. Witness `(begin,end)` values stay in registers and their forcing
read still does not reprobe. Root-column loads are demand-driven, so supplier
keys are read after order/customer filters. Output capacity checking and
grow/relaunch remain exact; the dedup set is touched only by the later dedup
kernel, not by each input row. Integer predicates and dictionary IDs remain
device integer comparisons.

The generated scan retains static grid-stride scheduling; the hand-written
scan uses an atomic dynamic tile cursor and packs four results per thread
before block-coalesced output. Neither examined PTX uses vectorized read-only
`ld.global.nc` loads: both use scalar generic 64-bit loads with coalesced lane
addresses. The speedup is therefore not attributed to vectorization or an
invented read-only-cache optimization. Static PTX/SASS evidence is retained in
`compare/gpu_h05_compiled.assembly.json`.

#### Hardware counters

Separate SF10 A `ncu` kernel-replay captures, one profiled launch each; these
are hardware counters, not the README's logical `scan_gbps` estimate:

| A kernel | DRAM bytes | DRAM % peak | Achieved occupancy % | Barrier stall % | Long-scoreboard stall % | Wait stall % |
|---|---:|---:|---:|---:|---:|---:|
| Compiled before | 994,048,256 | 8.01 | 42.40 | 54.97 | 11.62 | 9.76 |
| Compiled after | 532,209,024 | 36.14 | 58.36 | 0.00 | 62.32 | 16.74 |
| Hand-written | 573,238,912 | 39.65 | 66.48 | 26.01 | 52.49 | 10.44 |

Replay durations were 13.612, 1.616 and 1.587 ms respectively; use the
unprofiled table above for performance ratios. Raw metric names/values are in
`compare/compiled_a.before.ncu.csv`, `compiled_a.after.ncu.csv` and
`handwritten_a.ncu.csv`.

The final B captures report 87,557,504 DRAM bytes at 13.21% peak for compiled,
versus 87,058,816 bytes at 28.81% for hand-written. Achieved occupancy is
52.82% versus 32.81%; long-scoreboard stalls are 75.89% versus 52.04%.
Their replay durations are 0.727488 versus 0.331648 ms. Full B counters are in
`compare/compiled_b.after.ncu.csv` and `handwritten_b.ncu.csv`; all parsed
profiles, original results and intermediate stages are in
`compare/gpu_h05_compiled.profile.json`.

#### Build/runtime changes and resources

In the pre-resident implementation, pure projections shared execution-owned allocations.
Filtered nonexpanding streams, including semijoins, use stable count/scan/scatter
and inherit conservative source bounds. Runs allocate/gather only the selected
logical prefix. Output count/error metadata uses one readback. Disabled
frontiers allocate no counters and launch no initializer.

Profiling also exposed a dominant host-side cause: the intermediate compiled A
capture loaded and unloaded CUDA modules **120 times over six executions**,
with 33.359 ms in `cuModuleLoadData`. The pinned loader does not cache those
modules. The experimental runtime now retains its module, and blocked-run
caches its module by actual CUDA-context identity. These are code-object
lifetimes, not changes to Structures or upstream cuda-oxide. Those historical
executions still rebuilt every recorded run and dictionary.
The final compiled **both-plan** capture has 14 module loads across twelve
executions, including per-call generated query modules and startup owners,
versus 120 across six A executions before code-object retention. These are
whole-process counts, not falsely normalized timed-region allocation counts.

`compare/gpu_h05_compiled.ptxas.txt` and `.resources.json` contain the final
`ptxas -v -arch=sm_89` audit:

| Kernel | Registers | Stack bytes | Spill stores / loads bytes |
|---|---:|---:|---:|
| Compiled A `cq_gpu_stream_0_r21` | 48 | 0 | 0 / 0 |
| Compiled B `cq_gpu_stream_1_r19` | 40 | 24 | 28 / 20 |
| Hand-written `plan_a` | 36 | 0 | 0 / 0 |
| Hand-written `plan_b` | 34 | 0 | 0 / 0 |

The seven other generated kernels use 22–26 registers and zero stack/spills.
B's residual spills are reported explicitly; the zero-spill acceptance is
met for A, not claimed for B.

Additional acceptance commands:

```sh
cargo +stable test -p cq-gpu
cargo oxide build --arch sm_89 -- --release -p gpu-h05-compiled --bin printer-oracle
./target/release/printer-oracle
```

The compiler tests check exact keyed-read coverage and witness no-reprobe;
five public-macro compile-fail fixtures name `CQ_GPU_MERGE`,
`CQ_GPU_UNION`, `CQ_GPU_NON_INTEGRAL_KEY`, `CQ_GPU_OPAQUE_GUARD` and
`CQ_GPU_GENERATOR`.
The GPU oracle exercises full-width signed multiword dedup with strings,
and a 20,000-answer fanout using a recorded scan join filter, retained witness,
batch-32 expansion, frontier publication and output grow/relaunch.
The smoke additionally checks an unstaged 4,097-row fanout (including output
growth) and mutates its input in place before another execution, verifying that
pool/module reuse does not retain stale input data. The historical run passed
21 printer unit tests and all four compile-fail fixtures; rerun the commands
above for current resident-code verification.


### Supported domain and explicit rejections

- Typed copied-column dereferences and literal binds/heads are supported.
  Explicitly typed integral/bool binds whose expressions reference no query
  variable (for example `y:i64 = (meta.x)`) are evaluated once on the host
  and encoded as scalar kernel parameters, with singleton output bounds.
  Closure is checked using Physical's Rust variable analysis and the bind's
  written lexical environment; a field named like a query variable is not a
  dependency. Query-dependent computations such as `*x + meta.x` remain
  `CQ_GPU_OPAQUE_BIND`. Nonliteral string captures are not supported.
  The printer oracle checks full tuples, boolean/integral captures, and
  once-only evaluation in both bounded and resumable paths.
  Union/fallback blocks, merge, opaque Rust binds/guards and iterator generators
  are rejected. Smallest traversal is supported only for the explicit
  sorted-trie binary-intersection contract described above.
  Recorded eager anti reads and bridge-certified device negative membership
  use exact resident sets. The upstream generator form is rejected as
  `CQ_GPU_GENERATOR`, including unused definitions. `#[total]` is not
  permission to emit an arbitrary Rust function on device.
- Keys must be integral. Strings support dictionary equality, not lexical
  ordering or arbitrary string methods. Integer encoding is checked to fit
  signed 64 bits. A blocked rank domain cannot exceed 2^32 positions.
- Ordinary multi-level providers are rejected; only explicitly recorded
  `PrefixRun` has the sorted-trie interpretation above. A recorded CPU
  canon provider is rejected; `retype(dedup => DeviceSet<_>)` is required.
- Positional carriers, positional selection/read adapters and standalone keyset
  annotations are rejected. Multiple recorded
  run providers are selected by their exact served key prefixes. Recorded join
  filters support the exact `AdaptiveKeySet` bitmap
  provider and its budget; Bloom or arbitrary filter implementations are not
  silently substituted. Unknown provider aliases and non-Vec retained-row
  representations are rejected.
- No aggregates, host fallback algorithms, RCCL device algorithms, CUDA C or
  NVRTC. Recursion is provided by the separate checked GPU Datalog bridge
  described above, not by a CQ operator. Existing cuda-oxide PTX/shared-memory
  and package-loader restrictions remain.
- Finite device output cardinalities must fit the runtime's row-index domain.
  Without `output(retry)`, an explicit `capacity(Relation)` is a checked
  allocation contract and never establishes an unrecorded PK constraint.
  The default bounded-output path
  saves each lane's program position, live values and expansion cursors,
  shared descriptors and claimed frontier-task identity. A failed output
  reservation leaves the output count unchanged. The runtime reads one
  32-bit control word per attempt, doubles capacity, preserves completed
  rows with a device-to-device copy, and resumes only unfinished work.
  It does not count then replay the stream or recompute completed rows.
  Explicit `capacity(Relation)` without retry uses the direct bounded kernel
  without this per-attempt control readback. `output(retry)` instead performs an
  exact full candidate count and treats `capacity(Relation)` as an initial
  allocation hint. Fused overflow retries use distinct-based geometric growth
  for at most 12 attempts; non-fused retry may relaunch once at the candidate
  count. Immutable inputs and matching candidate counts are required; other
  errors do not retry. Its checked control/count readbacks remain required even
  with `completion(round)`.
  Tuple transfer remains explicit for every policy.

## Adaptive blocked-run ordering

`Run` records one ordering contract: lexicographic order over all recorded key
levels, with original row ID breaking ties. Algorithm selection is a runtime
device decision, not an IR fact. The first key still owns the dense rank
directory; the accepted domain and zero-filled inactive-output contracts are
unchanged.

The existing order check measures adjacent descents. For unsorted input only,
the histogram also measures observed minimum and maximum offsets within each
supplied domain; `word_offsets` selects the method after that kernel completes.
Sorted input therefore pays no range-statistics atomics. Unsigned subtraction
handles negative keys and domains beside either i64 endpoint. Warp reductions
amortize the statistics atomics.

| Device condition | Ordering |
|---|---|
| No descents | Existing sorted starts/directory/gather path; ordering kernels return immediately |
| At most 4096 active rows | One shared-memory radix block for packed keys; bitonic row-ID sort with the complete comparator for wider tuples |
| Sum of observed key widths plus row-ID width at most 64 | Packed u64 key/u32 row-ID stable eight-bit LSD radix |
| Wider tuple | Original first-key atomic partition and per-bin tuple heapsort fallback |

The radix implementation is local Rust CUDA using dynamic shared memory only.
It computes all digit histograms in one key read, scans their global bases,
then uses stable warp ranking, a shared-memory grouping exchange and
acquire/release decoupled lookback. Monotonic tile assignment and a bounded
persistent grid avoid waiting for an unassigned predecessor. Lookback state is
u64: a separate 32-bit status and full 32-bit row count. Row IDs are included in
the packed key, but their low bits are already ordered at input: stable passes
start above them, avoiding redundant provenance passes. Payloads are gathered
once, after ordering.

Selection performs **no host readback or synchronization**. Host-known capacity
and recorded domain bounds only limit scratch and possible pass submissions;
the device-selected bit count gates unused passes. A capacity at most 4096
requires no radix scratch or radix launches. Larger capacities add one gated
small-sort launch plus three gated radix setup launches and two launches per
possible digit (at most 20 additional launches). Sorted input keeps the
directory/gather work unchanged, but pays those explicitly gated launches.
No few-run merge path is selected.

The reusable launch sequence is:
`initialize → sorted_check/prefix_sorted_check → clear_histogram →
sorted_starts → histogram → word_offsets → row scan → bitmap-prefix scan →
complete_stats → scatter(pack or fallback) → small_sort →
[radix_clear_histogram → radix_histogram → radix_prefix_bins →
(radix_init_pass → radix_scatter)*] → order_prefix_bins(directory plus
non-packable fallback only) → sorted_directory → gather → fences →
payload gathers`.

The CPU oracle checks complete outputs, row-ID stability, inactive tails,
active device counts, small-sort boundaries, multi-tile radix, Zipf and extreme
skew, deep prefixes, non-packable tuples, and wide metadata domains with narrow
observed values. Benchmark checksums include every public output buffer, so the
same harness can compare the original and adaptive builders exactly.

```sh
export CUDA_HOME=/opt/nvidia/hpc_sdk/Linux_x86_64/25.5/cuda/12.9
cargo +nightly-2026-08-28 test -p cq-gpu -p cq-gpu-datalog -p gpu-blocked-run --tests
cargo +nightly-2026-08-28 oxide build --arch sm_89 -- --release \
  -p gpu-h05 --bin blocked-run-oracle --bin sort-bench
target/release/blocked-run-oracle
target/release/sort-bench --n 31,4096,1000000,10000000,100000000 --levels 1
nsys profile --force-overwrite=true -o /tmp/blocked-sort \
  target/release/sort-bench --case zipf1.5 --n 100000000 --levels 1 --repeat 1
nsys stats --report cuda_gpu_kern_sum,cuda_api_sum /tmp/blocked-sort.nsys-rep
```

The optional `radix-probe` binary loads RCCL's own `cuda::Program` beside the
blocked-run module and invokes selected-bit u32/u32 and u64/u32 pair sorts on
one non-default stream, using the same pool for all pair and control storage.
It is registered in `gpu-h05`, not in `gpu-blocked-run`: cuda-oxide's embedded
package names are not unique between a library and its binary, so putting
RCCL-generated binary PTX under the library's package name makes the blocked-run
loader select the wrong bundle (`DriverError(500, "named symbol not found")`).
RCCL is an optional dependency confined to the probe feature.

```sh
cargo +nightly-2026-08-28 oxide build --arch sm_89 -- --release \
  -p gpu-h05 --features rccl-probe --bin radix-probe
target/release/radix-probe 1000000 10000000 100000000
```

### Ordering measurements (2026-10-19)

RTX 6000 Ada, sm_89, pinned nightly-2026-08-28. All timed runs started with an
empty `nvidia-smi --query-compute-apps=pid` result. The original builder is
`d6527a3`'s `blocked-run/src/lib.rs`, compiled in an isolated temporary package
against the identical benchmark source. Inputs use seed `81985529216486895`,
domain 1,000,000 and one original-row-ID payload. Generation, uploads,
readbacks and checksums are outside CUDA-event timing. Old figures are one
sample after one warmup; new figures are medians of three after one warmup.
These are complete index builds, not just radix kernels.

| Case | Rows | Old ms | New ms | Speedup |
|---|---:|---:|---:|---:|
| sorted | 31 | 0.053280 | 0.056576 | 0.94x |
| reverse | 31 | 0.057216 | 0.069792 | 0.82x |
| uniform | 31 | 0.056512 | 0.068256 | 0.83x |
| extreme-skew | 31 | 0.051328 | 0.048896 | 1.05x |
| Zipf 1.0 | 31 | 0.057440 | 0.069696 | 0.82x |
| Zipf 1.5 | 31 | 0.058720 | 0.068256 | 0.86x |
| sorted | 4,096 | 0.055232 | 0.056096 | 0.98x |
| reverse | 4,096 | 0.059008 | 0.080704 | 0.73x |
| uniform | 4,096 | 0.060768 | 0.080768 | 0.75x |
| extreme-skew | 4,096 | 0.046304 | 0.050336 | 0.92x |
| Zipf 1.0 | 4,096 | 0.171584 | 0.081152 | 2.11x |
| Zipf 1.5 | 4,096 | 0.888192 | 0.076320 | 11.64x |
| sorted | 1,000,000 | 0.091872 | 0.109120 | 0.84x |
| reverse | 1,000,000 | 0.146400 | 0.303424 | 0.48x |
| uniform | 1,000,000 | 0.224384 | 0.332256 | 0.68x |
| extreme-skew | 1,000,000 | 1264.332886 | 0.221792 | 5700.53x |
| Zipf 1.0 | 1,000,000 | 70.000290 | 0.370592 | 188.89x |
| Zipf 1.5 | 1,000,000 | 492.642731 | 0.339808 | 1449.77x |
| sorted | 10,000,000 | 0.605920 | 0.652128 | 0.93x |
| reverse | 10,000,000 | 2.045792 | 2.908928 | 0.70x |
| uniform | 10,000,000 | 3.292832 | 3.823520 | 0.86x |
| extreme-skew | 10,000,000 | 14743.226562 | 2.260704 | 6521.52x |
| Zipf 1.0 | 10,000,000 | 900.171570 | 4.287840 | 209.94x |
| Zipf 1.5 | 10,000,000 | 5846.967773 | 3.906528 | 1496.72x |
| sorted | 100,000,000 | 7.211296 | 7.246624 | 1.00x |
| reverse | 100,000,000 | 42.613663 | 27.269695 | 1.56x |
| uniform | 100,000,000 | 83.977409 | 57.784832 | 1.45x |
| extreme-skew | 100,000,000 | 171837.812500 | 22.606239 | 7601.34x |
| Zipf 1.0 | 100,000,000 | 10725.708008 | 60.120415 | 178.40x |
| Zipf 1.5 | 100,000,000 | 69393.437500 | 44.164223 | 1571.26x |

The new complete-build throughput at 1M / 10M / 100M rows is respectively
3.010 / 2.615 / 1.731 G rows/s for uniform input,
2.698 / 2.332 / 1.663 for Zipf 1.0, and
2.943 / 2.560 / 2.264 for Zipf 1.5. At 100M, extreme skew reaches
4.424 G rows/s. Uniform/reverse small and medium inputs remain regressions;
the replacement eliminates the skew-serialized tail, not every overhead.
The 31/4096-row extreme-skew samples happen to contain only one repeated key
and therefore take the sorted skip.

Prefix builds at 1M rows, one sample after one warmup:

| Case | Levels | Old ms | New ms |
|---|---:|---:|---:|
| sorted | 2 | 0.095744 | 0.122816 |
| sorted | 3 | 1.117312 | 1.147712 |
| reverse | 2 | 0.155712 | 0.426080 |
| reverse | 3 | 1.183680 | 1.298080 |
| uniform | 2 | 0.243936 | 0.472928 |
| uniform | 3 | 1.251040 | 1.390528 |
| extreme-skew | 2 | 3645.431396 | 0.323872 |
| extreme-skew | 3 | 6361.488281 | 6367.875488 |
| Zipf 1.0 | 2 | 272.803406 | 0.437312 |
| Zipf 1.0 | 3 | 342.412354 | 342.472290 |
| Zipf 1.5 | 2 | 1647.500854 | 0.402656 |
| Zipf 1.5 | 3 | 2664.909424 | 2667.197998 |

Two 20-bit levels plus row ID fit u64 and need five key-field radix passes.
Three such levels plus row ID do not fit: they intentionally retain the old
fallback and its skew cost. This is not a claim of skew-insensitivity for
non-packable keys.

The oracle passed **104 exact cases**, including non-packable tuples and
observed ranges narrower than metadata bounds. All 90 single-key measured
samples and all 12 prefix samples had the same complete-output checksum as
the original builder. Data and oracle output are in
`compare/adaptive-sort-{baseline,new,prefix-baseline,prefix-new}.csv` and
`compare/adaptive-sort-oracle.txt`.

### 100M skew profile and sorted launch overhead

Nsight Systems 2025.3.1, 100M extreme-skew rows, one build without warmup:
22.604383 ms. The profile contains 29 kernels:

| Kernel/stage | Launches | Total ms |
|---|---:|---:|
| `radix_scatter` | 3 | 9.670769 |
| key and payload `gather` | 2 | 4.905129 |
| `histogram` including observed ranges | 1 | 3.269500 |
| `scatter` packed-key generation | 1 | 2.434421 |
| all-pass `radix_histogram` | 1 | 1.000840 |
| `sorted_check` | 1 | 0.899111 |
| `radix_init_pass` | 3 | 0.256515 |
| directory/scans/fences and gated stages | 17 | 0.059233 |

The three u64/u32 scatter passes require at least
`100M * 12 bytes * 2 * 3 = 7.2 GB` of pair traffic. Their measured 9.670769 ms
corresponds to **744.5 GB/s**, or **77.6%** of the approximately 960 GB/s
bandwidth bound (ideal 7.5 ms). This is an effective minimum-traffic rate,
not a hardware DRAM-counter measurement; it excludes lookback traffic, packing
and payload gathering. The complete build is 4.424 G rows/s.

For this one-level 20-bit domain, capacities above 4096 enqueue three possible
passes: 19 original directory/build launches plus **10** adaptive launches.
Sorted 100M input executes those ten as no-ops, totaling **13.088 microseconds**
of GPU time in its separate profile. Complete sorted build time is
7.246624 ms versus 7.211296 ms originally, a measured 35.328-microsecond
difference. Capacities at most 4096 add only one gated small-sort launch.
Two-level packable benchmark inputs enqueue five possible passes (33 total
launches); the three-level non-packable case enqueues eight gated passes
(39 total) before its existing fallback.

Between the first and last build-kernel submissions, the 100M trace contains
29 `cuLaunchKernel`, 4 `cuEventRecord`, 3 asynchronous memsets,
14 `cuStreamWaitEvent`, 24 `cuStreamGetId`, and one cold-pool `cuMemAlloc`.
It contains **no DtoH copy or stream/context/event synchronization** in that
submission interval. Completion waits and output downloads belong to the
benchmark outside it. Raw kernel/API summaries are
`compare/adaptive-sort-{100m,sorted-100m}-profile.csv`; the full reports were
captured as `/tmp/adaptive-sort-{100m,sorted-100m}.nsys-rep`.

### RCCL probe outcome and isolated repair

At pinned RCCL `745f53c`, loading `rccl_cub::cuda::Program` alongside blocked-run
fails with `DriverError(218, "a PTX JIT compilation failed")`. The public
generic pair-sort API still emits its specializations into the consumer's
PTX: the probe declares `__shared_mem_0[4096]` while RCCL's fixed u32 scan
declares `__shared_mem_0[8192]`. `ptxas` reports both inconsistent redefinition
and duplicate definition. Exact evidence is in
`compare/adaptive-sort-rccl-collision.txt`.

The contained scan repair was committed, without pushing, as **`6e30ca6`** on
`/data/rccl` branch **`mini-linq/dynamic-lookback-scan`**. It replaces that scan's
static allocation with an 8192-byte dynamic allocation and matching launcher
contract. RCCL's 94 host tests passed, as did its GPU inclusive/exclusive scan
probe expanded around the 1920-item tile boundary.

That repair removes the shared-global collision but does not make the combined
program load: RCCL's generic loader merges *all* embedded bundles, exposing
a second error, **duplicate function `scan`**, defined by both `gpu_tiles.ptx`
and `gpu_blocked_run.ptx`. The experimental local workspace override has been
removed; this workspace retains the pinned RCCL revision.

Future RCCL work could add an explicit
`Program::load_with_modules(context, &["rccl-cub", "gpu-h05"])` that binds its
internal modules to only the selected RCCL/consumer-specialization image.
That API was not implemented. The optional reference experiment was stopped
at this second loader failure, so **no RCCL 1M/10M/100M throughput numbers or
executed stream/synchronization claims are reported**. Its source dispatcher
uses the supplied stream without host size readback, but that is not a
substitute for a successful execution trace.

The local device-gated radix remains the production implementation: it is
exact, stream-ordered without selector readback, and its skewed scatter
already reaches 77.6% of the minimum-traffic bandwidth bound. No CUDA-oxide
source was changed, and no compiled-query binary was run.

**Resolution (2026-10-20).** Both collisions are fixed by two local patches,
applied through `[patch]` in this workspace's `Cargo.toml`:

- RCCL `6e30ca6` (`/data/rccl`, branch `mini-linq/dynamic-lookback-scan`):
  the u32 lookback scan uses dynamic shared memory, removing the static
  `__shared_mem_0`.
- cuda-oxide `4f29edb0` (`/data/cuda-oxide`, branch `mini-linq/merge-dedupe`
  on `5f7c34f`): `load_all_ptx_bundles_merged` drops a later bundle's
  definition when its code matches the first (trivia removed, body-local
  labels renumbered) and rejects same-named definitions that differ. A
  cross-crate `#[device]` function is emitted into every bundle whose kernels
  reach it; `h05-reference` reproduced this with device-columns `unpack` and
  `transposed_position` (CUDA error 218, ptxas "Duplicate definition of
  function").

With both, `h05-reference` loads RCCL's merged program and runs its index
build and dedup on RCCL (see the general H05 reference above). Neither patch is
pushed upstream.
