# Mac implementation: provider, maintained runs, then backend

Status: this tree has MCCL/MLX shared-source construction, immutable CPU reads,
and online flat/binary-carry snapshots for its existing packed u32/u32 provider.
It also has an explicitly selected **GPU-built grouped RadixRun**, including
parallel spline construction and stable grouped merge, using the native CPU
reader. See [GPU-built RadixRun](#gpu-built-radixrun-2026-10-27) and the
[shared-relation input cutover](#shared-relation-input-cutover) for the current
APIs. Wider GPU index keys, Datalog publication integration, automatic placement
and a Metal query backend remain separate work. Earlier measurement sections
and command blocks retain their historical boundaries.

## Decision

Start with GPU-built, CPU-readable immutable indexes and an explicitly
host-owned append-only maintained-run provider. Preserve ordinary CQ and the
existing Datalog round semantics. Keep both **flat compaction** and **LSM-like
binary-carry runs** as selectable maintenance policies over the same run format.
Do not make a new logical join algorithm or a GPU-resident fixpoint scheduler a
prerequisite for those providers.

A data-structure swap is sufficient only for an immutable read-compatible
provider, or for externally maintained snapshots passed into an ordinary query.
It is not sufficient for asynchronous publication, multi-run retained ranges,
Datalog's generated layout updates, or a Metal-resident execution backend.

## Integrated state and compiler-change inventory

The GPU experiment is at [experiments/mac/gpu-index](../experiments/mac/gpu-index/README.md),
alongside the main tree's optimized native `RadixRun`. Its runtime and canonical
native implementation are consolidated in `experiments/mac/cq-mac` and
`experiments/mac/mccl`, separate from the portable core workspace. The native
cached numeric-key sort, lazy exact dense-membership bitmap, H09 workload order,
and profiling changes remain intact. The earlier sibling snapshots are historical
backups, not build dependencies or active work directories. Archived timings
retain their original source/machine provenance; integration is not a new result.

The current tree includes the following compiler/runtime boundary. The subsequent
provider-contract refactor replaces the earlier RadixRun name-specific checks:

| Layer | Existing change |
|---|---|
| `languages/cq-lang-structures/src/{lib,share,stage,join_filter,repr}.rs` | Check declared `supports [retain, expand, keys]` on the selected layout, with no provider-name admission or blacklist. |
| `passes/cq-sequence-pull/src/lib.rs` and Sequence catalogs | Target-owned registrations name implementations and their read/support contracts; selection and replay preserve live obligations. |
| `features/rewrites/passes/src/structures.rs` and lookup advice | Provider selection records support; generic retention/expansion/filter advice consumes it. Bare retyping clears stale promises; explicit retyping can replace them. |
| `runtime/cq-storage/src/{radix_run,radix_spline,sorted_run,sequence}.rs` | Native provider, grouping, membership cache and range adapter. This is runtime work. |
| `doc/formal/{ir_structures,backend}.tex`, `rules.tsv` | Existing provider/range obligations and rule inventory. |

The latest join tuning is provider/runtime optimization plus profiling plumbing;
it does not add CQ/VO/Physical operators. The recent upstream merge
`2698fc8f` separately brings H19 string interning, loader support, workload
changes and CUDA experiment advice/results; it does not add Mac execution or
an LSM lifecycle. The initial experiment import changed no compiler code; the
provider-contract refactor described here is a separate compiler change.

## Boundaries: how much compiler work?

| Target | Runtime work | Compiler work |
|---|---|---|
| GPU build, synchronous CPU reads of one immutable run | Metal buffer ownership, encoding, stable build, completion before exposure, lawful cursor | No new CQ/VO/Physical semantics. Declare actual provider support and register target reads; generic checks no longer need a new provider-name case. |
| External append-only LSM owner, ordinary CPU queries over frozen snapshots | Append/carry/manifest publication, all-run lookup, safe retirement | Can remain outside the core compiler. Expose snapshots through existing exact-read traits; do not claim a single contiguous Range capability. |
| Maintained Datalog indexes with CPU-controlled rounds | Batch admitted rows into full layouts; rebuild current delta layouts; publish complete snapshots | Feature-local Datalog storage/printer realization is needed. Reuse checked Round order rather than changing logical semantics. |
| Ordinary Metal-resident Sequence execution | Metal build/probe/expand/canon/output primitives and ownership/completion protocol | A real Metal machine/catalog/translator is needed; the active GPU translator emits CUDA. Existing finite-query Sequence vocabulary should be reused where it expresses the chosen execution. |
| Compiler-scheduled persistent GPU maintenance / asynchronous cross-query state | Explicit dependency, snapshot and retirement machinery | Requires an explicit checked lifecycle/scheduling design. Current Sequence has no append/compact/snapshot/round term; this is not provider substitution. Bring proposed core changes back for review before implementation. |

No new core language or logical planner change is required for the recommended
first two targets. Provider registration and capability-check changes still count
as compiler changes and should be reported separately, even when small.

Backend-private asynchronous command encoding does not by itself require new
core syntax: it may remain private while satisfying existing completion and
publication boundaries. Compiler-chosen build placement, transfers, cross-queue
overlap and persistent maintenance are different. `Catalog` provider levels
describe **consumption, not construction**; neither a provider token nor machine
grading establishes buffer readiness. Mac machine/layout and catalog adapters
can use the existing external `decompose`, `grade` and `print` pass slots.

### Implemented general provider boundary

An independently authored provider or alias can now record:

```text
index = MyProvider<_, _> [key] supports [expand, keys];
```

`retain` requires an exact immutable Copy lookup cursor. `expand` additionally
requires `ExactSizeIterator` and implies retain; it does not imply contiguous
Sequence Range or Ordered. `keys` uses the existing `Keys` trait. Native emission
proves declared support with Rust bounds, even without an optimization consumer.
It retains structural checks for coverage, selected payload versus candidate
layout, eager sharing, partiality, snapshot identity and witness escape.
Column markers likewise use a general `ColumnSlot<Host, N>` contract rather
than recognition of `ColumnAt` spellings. A marker alias selecting another slot
is rejected even when both columns have the same key type. Codec/layout checks
use the provider's Rust bounds rather than an AdaptiveKeySet name exception.

Selection/retyping and replay carry support with the provider. An unused default
host fallback does not impose Hash or other read bounds on a valid native host
path; actual dispatch proves the traits of the chosen handle. Unsupported
target shapes fail explicitly, including retained/expanded Smallest on native
realization and Structures staging/witness/filter decisions on the CUDA
Sequence target. This is not a new GPU or LSM implementation. The approved
provider-contract refactor and the subsequent GPU runtime are consolidated here.

## Existing contracts to reuse

- [`Build` and `IndexRead`](../runtime/cq-storage/src/hash_index.rs) already state
  the immutable construction/exact-occurrence contract. `Build::build` is
  synchronous; `IndexRead` returns an iterator. Neither owns append, epochs,
  compaction, command completion, or publication.
  It also has no context argument or `Result` return. Prefer explicit fallible
  preparation with a caller-owned Mac context and pass the completed snapshot
  into ordinary queries. A compiled-build adapter needs an explicit context and
  failure policy; do not silently hide GPU setup failures or change `Build`
  globally to accommodate the first Mac provider.
  Recorded native host reads may additionally require `PointLookup`/`Rows`
  adapters and complete-row/key lifetimes; payload-only `IndexRead` is not enough
  to promise every host-access mode.
- [`RangeCursor`](../runtime/cq-storage/src/sequence.rs) requires one slice
  `items()` and one contiguous `range(key)` into that slice. A key occurring in
  several LSM runs does **not** satisfy that contract. Advertising Range merely
  because each individual run has a range would be incorrect.
- [`incremental.tex`](formal/incremental.tex), a **design specification**, already
  states lawful build/merge/read, generic runs, exclusive cuts, publication,
  set admission and maintained device-run laws. Use those obligations; do not
  invent Differential Dataflow timestamps, subtraction or retraction semantics.
- [`features/datalog/print`](../features/datalog/print/src/lib.rs) has
  `render_with_publication` for alternative checked publication realizations.
  Current initialization/layout emission and `full_insert`/`delta_rebuild`
  still use collection-style insertion; a new provider is not a type-only
  change to that generated code.
- [`GPU-DESIGN.md`](GPU-DESIGN.md) records removal of the former maintained-index
  backend. Historical provider-name exceptions have now also been removed from
  Structures; they grant neither capabilities nor a maintenance protocol.

## Runtime design

### One immutable run

A run owns its order-preserving encoded keys, occurrence/payload positions,
exact lookup directory, source-payload owner, and chronological batch interval.
Published buffers never mutate. GPU work writes new buffers and completes
successfully before any CPU-readable snapshot exposes them.

Reuse the measured four-pass 8-bit stable sort and parallel-partitioned merge.
A 32-bit position must remain tied to a stable source occurrence, not to a
movable temporary allocation. Sorting payload positions does not grant the GPU
permission to dereference arbitrary Rust references or copy arbitrary Rust
layouts. For borrowed/wide payloads, retain a typed host owner and resolve
positions on CPU, or explicitly build a lawful packed representation.

The experiment supports u32 keys/positions only. Native RadixRun also accepts
full-width signed/unsigned 64-bit and borrowed keys; those must not be narrowed.
An initial provider may explicitly advertise a smaller supported domain, but a
production replacement requires the intended full key domain, signed monotone
encodings, row-count limits, and payload-layout validation. Unsupported domains
must remain visibly unselected by advice, not be silently truncated.

Keep GPU exact radix-prefix and native CPU RadixSpline directories distinct.
The former avoids sequential spline construction but has unmeasured probe costs
and skewed buckets on dense key domains. Choose using matched build-plus-read
workloads, including the native membership-only optimization, not build time alone.

### Maintained owner and immutable snapshots

These are proposed responsibilities, not a fixed public Rust API:

- **Writer:** owns publication state, pending batches and the chosen compaction
  policy. A batch is admitted and assigned its chronological interval once.
- **Pending update:** owns unpublished runs, directories and command-use lifetimes.
- **Snapshot:** owns one immutable manifest containing an epoch and all visible
  runs/payload owners. A query pins one snapshot; subsequent reads cannot follow
  a mutable latest-version pointer.
- **Cursor:** borrows the query's pinned snapshot, or owns the necessary snapshot
  references. Prefer borrowing a once-pinned snapshot over per-row/per-probe
  reference counting. Buffer reuse waits for both snapshot/cursor lifetimes and
  in-flight GPU readers.

Prepare the delta, sort it, perform selected carries and build directories in
unpublished storage. Publish the complete new manifest only after successful
GPU completion. Failed completion leaves the previous manifest visible; no
mixed run/directory pair may become readable. Multiple indexes of a relation,
and every relation required by one Datalog evaluation, must publish a
version-consistent set before that evaluation starts.

Retained snapshots may keep old allocations alive while a newer manifest
compacts across their historical boundaries. This is persistent snapshot
retention, not a promise to reconstruct arbitrary cuts inside an already merged
interval. Reclamation is part of the implementation cost and acceptance gate.

### Maintenance policies

1. **Flat:** stable sort of the accepted delta, merge with the existing run,
   rebuild the directory, publish one run. It minimizes read amplification but
   rewrites the accumulated base on each publication.
2. **Binary carries / LSM-like:** publish immutable sorted batches; when a level
   is occupied, merge chronologically adjacent older/newer runs into the next
   level. Keep at most one run per level. Initially levels represent batch-count
   rank, not an asserted row count: variable-sized batches do not have equal
   physical sizes. Size-tiered thresholds are a separate measured policy.

Both policies preserve the same abstract occurrence bag. LSM-like here means
in-memory append-only sorted runs and compaction, not disk persistence, WAL,
last-write-wins keys, tombstones or deletes.

Lookup must examine **every occupied run** and concatenate exact key matches in
source chronology, not numerical level order. Equal keys and identical payload
occurrences remain present. Merge takes the older run first on equal keys;
payload values are never tie-breakers. Membership is OR across exact per-run
membership, while counting sums matching occurrences.

Start with Exact/Membership and ordinary iterator cursors. Do not flatten all
runs on every query to manufacture Range support. A snapshot-bound multi-run
cursor may declare `retain`/`expand` if it truly satisfies the immutable
Copy/exact-size contracts; implement efficient positioning rather than repeated
full scans. This still does not satisfy the distinct contiguous-array
`RangeCursor` contract. Device expansion of multiple segments remains target
runtime work, not a reason to add an LSM type-name exception to core checks.

## Datalog integration: preserve the existing round

Current production ordering is:

```mermaid
flowchart LR
    E["Evaluate immutable All_t / Delta_t"] --> A["Admit fresh full rows"]
    A --> F["Append to all full layouts"]
    F --> D["Replace Delta; rebuild delta layouts"]
    D --> P["Complete and publish next snapshot set"]
    P --> N["Next evaluation"]
```

Use existing full-row `seen` admission first: candidates are self-deduplicated
and checked against accumulated facts before maintained layouts receive them.
The index itself still preserves occurrences. Equal projected keys with different
payloads are not duplicate facts. Ordinary CQ bag indexes must never inherit
Datalog's set admission accidentally.

All readable full layouts already include the currently published delta. Keep
that invariant. A different frontier scheme that probes a delta against an older
full index and only then absorbs that same delta can omit delta–delta joins in
nonlinear recursion. Such a scheme needs a checked coverage argument and is not
a representation-only optimization.

The feature-local integration should batch admitted rows once per publication,
complete all requested full-layout updates, replace/rebuild delta layouts even
when empty, and only then enable the next evaluation. Initialization and EDB
canonicalization must use the same lawful provider path. Existing row-by-row
`.entry(...).or_default().push(...)` emission cannot express the required GPU
batch finalization without a publication realization change.

## Implementation order and gates

1. **Lawful immutable Mac provider.** Isolate Metal in a Mac-specific runtime
   outside the default portable dependency path. Implement complete build and
   CPU lookup for the explicitly selected key/payload domain; pin ownership and
   complete GPU work before returning. Check exact outputs against native hash
   and RadixRun. Measure allocation, encoding, synchronization, payload gather,
   build, membership and expansion separately, then end-to-end.
2. **Online flat and binary-carry owner.** Query every intermediate manifest,
   especially 3/5/6/7 batches, empty/unequal batches and cross-run duplicate keys.
   Hold partially consumed cursors across append/compaction and writer disposal;
   the old snapshot must remain exact while the new one sees appended rows.
   Validate completion/error publication and retirement. The offline eight-batch
   experiment does not satisfy this gate.
3. **Feature-local Datalog realization.** Preserve current Round checks and set
   admission. Test seeds, mutually recursive relations, nonlinear delta–delta
   cases, overlapping semi-naive variants, duplicate EDB/candidate facts, empty
   delta replacement and frozen lower-stratum negation against the existing
   evaluator. Ensure each full/delta layout is maintained once per publication,
   with no hidden per-hole full rebuild.
4. **Read-policy and cost selection.** Compare flat versus binary carries using
   build-plus-probe workloads and memory high-water marks. Include all-level
   probe cost, compaction amplification, pending/retained generations, batching
   latency and CPU/GPU shared-memory contention. Do not import M5 coefficients
   into this M3 experiment. Keep small-update CPU versus GPU placement explicit;
   no crossover threshold has yet been established by end-to-end query tests.
5. **Metal-resident query backend, then lifecycle scheduling if justified.**
   Implement and grade actual Metal primitives; reuse existing finite-query
   Sequence obligations. A future persistent/async maintenance schedule must
   explicitly represent its dependencies and lifetimes and obtain separate
   core-design review. Do not resuscitate the removed GPU Datalog bridge.

## Integration verification

After importing the experiment into the current join-optimized tree:

- `cargo test -p cq-storage -p cq-lang-structures -p cq-sequence-pull --all-features`:
  149 tests passed.
- `cargo run --manifest-path features/rewrites/Cargo.toml --example radix_runs`:
  hash, immediate RadixRun, retained, staged32 and retained+staged32 each produced
  150 identical exact answers.
- `cargo run --release --locked --manifest-path experiments/mac/gpu-index/Cargo.toml -- --check-only`:
  actual Metal stable sort/merge, grouping, directory and exact-range checks passed.
- The integrated `--updates-only --sizes 65539 --runs 1` smoke also exercised
  irregular-size updates and both offline maintenance histories, with batched
  and separately completed submissions. Output:
  `experiments/mac/gpu-index/target/integration-smoke.json`. This validates the
  imported experiment, not the proposed online snapshot/LSM owner.

### Provider-contract refactor verification

- Standard workspace gate: 958 tests passed.
- Rewrite workspace: 187 tests passed, including custom/aliased providers,
  retained native/fallback host cursors, unit-key occurrence order, and
  compilation rejection of false cursor/key/codec/marker contracts.
- Sequence rewrite/replay workspace: 99 tests passed.
- External push target: 237 tests passed; join-filter workspace: 7 tests passed.
- The five-route native join executable still returns 150 exact answers per route.
- All 27 public API snapshots were updated and checked; the formal inventory
  check passes for 245 rules.

Broader gate limits are recorded rather than hidden: `--all-features` enables
the untouched optional HW4 aggregate exercises, whose two unimplemented cases
fail while the other workspace targets pass. The dependency checker reports
version drift in the Datalog, own-dialect and Sequence-rewrite lockfiles;
those files and the root lockfile match the untouched snapshot byte-for-byte.
The full CUDA workspace still requires missing machine-local
`/data/cuda-oxide` patches. Its actual compiler sources passed 15 host-only
unit checks through a temporary isolated manifest; CUDA device execution was
not exercised. No such prerequisite was bypassed by changing runtime behavior.

The provider-contract refactor and subsequent GPU runtime work are now integrated
in this tree. The earlier `mini_linq-gpu-index-20261024` snapshot remains historical;
the verification counts above describe their original source states, not this merge.

## Implemented hybrid runtime

[`experiments/mac/cq-mac`](../experiments/mac/cq-mac/src/lib.rs) is a standalone Cargo workspace,
not a dependency of the portable workspace. It adds no compiler pass, provider
name exception, Datalog emission change or implicit `Build` implementation.

- `MacContext::new()` fallibly creates an MCCL context. Index construction
  validates unified memory, capacity and compiled pipeline geometry; unsupported
  platforms fail explicitly without a CPU fallback.
- `relation_u32`/`relation_i64` explicitly ingest numeric rows into immutable
  shared storage once; no index is built by ingestion.
- `build(&Relation, IndexSpec, selection)` runs GPU projection, optional stable
  mask compaction, original row-ID generation, four-pass radix sort and exact
  radix-12 directory construction. It performs no host relation scan, packing
  or source copy, and waits before publishing the immutable result.
- `build_on_cpu(&Relation, IndexSpec, selection)` is an explicit stable CPU
  comparison-sort baseline over the same source. It writes its own CPU-built
  result; lazy source dependencies may first need evaluation for CPU access.
  It is not a fallback. All builders publish the same reader representation.
- `build_on_cpu_radix` is an explicit single-threaded byte-radix candidate.
  It detects varying key bytes during CPU projection, omits constant-byte histograms
  and passes, and buffers one 128-byte line per output bucket before scattering.
  Full-width keys and stable equal-key order remain unchanged.
- MLX owns inactive-buffer recycling and in-flight scratch. There is no separate
  `MacWorkspace`, allocation-policy enum, command queue or profiled raw-Metal
  builder. The macOS `cq_mac::memory` module exposes MCCL's process-wide cache
  and wired-budget controls; they are not per-index advice. No wired budget is
  enabled implicitly, and live arrays are never recycled by cache clearing.
- `merge(older, newer)` uses stable partitioned GPU merge; equal keys take
  older occurrences first. Published inputs are never modified. Both inputs
  must belong to the same MCCL context, not merely the same physical GPU.
  Nonempty maintained appends enforce this even when no carry would occur.
- CPU lookup reads those completed Metal allocations directly. It does not
  gather a second host copy or submit a GPU command. `&MacRun` and `&Snapshot`
  implement `IndexRead`; both owned types implement `ExactMembership`. Borrowed
  cursors are Copy, exact-size and fused; probes allocate no heap storage and
  do not increment reference counts. Snapshots do not claim contiguous Range.
- `MaintainedIndex::{new, append, snapshot}` supports `Flat` and `BinaryCarry`;
  append takes an already-ingested relation and an `IndexSpec`.
  Every nonempty append advances one epoch; an empty append is a no-op.
  Binary levels count batches, not rows, and reads visit every run in source
  chronology. Every selected build/merge completes before publication; an
  error drops unpublished work and preserves the previous manifest.
- Runs and manifests use `Rc`: pinned snapshots survive compaction and disposal
  of the writer/context, but this API is intentionally thread-confined, not a
  cross-thread publication service. Reclamation follows the last owning pin.

The index domain remains full-width **u32 keys and u32 payload values**. Shared
source relations may retain full-width i64 columns; GPU projection validates
only selected index fields and generates original source positions for `RowId`.
Total source rows are capped at `1 << 30`, with device limits also enforced.
This does not make arbitrary Rust objects GPU-readable, silently narrow wide
keys, or replace the broader native `RadixRun` domain.

Canonical shaders live in `experiments/mac/mccl/native/index` within this tree.
The offline experiment consumes those sources but retains its explicit raw-Metal
measurement harness. Production CQ code contains no Metal allocator, queue,
shader copy or CPU radix implementation; MCCL owns these construction primitives.
GPU sort/merge/directory dependencies are encoded through MLX's hazard handling
and evaluated before publication. CPU probes read completed storage directly.

### Run and verification

From the repository root, build the local MCCL native bundle first:

```sh
cmake -S experiments/mac/mccl -B experiments/mac/mccl/build/native -DCMAKE_BUILD_TYPE=Release
cmake --build experiments/mac/mccl/build/native --parallel 8
cargo test --locked --manifest-path experiments/mac/cq-mac/Cargo.toml
cargo run --release --locked --manifest-path experiments/mac/cq-mac/Cargo.toml --example hybrid -- 65539
cargo run --release --locked --manifest-path experiments/mac/gpu-index/Cargo.toml -- --check-only
```

Before the MCCL cutover, on the Apple M3 Max (40 GPU cores), six runtime tests
passed. The following results describe that raw-Metal revision, not a remeasurement:
Execution covered empty/irregular tiles, u32 extrema, repeated identical pairs,
stable merge order, and exact native HashIndex/RadixRun comparisons. Online
histories checked all eight intermediate manifests, including 3/5/6/7 unequal
batches, empty appends, exact cursor lengths and old cursors after compaction
and writer/context disposal. A private failure-injection test completed one
real carry, failed before the second, verified unchanged publication, retried
exactly, and checked old-manifest retirement through a weak owner. This checks
failure-atomic orchestration, not an induced hardware command-buffer fault.

The release `hybrid` executable also passed: seven unequal batches reached
458,422 occurrences under each policy, with one flat run versus three binary
runs at epoch seven. Every selected key matched the native reference at every
publication, and retained cursors stayed exact. The shared-kernel experiment's
stable-row, directory, grouping and lookup checks passed unchanged.

These are correctness/smoke results, not a CPU/GPU crossover or saturation
claim. The smoke prints host timings, but it was not a controlled benchmark
(the kernel check ran concurrently). Phase-separated allocation/encoding/wait
costs, shared-memory high-water marks, large retained generations, cross-thread
publication, full payload/key domains, and Datalog integration remain unmeasured
or unimplemented as appropriate. The pinned `metal`/`objc` stack also emits
legacy `cargo-clippy` cfg warnings and a `block` future-compatibility warning;
no warning suppression or CPU fallback was added.

## Query-first manual expansion experiment

[`experiments/mac/query-expansion`](../experiments/mac/query-expansion/src/lib.rs)
includes the existing authored H03 and H09 query source files, retaining their
default positional-host pipeline, column types and predicates. Its lightweight
shell omits benchmark loaders and alternative configurations. `cargo expand`
was run and the generated builds, filters, host dispatch and join loops inspected
before implementing the handwritten Mac paths. These are the positive retail
join cores, not full canonical TPC-H Q3/Q9 aggregates.

```sh
cargo expand --manifest-path experiments/mac/query-expansion/Cargo.toml --lib
cargo run --release --locked --manifest-path experiments/mac/query-expansion/Cargo.toml -- 65539
```

### Observed execution boundaries

| Query | Expanded execution | Handwritten Mac realization |
|---|---|---|
| H03 | Filter Customer by `BUILDING`; build customer-key occurrences; filter Orders by date and Customer membership; build order-key to customer payload; derive the optional order-key join filter; scan date-filtered Lineitem, probe Orders then Customer, canon. | CPU strings, dates, Customer index and the same optional adaptive join filter. GPU-build only the selected Orders as `(order_key, original Orders rowID)`; CPU gathers the original full-width customer key and completes the joins. |
| H09 | Scan Part and apply `contains("green")`; lazily build Lineitem by part key; probe PartSupp by its composite key, then Orders, Supplier and Nation; canon. | GPU-build only Lineitem as `(part_key, original Lineitem rowID)`, and only after a green Part exists. CPU gathers `(order_key, supplier_key)` and probes the unchanged composite/remaining typed hash indexes. |

The manual implementations are
[`manual_h03.rs`](../experiments/mac/query-expansion/src/manual_h03.rs) and
[`manual_h09.rs`](../experiments/mac/query-expansion/src/manual_h09.rs).
They execute complete queries with real Metal builds and CPU reads from the
completed shared allocations; they do not call the compiled query internally.
The driver independently executes the compiled originals and compares complete
canonical answer vectors. Duplicate witnesses are preserved before canon.

The 65,539-Lineitem synthetic run passed: H03 built 4,372 selected Orders and
returned 4,372 answers; H09 built 65,539 Lineitem occurrences and returned 16,372
answers. Boundary fixtures produced four answers each from respectively 12 and
68 pre-canon witnesses. They cover duplicate keys/rows, missing join partners,
strict date cutoffs, case-sensitive substring predicates, u32 maximum keys,
and negative/full-width payload fields. Empty/no-match cases passed; H09 with
no green Part performed zero GPU builds. Wider or signed GPU keys are rejected
explicitly while the compiled original still accepts them.

The larger `-- 1048579` run also matched complete compiled answers. Its single
execution observations on the M3 Max were:

| Query | GPU-indexed occurrences | Canonical answers | Compiled CPU ms | Manual hybrid ms |
|---|---:|---:|---:|---:|
| H03 | 69,908 selected Orders | 69,908 | 20.501 | 29.653 |
| H09 | 1,048,579 Lineitem | 261,754 | 126.061 | 84.370 |

These fixtures are not dbgen data or DuckDB-oracle measurements. The executable
prints single-execution times including preparation, builds and CPU joins,
excluding context setup and input generation. They do not isolate provider
costs: the handwritten realization also uses explicit row gathers and its own
CPU loop/dedup implementation. No speedup or placement threshold is established.

### Decisions from the expansion, before compiler integration

1. **A useful index handle needs a pinned source-row owner and a projection.**
   The GPU payload is an occurrence position, not the logical query payload.
   The manual functions safely borrow `Data` for their complete execution; a
   reusable snapshot must bind the matching owner to the index rather than let
   callers combine arbitrary source rows and positions.
2. **Construction follows the selected query plan.** H03 cannot prebuild raw
   Orders and call it the same plan: its build depends on a filtered Customer
   index. H09 must retain demand-driven construction. Context, fallibility and
   completion belong at those construction boundaries, not in a hidden
   provider-name rule or an infallible `Build` adapter.
3. **Read shape is more than scalar `IndexRead`.** H09's Lineitem key is not its
   leading declared column: expansion selects its fallback read, whereas the
   leading/composite-key hosts use native-capable dispatch. A scalar u32 cursor
   alone does not satisfy that projected tuple/payload contract. H03 also
   consumes distinct build keys for its join filter; the manual implementation
   explicitly derives those keys from the same selection.
4. **Keep CPU and GPU choices per index.** H09's composite PartSupp index is not
   packed into u32. Strings/dates and full-width gathered payloads stay typed on
   the CPU. General adoption still needs full key encodings, and input packing,
   row gathers, source-owner retention and synchronization must be charged.
5. **LSM maintenance is not query maintenance.** A pinned append-only Lineitem
   index could be reused across H09 queries with a matching source generation.
   H03's derived Orders index additionally depends on Customer membership:
   newly qualifying Customers can admit old Orders. Appending only new Orders
   cannot maintain that derived relation correctly.

The initial manual expansion changed no runtime API or compiler integration.
The controlled construction comparison below subsequently adds an explicit CPU
constructor; compiler integration remains unchanged.

## Controlled construction placement comparison

The experiment executes one H09 function with a constructor callback (initially
a function pointer; now a mutable callback also permits an explicit workspace).
CPU and GPU modes share projections, source rows, source-rowID payloads,
`MacRun` layout, CPU probes, residual indexes, loop order and output deduplication.
Only construction changes.
CPU construction uses stable `slice::sort_by_key` plus an exact radix directory;
GPU construction uses the measured stable 8-bit radix kernels. CPU stable-sort
scratch follows Rust's normal allocation behavior, not a recoverable `MacError`
guarantee for every possible allocation failure.

```sh
cargo run --release --locked \
  --manifest-path experiments/mac/query-expansion/Cargo.toml \
  --bin construction -- \
  --sizes 16387,65539,262147,1048579,4194307 --runs 9 \
  --out experiments/mac/query-expansion/results/construction-m3max-20261025.json
```

Two warmups per mode precede nine paired samples at each size. Pair order
alternates CPU-first/GPU-first. Every timed query reconstructs its demanded
indexes; only the input data, context and validation reference persist. All
samples match the authored query's complete answers, identical enumeration
order and identical pre-canon witness/gather counts. Validation and output
destruction occur outside timing. Total time includes input packing, buffer
allocation, construction/encoding/wait, remaining CPU indexes/joins, dedup and
temporary destruction; it excludes context setup and fixture generation.

M3 Max medians, milliseconds:

| Lineitem rows | CPU build | GPU build | CPU-built whole query | GPU-built whole query |
|---:|---:|---:|---:|---:|
| 16,387 | 0.159 | 1.143 | 1.040 | 2.070 |
| 65,539 | 0.537 | 1.072 | 3.561 | 4.020 |
| 262,147 | 5.595 | 1.463 | 18.755 | 14.455 |
| 1,048,579 | 22.096 | 2.727 | 99.448 | 79.809 |
| 4,194,307 | 88.267 | 9.032 | 493.206 | 412.039 |

[Raw samples and quartiles](../experiments/mac/query-expansion/results/construction-m3max-20261025.json)
include separate packing and remaining-query times. The remaining work is
similar across modes; unlike the earlier compiled-versus-manual comparison,
the principal time difference here is localized to index construction.
The crossover lies between sampled 65K and 262K sizes **for this fixture and
these constructors**, not at a portable threshold. This is not a fastest-CPU
algorithm claim: a CPU radix builder or different layout is a separate candidate.
These are warmed synthetic runs, with normal hash randomization and possible
external OS contention; not dbgen results or a bandwidth-saturation measurement.

Six runtime tests passed after extending CPU/GPU occurrence equivalence, all
radix-prefix boundaries, mixed-construction GPU merges and context-disposal
checks. The sweep also exercised duplicates, full-width payloads, empty inputs
and no-green-Part zero-build demand under both constructors.

## Compiler decision: separate construction from access

The controlled experiment demonstrates two construction implementations of
**one physical representation and one read implementation**. Its abstract
occurrence bag, stable equal-key chronology and query meaning do not change.
Operational requirements differ: CPU construction returns after initialization;
GPU construction returns after successful command completion. Unified memory
does not eliminate that completion boundary.

### What the current compiler already records

| Concern | Existing contract |
|---|---|
| Physical layout and payload | `Structure::Built`, `Structure::Host`, and `Keyed { ty, levels, supports, membership, filtered }` in `cq-lang-structures`. These `levels` are key levels, not machines. |
| Legal reads on a machine | Sequence `ProviderToken { capabilities, supports, levels, elements }`. Its `levels` explicitly govern consumption, not construction (`cq-lang-sequence/src/lib.rs`). |
| Execution and publication | Sequence `Grade`, `Store` and derived `Dispatch { level, producer_level, failure_channel, ... }`. Existing grading proves scope/publication obligations, not a general pending-index readiness protocol. |
| Native construction ABI | `cq-print::Context::materialize` emits `Build::build`; native fallback uses `Lazy<T,F>`. Both are context-free/infallible at that interface. Mac construction needs an explicit context and error policy. |
| Advice | Sequence `Advice::apply` replays checked rewrites. Recorded grades/rewrites are decisions, while catalog ordering and machine geometry guide candidate selection. “Advice” does not universally mean a soft hint. |

`supports [retain, expand, keys]` remains a read-law declaration. It must not
acquire a hidden meaning of “GPU-buildable.” Retyping chooses a representation;
`build_host_reads` changes retention/materialization and is not build-placement
advice. In particular it is not a lazy-preserving substitute for H09's fallback.
The manual H09 creates residual lazy HashIndex handles after the first green
Part; their source traversal/bucket construction still waits for first lookup.
The compiled fallback defers even those handle allocations through `Lazy`.

### Recommended boundary, not committed surface syntax

1. **Runtime-private choice:** if CPU/GPU builders satisfy the same input domain,
   source chronology, completed-handle and read contracts, choosing between them
   inside an explicitly configured runtime policy does not require new logical
   or scheduling IR. The policy and failure behavior must not be hidden changes.
2. **Compiler-controlled construction:** if advice chooses the builder, retain
   the choice as a separately inspectable, checked, replayable construction
   decision. Identify a construction site and its exact input/projection, not
   merely a relation name. Keep representation identity, builder implementation,
   execution place, read places and source-owner requirements distinct.
   Target-owned catalog entries should declare lawful construction choices.
   Existing provider read levels are not a substitute for this missing contract.
3. **Advice strength:** a required builder/place errors if unsupported; a
   preference may resolve to another lawful candidate, but the resolved choice
   must be visible. An explicit automatic policy may decide from actual selected
   row counts at runtime. No universal crossover constant follows from this run.
4. **First compiler slice:** a target-owned synchronous construction realization
   should accept caller context, propagate build errors, bind rowIDs to the
   borrowed/pinned source owner, and preserve H03 dependencies and H09 demand.
   Existing external terminal arguments can carry context without teaching the
   facade Metal names. Before extending shared IR, prove that terminal path on
   these two workloads; no provider-spelling exceptions or global infallible
   `Build` workaround.
5. **Scheduling is separate:** only compiler-owned overlap, transfers or
   persistent maintenance need explicit pending/ready dependencies, publication,
   failures and versioned source/snapshot lifetimes. Do not encode these merely
   by tagging a data-structure type or a read level. The proposed incremental
   formal chapter is not an existing implementation of this lifecycle.

The separation is sufficiently concrete to start compiler integration design.
The exact shared construction-record grammar and a fallible target terminal
still need implementation/proof; this change introduces neither speculative
syntax nor an unverified compiler path.

Existing compiler boundary checks were exercised without changing compiler code:
`cargo test -p cq-sequence-plan --test laws` passed 30 tests, including catalog
read-level scope, authored-decision precedence and exact trace replay;
`cargo test --manifest-path features/rewrites/Cargo.toml --test provider_support`
passed 13 tests. These validate the current contracts, not the proposed
construction record or a fallible Mac terminal.

## CPU tuning and GPU bandwidth reconciliation

The earlier end-to-end construction timings are not GPU bandwidth measurements.
The matching bandwidth reference is the M3 Max `gpu-index` experiment, not the
M5 Pro primitive-calibration coefficients in `experiments/mac/README.md`.
The reference preallocates and pre-touches buffers outside its timing window.

### Matched M3 Max reference rerun

The existing five-sample sort/copy sweep reproduced the historical throughput:

| Rows | Copy GPU ms | Copy logical GB/s | Four-pass radix GPU ms | Radix logical GB/s |
|---:|---:|---:|---:|---:|
| 1,048,576 | 0.040 | 414.68 | 0.360 | 268.15 |
| 16,777,216 | 0.764 | 351.45 | 5.404 | 285.66 |
| 67,108,864 | 3.023 | 355.18 | 22.281 | 277.13 |

[Full reference rerun](../experiments/mac/gpu-index/m3max-bandwidth-recheck-20261025.json).
These are modeled logical buffer bytes per GPU timestamp duration, **not DRAM
controller counters**. Cache-resident copy can exceed DRAM bandwidth. Four
radix passes plus histogram/scan traffic move about 96.5 MB of modeled traffic
for 1M eight-byte rows, not merely the 8 MiB final array. The 67M sort moves
about 6.175 GB in that model; its kernel time is consistent with the previous
277.79 GB/s result.

### Fresh constructor versus resident kernels

`query-expansion --bin bandwidth` validates complete stable rows after each
build and records raw phases. It compares comparison-sort CPU, CPU radix,
profiled fresh GPU, explicitly pre-touched GPU, and unprofiled fresh GPU.
The initial mixed-constructor sweep is preserved in
[`bandwidth-m3max-20261025.json`](../experiments/mac/query-expansion/results/bandwidth-m3max-20261025.json).
It used the initial unbuffered CPU radix candidate, not the final tuned code.

A separate GPU-only sweep removes interleaved CPU sorts. Nine-sample medians for
67,108,864 uniform keys:

| Fresh construction mode | Total host ms | CPU upload ms | Explicit pretouch ms | Commit/wait ms | GPU execution ms |
|---|---:|---:|---:|---:|---:|
| No pretouch | 77.581 | 33.637 | 0 | 42.922 | 24.734 |
| Pretouched scratch/metadata | 92.585 | 34.013 | 24.370 | 33.980 | 23.201 |

[GPU-only raw phases](../experiments/mac/query-expansion/results/bandwidth-gpu-burst-m3max-20261025.json).
GPU execution is **inside** commit/wait; do not add it again. Marginal phase
medians need not sum to the total median. Allocation API calls themselves were
small; first population of new shared mappings and submission/residency work
are charged elsewhere. Paying a CPU zero-fill merely to improve the reported
GPU timestamp can make the complete build slower.

Small fresh builds still differ materially from the preallocated reference:
the GPU-only 1M uniform case took 2.636 ms total, including 0.548 ms upload and
2.019 ms commit/wait; GPU execution was 1.260 ms. Removing interleaved CPU sorts
did not erase that gap. These results do not justify attributing every residual
delay to clock ramp, page faults or shader bandwidth without additional evidence.

### Metal trace evidence and limits

A separate Instruments Metal System Trace captured seven fresh 16M-row builds.
The driver recorded 14 successful 128 MiB memory-wiring events, totaling
14.741 ms, consistent in size/count with two row buffers per build. This is
direct evidence of residency work beyond shader execution, not just a theory
based on the wall/GPU difference.

[Target-only trace summary](../experiments/mac/query-expansion/results/trace-summary-m3max-20261025.json);
the full trace and exported XML remain under the experiment's ignored `target/`.
The default template captured no DRAM bandwidth counter set. Device performance
states were not converted to clock frequencies or used to assert throttling.
Instrumented trace durations are not mixed into benchmark medians.

The next GPU construction optimization target is input population and resource
residency/reuse, not a conclusion that the radix kernels cannot reach the
previous throughput. Any scratch reuse must remain private and wait for GPU
completion; published output buffers cannot be recycled while a snapshot owns
them. Input packing and these lifecycle costs must remain visible in the
construction cost model.

### Final CPU candidate and revised H09 comparison

The CPU candidate retains the shared `MacRun` layout, one thread, full u32 keys
and stable source occurrences. Histogram collection skips byte positions proved
constant during input population. Scatter buffers 16 rows (one measured
128-byte M3 Max cache line) per radix bucket, avoiding a separate scattered
write for every input row. The temporary write buffers occupy 32 KiB on the
stack, in addition to at most 8 KiB of counters; no per-row allocation is added.

The initial unbuffered radix candidate remained disproportionately slow on the
dense periodic fixture. Omitting constant-byte histogram increments alone did
not fix it. Buffered contiguous writes materially improved that case; cache-set
conflict/write-allocation pressure is the interpretation, not a hardware-counter
measurement. Buffering trades some uniform-key throughput for better dense-key
behavior, so this is an explicit candidate, not a claim of the best CPU sort.
`build_on_cpu` remains the comparison-sort baseline; neither constructor
silently selects a processor or changes the query's read implementation.

Nine-sample standalone construction medians (fresh shared buffers, milliseconds):

| Rows | Key distribution | Comparison sort | Buffered CPU radix |
|---:|---|---:|---:|
| 1,048,576 | full-width uniform | 15.235 | 6.837 |
| 1,048,576 | dense periodic | 22.081 | 6.313 |
| 4,194,304 | full-width uniform | 65.626 | 27.895 |
| 4,194,304 | dense periodic | 89.240 | 32.864 |
| 67,108,864 | full-width uniform | 1185.618 | 449.076 |
| 67,108,864 | dense periodic | 1439.549 | 566.375 |

[Buffered CPU raw samples](../experiments/mac/query-expansion/results/bandwidth-cpu-buffered-m3max-20261025.json).
The intermediate unbuffered/active-byte-histogram measurements remain in
[`bandwidth-cpu-tuned-m3max-20261025.json`](../experiments/mac/query-expansion/results/bandwidth-cpu-tuned-m3max-20261025.json);
they are historical tuning evidence, not the final implementation.

The same-query H09 sweep accepts `--cpu comparison|radix`. With `--cpu radix`,
nine paired samples produced:

| Lineitem rows | CPU radix build ms | GPU build ms | CPU-built query ms | GPU-built query ms |
|---:|---:|---:|---:|---:|
| 16,387 | 0.120 | 0.992 | 0.911 | 1.816 |
| 65,539 | 0.361 | 1.074 | 3.303 | 3.973 |
| 262,147 | 1.579 | 1.540 | 15.965 | 15.571 |
| 1,048,579 | 6.421 | 2.805 | 85.197 | 81.475 |
| 4,194,307 | 32.539 | 8.166 | 439.805 | 414.116 |

[Final H09 raw samples](../experiments/mac/query-expansion/results/construction-cpu-buffered-m3max-20261025.json).
All samples matched complete authored answers and identical source-order,
gather and witness counts. Seven runtime tests passed, including zero/odd/even
active radix-byte counts, noncontiguous varying bytes, duplicate payload order,
empty/extreme inputs, both profiling modes and GPU merge of CPU-built runs.
No compiler code changed and no crossover threshold was installed in advice.

Reproduce the final candidates and phase diagnostics from the repository root:

```sh
cargo run --release --locked --manifest-path experiments/mac/query-expansion/Cargo.toml \
  --bin construction -- --cpu radix --runs 9
cargo run --release --locked --manifest-path experiments/mac/query-expansion/Cargo.toml \
  --bin bandwidth -- --modes cpu_comparison,cpu_radix --runs 9
cargo run --release --locked --manifest-path experiments/mac/query-expansion/Cargo.toml \
  --bin bandwidth -- \
  --modes gpu_fresh_profiled,gpu_pretouched_profiled,gpu_fresh_unprofiled --runs 9
```

The CPU improvement narrows the whole-query GPU benefit substantially compared
with the original comparison-sort baseline. Placement must charge the chosen
CPU algorithm, input packing, resource residency and remaining CPU work—not
derive a fixed threshold from the nominal GPU bandwidth or an untuned baseline.

## Apple memory contracts and allocation-policy audit

The measurements do **not** establish that shared storage is an incorrect
allocator for CPU-readable indexes. The old helper did, however, conflate
different access roles by allocating every buffer as shared/default-cache and
requiring every allocation to have a CPU mapping. The implementation now has
separate checked allocation paths for cached shared, write-combined shared
upload, and GPU-private storage. Private allocations are never inspected
through `contents()`.

### Primary Apple guidance

- [Choosing a resource storage mode for Apple GPUs](https://developer.apple.com/documentation/metal/choosing-a-resource-storage-mode-for-apple-gpus):
  shared and private resources both use system memory on Apple GPUs. Shared is
  CPU/GPU-accessible; private is GPU-only and is recommended for GPU-owned
  intermediates. Private does not mean a separate dedicated VRAM pool.
  Memoryless storage is GPU tile memory for textures, not a buffer allocation
  option for these indexes.
- [Private storage](https://developer.apple.com/documentation/metal/mtlstoragemode/private):
  Metal **may** apply optimizations unavailable for shared/managed resources;
  this is not a guarantee that every private-buffer workload is faster.
- [CPU write-combined caching](https://developer.apple.com/documentation/metal/mtlcpucachemode/writecombined):
  intended for resources the CPU writes but never reads; CPU reads may perform
  very poorly. Apple cautions against using it without a demonstrated
  cache-pollution problem. It is not appropriate for the published CPU-probed
  row array or directory.
- [Shared storage synchronization](https://developer.apple.com/documentation/metal/mtlstoragemode/shared):
  CPU/GPU access must still be synchronized. Completion before publication
  remains required; unified memory is not permission for concurrent mutation.
- [Residency sets](https://developer.apple.com/documentation/metal/mtlresidencyset)
  and [requestResidency](https://developer.apple.com/documentation/metal/mtlresidencyset/requestresidency()):
  GPU residency is distinct from CPU access. Early residency preparation can
  defer work under contention and is not a completion or hazard-synchronization
  barrier. No residency-set shortcut was added here.
- [Persistent objects](https://developer.apple.com/library/archive/documentation/3DDrawing/Conceptual/MTLBestPracticesGuide/PersistentObjects.html)
  and [CPU/GPU resource reuse](https://developer.apple.com/documentation/metal/synchronizing-cpu-and-gpu-work):
  allocate early and reuse resources, respecting completion and ownership,
  rather than recreate every resource in a compute loop.
- [No-copy buffers](https://developer.apple.com/documentation/metal/mtldevice/makebuffer(bytesnocopy:length:options:deallocator:)):
  require a page-aligned region within one VM allocation and correct ownership.
  An arbitrary Rust `Vec` is not automatically a valid no-copy Metal allocation.
  Unified physical memory does not remove the copies our code explicitly makes.

### Access-role decision

| Resource | CPU use | Appropriate options |
|---|---|---|
| Published sorted rows and exact directory | Reads after GPU completion | Shared, default CPU cache |
| In-place build input/output | CPU writes first, CPU reads after GPU sort | Shared, default CPU cache |
| Sort scratch and histogram/scan levels | None in ordinary GPU construction | Private is lawful/recommended; shared remains a measured reference |
| CPU comparison/radix scratch | CPU reads and writes | Shared, default CPU cache |
| Separate upload input | CPU writes only; GPU reads before release | Shared cached, or experimentally write-combined |
| GPU merge partitions | GPU-only | Private is lawful; current merge allocation was not changed in this construction comparison |

Managed storage's separate-copy/synchronization model is not a faster shared
mode selected for this Apple-GPU runtime. Disabling hazard tracking or making
the final output private would not be a valid fix for the current CPU-read API.

### Controlled role/cache comparisons

`BuildMemory` provides `Shared`, `SharedPretouched`, `PrivateScratch`,
`CachedUpload`, and `WriteCombinedUpload`. Normal `build` remains `Shared`:
the measured benefit of private scratch was inconsistent, so no default speed
claim or policy flip is made. All modes use the same sort/directory kernels and
stable occurrence semantics.

- `Shared` versus `PrivateScratch` holds in-place upload/output topology fixed.
- `CachedUpload` versus `WriteCombinedUpload` holds the separate-input/private-
  scratch/cached-output topology fixed; only upload CPU caching changes.
- Separate upload is read directly by radix pass zero, not copied by an added
  blit. It still adds a buffer, and the final output is now first populated by
  the GPU rather than by CPU upload.
- Every build is followed by a separately timed first harness CPU checksum
  scan and complete exact row validation. The checksum scan is not included
  in construction time and is not a pure memory-bandwidth measurement.

Nine-sample build medians for 1,048,576 uniform rows, milliseconds:

| Policy | Original mode order | Reversed mode order |
|---|---:|---:|
| All shared, in-place input/output | 2.697 | 2.754 |
| Private scratch, shared in-place input/output | 2.644 | 2.887 |
| Private scratch, separate cached upload | 2.773 | 2.922 |
| Private scratch, separate write-combined upload | 3.001 | 3.188 |

[Full size/distribution sweep](../experiments/mac/query-expansion/results/memory-roles-m3max-20261025.json)
and [reversed-order sweep](../experiments/mac/query-expansion/results/memory-roles-reversed-m3max-20261025.json).
Both cover uniform and dense keys; the first spans 64K through 67M rows.
Write-combined input population was slower than its cached counterpart in
these measurements, matching Apple's warning that the mode has pitfalls.
Private scratch did not consistently remove the constructor's excess over the
resident-buffer kernel benchmark. First CPU checksum scans also distinguished
in-place and separately GPU-populated output even though both outputs had the
same default-cache shared storage; cache flags alone do not describe first-use
or residency costs.

Seven runtime tests passed with all five memory policies in profiled and
unprofiled construction. Complete H09 answers and witness order also passed
with [private scratch](../experiments/mac/query-expansion/results/construction-private-memory-m3max-20261025.json),
[cached staging](../experiments/mac/query-expansion/results/construction-cached-upload-smoke-20261025.json)
and [write-combined staging](../experiments/mac/query-expansion/results/construction-writecombined-smoke-20261025.json).
The H09 comparison driver accepts `--gpu shared|shared-pretouched|private|cached-upload|write-combined-upload`.

Conclusion: allocation **role**, CPU cache policy, residency, data initialization
and lifetime must be distinguished. Shared/default-cache is correct for the
CPU-readable result. These tests motivated safe resource reuse and direct input
population rather than a blanket replacement of shared memory with private or
write-combined memory. The following investigation measures first-touch and
implements opt-in reuse. No core compiler or synchronization semantics changed.

## First-touch and resource-lifetime root-cause isolation

### Apple-specific sources and what they establish

[Metal for Pro Apps, WWDC19](https://developer.apple.com/videos/play/wwdc2019/608/)
explicitly shows virtual-memory faults and zero-fill cost stalling CPU work on
large resources, and recommends pre-warming and reusing buffers. This predates
Apple-silicon Macs and is not used as a numerical M3 coefficient.
[Apple's allocation guidance](https://developer.apple.com/library/archive/documentation/Performance/Conceptual/ManagingMemory/Articles/MemoryAlloc.html)
distinguishes virtual allocation from first-use physical backing, zero-fill and
page-table updates. Its archived 4 KiB examples are not the page size assumed
here: `getconf PAGESIZE` and the diagnostic reported **16,384 bytes** on this Mac.
The modern [residency-set guidance](https://developer.apple.com/documentation/metal/simplifying-gpu-resource-management-with-residency-sets)
separately explains why resource preparation at command-buffer commit can delay
GPU submission.

Do not generalize these sources into “all Metal memory is committed at creation”
or “GPUs cannot handle page faults.” CPU fault counters do not measure GPU faults,
and unified memory does not by itself specify allocation/first-access behavior.

### Direct first-write control

The `gpu-index --bin first_touch` diagnostic uses resident source bytes and
compares fresh anonymous `mmap` regions with fresh cached/shared Metal buffers.
It times allocation, two equal-size writes, reads and release separately, with
`getrusage(RUSAGE_SELF)` fault deltas around each phase. Full contents are checked.
No GPU command executes in this control.

Nine-sample medians for Metal shared-buffer `memcpy`:

| Buffer bytes | First write ms | Second write ms | First-write minor faults | Second-write minor faults |
|---:|---:|---:|---:|---:|
| 8 MiB | 0.433 | 0.122 | 512 | 0 |
| 128 MiB | 7.103 | 2.160 | 8,192 | 0 |
| 512 MiB | 28.279 | 8.723 | 32,768 | 0 |

Anonymous-memory controls showed the same one-fault-per-page pattern and similar
timings. Major-fault deltas were zero. This is first-use VM work, not evidence
of disk paging or an intrinsically slow shared Metal cache mode.
[Raw first-touch samples](../experiments/mac/gpu-index/m3max-first-touch-20261025.json).
A separate System Trace recorded 106,698 target-process **Zero Fill** events
across the 128 MiB controls, including source allocation/startup; see the
[target-only trace summary](../experiments/mac/gpu-index/m3max-first-touch-trace-summary-20261025.json).
Instrumented event durations are not substituted for uninstrumented medians.

### Matched resource-lifetime ablation

The `lifetime` binary compiles canonical 8-bit/eight-row sort and directory
pipelines once. Separate immutable input ensures every repeated sort consumes
the same unsorted data. Cases change only specified allocations, CPU writes,
CPU reads, residency requests or command-object lifetime. Resource setup and
first submission are reported separately. Non-readback cases validate only
after the timed series, so checking does not accidentally introduce CPU reads.

67,108,864 rows, nine subsequent-submission medians:

| Case | CPU upload ms | Encode/commit/wait ms | GPU timestamp ms | Round total ms |
|---|---:|---:|---:|---:|
| Reuse buffers, no CPU rewrite | 0 | 22.835 | 22.508 | 22.838 |
| Reuse buffers, rewrite input | 8.678 | 23.365 | 22.584 | 32.027 |
| Fresh input only | 28.363 | 27.838 | 22.589 | 56.381 |
| Fresh output/directory only | 0 | 35.492 | 22.464 | 35.626 |
| Fresh scratch/scan only | 0 | 35.079 | 22.601 | 35.210 |
| Fresh everything | 28.538 | 53.181 | 22.840 | 81.930 |

GPU time is inside encode/commit/wait. Fresh allocation API calls themselves
were small; input first-touch faults and deferred submission work carried the
large costs. With fresh resources, explicitly requesting residency moved
29.279 ms into preparation and reduced encode/commit/wait to 23.303 ms, but
round total remained 81.278 ms. It moved the work, not removed it. Keeping
already reused resources explicitly resident gave no substantial additional
gain. Deferring autorelease drainage likewise did not explain the discrepancy.
[Full raw matrix](../experiments/mac/gpu-index/m3max-lifetime-20261025.json).

### Small-workload history was a separate confound

The same 1M-row reused-buffer kernel measured about 1.25 ms when run first,
but about 0.348 ms after the larger workload ran first in the same process.
Neither data nor the per-submission allocation/read/write policy changed.
[Uninstrumented ordering run](../experiments/mac/gpu-index/m3max-small-after-large-20261025.json).
An instrumented 1M → 16M → 1M run confirmed device performance-state transitions:
the initial 1M commands overlapped states 1/2, while the final identical 1M
commands overlapped state 3 and were faster. These numeric states are not
converted to clock frequencies; no fixed clock or DRAM counter was used.
[Instrumented timings](../experiments/mac/gpu-index/m3max-ordering-instrumented-20261025.json).

Thus a hot resident-buffer bandwidth coefficient is not a complete predictor
of isolated small query-build latency. Memory first use, residency preparation
and GPU workload/performance-state history are distinct measured effects.

### Historical raw-Metal correction: explicit bounded reuse

This subsection records the pre-MCCL runtime and its measured results. Its
`MacWorkspace` API and `workspace` executable were removed by the cutover below;
the corresponding commands apply only to that historical revision.

`MacWorkspace` retains scratch and a single spare output/directory pair. A weak
recycler in completed runs offers buffers only at their final `Rc` drop; the
workspace never overwrites a still-pinned run. It grows lazily, creates no large
allocation for a no-build query, and shares the actual GPU encode implementation
with fresh construction. No residency-set API is required in the production
workspace. Dropping it frees its cache; live runs retain their own allocations.

Nine alternating fresh/reuse pairs, same constructor fixture and read layout:

| Source pairs | Fresh constructor ms | Workspace constructor ms |
|---:|---:|---:|
| 1,048,576 | 2.702 | 1.949 |
| 16,777,216 | 25.863 | 11.605 |
| 67,108,864 | 83.730 | 40.348 |

[Constructor samples, including cold first calls](../experiments/mac/query-expansion/results/workspace-construction-m3max-20261025.json).
The fixture has repeated full-width keys and is not the lifecycle diagnostic's
SplitMix distribution. Only within-fixture comparisons isolate reuse.
The unchanged H09 execution body also passed with the workspace: every query
repacked/rebuilt its index, matched compiled answers and preserved witness/order
counts. Constructor medians improved; CPU joins still dominate full-query time.
[H09 samples](../experiments/mac/query-expansion/results/workspace-h09-m3max-20261025.json).
Whole-query median differences also contain residual CPU/cache/hash variance;
do not attribute them all to the measured constructor delta.

Ten runtime tests passed, including retained clones, partially consumed cursors,
grow/shrink/empty rebuilds, workspace/context disposal and GPU merge of retained
runs. The original GPU experiment's correctness gate and the migrated H03/H09
executable passed. This is opt-in memory retention, not an unbounded global pool,
query-answer cache, asynchronous scheduler or automatic placement threshold.
Creating a new workspace inside every query would forfeit the reuse benefit.

```sh
cargo run --release --locked --manifest-path experiments/mac/gpu-index/Cargo.toml \
  --bin first_touch -- --bytes 8388608,134217728,536870912 --runs 9
cargo run --release --locked --manifest-path experiments/mac/gpu-index/Cargo.toml \
  --bin lifetime -- --sizes 1048576,16777216,67108864 --runs 9
cargo run --release --locked --manifest-path experiments/mac/query-expansion/Cargo.toml \
  --bin workspace -- --mode constructor --sizes 1048576,16777216,67108864 --runs 9
cargo run --release --locked --manifest-path experiments/mac/query-expansion/Cargo.toml \
  --bin workspace -- --mode h09 --runs 9
```

Residency cases in the diagnostic require the supported macOS 15+ API and fail
explicitly when unavailable. The production workspace uses the existing
context's platform requirements and synchronous completion protocol.

## MCCL provider cutover

`experiments/mac/cq-mac` depends on the local `experiments/mac/mccl` workspace.
Both remain outside the portable core workspace. Core `MacContext` construction/merge,
`MacRun`, CQ read traits, chronological bags and pinned snapshot contracts remain.
MCCL provides stable u32/u32 GPU radix sort, partitioned merge, exact radix-12
directories and explicitly selected CPU comparison/radix construction. Tuple
input is written field-by-field into exclusive MLX allocations without an
intermediate converted vector. GPU publication waits for the directory and all
its dependencies; reading a published run performs no evaluation or FFI.

Migration details:

- Removed `BuildMemory`, `BuildProfile`, `MacWorkspace`, `build_with_memory`,
  `build_profiled`, and the duplicate production Metal/CPU-radix implementations.
  Use normal `build` with MLX's pool; process-wide cache/wired controls are
  explicitly available through `cq_mac::memory` on macOS.
- `MacError::ContextMismatch` replaces device-only acceptance. Independently
  created contexts cannot merge or extend one another's nonempty manifests.
  Rejected appends preserve the previous snapshot; empty appends remain no-ops.
  MCCL backend errors retain their diagnostic message via `CommandFailed`.
- CMake records the configured native directory in MCCL/MLX dylib install names.
  A downstream Rust dependency therefore needs no copied rpath or `DYLD_*`
  workaround. Relocation requires native reconfiguration and consumer relinking.
- Tile selection uses compiled pipeline resource limits. Normal sort/merge
  prefer eight/four rows per thread; smaller four/two-row variants accommodate
  instrumentation or tighter limits. Shader Validation enlarged the eight-row
  scatter to 37,396 bytes on this M3 Max, exceeding its 32,768-byte limit; the
  smaller stable variant permits validation without suppressing the check.

Current diagnostic commands, from the fork root:

```sh
cargo run --release --locked --manifest-path experiments/mac/query-expansion/Cargo.toml -- 1048579
cargo run --release --locked --manifest-path experiments/mac/query-expansion/Cargo.toml \
  --bin construction -- --cpu radix --sizes 65539,1048579 --runs 9
cargo run --release --locked --manifest-path experiments/mac/query-expansion/Cargo.toml \
  --bin index_lifecycle -- --sizes 65536,1048576 --runs 5
cargo run --release --locked --manifest-path experiments/mac/query-expansion/Cargo.toml \
  --bin pool -- --mode constructor --sizes 1048576,16777216,67108864 --runs 5
cargo run --release --locked --manifest-path experiments/mac/query-expansion/Cargo.toml \
  --bin pool -- --mode h09 --sizes 65539,1048579 --runs 5
```

`construction` compares explicit CPU and GPU construction inside identical H09
execution. The removed `--gpu` allocation-policy switch has no alias.
`index_lifecycle` replaces the old `bandwidth` driver: it records completed
construction and first CPU-scan wall times, not unavailable GPU timestamps.
`pool` replaces the old `workspace` driver: it compares clearing inactive MLX
cache before each sample against retaining the same bounded process-wide pool.
Cache-boundary time is recorded separately and excluded; previous cache limits
are restored, wired limits are unchanged, and first observations are separate
from paired samples. Clearing inactive cache does not guarantee cold physical
pages or affect pinned live outputs. Historical JSON results are unchanged.

On the M3 Max/macOS 27.0.1, direct native C smoke and the actual `hybrid` and
H03/H09 executables passed with Metal Shader Validation. `hybrid` preserved old
cursors while both policies reached 458,422 occurrences after seven batches.
The 1,048,579-Lineitem query smoke matched complete authored results: H03
69,908 answers and H09 261,754 answers. Empty/extreme/duplicate cases, row-ID
payload preservation, no-green demand avoidance and unsupported key rejection
also passed. These instrumented single runs are correctness evidence, not
performance crossover measurements. Native linking still warns about the
27.0 deployment target versus MLX built for 27.0.1; older-OS execution is not
verified. The unrelated `authored_advice` unused-variable compiler warning remains.

Final checks passed: six MCCL integration tests, all four index regressions again
with Shader Validation, eleven CQ runtime tests, strict Clippy for MCCL and
`cq-mac`, all query-expansion targets, and the offline kernel correctness CLI.
The unsupported-platform surface also cross-compiled for
`aarch64-unknown-linux-gnu` without MCCL; it was not executed on Linux.

### Post-cutover measurements

M3 Max, Shader Validation disabled, five alternating paired samples. H09 uses
identical synthetic fixtures, row-ID payloads, CPU joins and exact answer/work
checks; only explicit CPU-radix versus GPU construction changes:

| Lineitem rows | CPU-radix build ms | MCCL GPU build ms | CPU-radix query ms | MCCL GPU query ms |
|---:|---:|---:|---:|---:|
| 65,539 | 0.280 | 0.964 | 3.024 | 3.853 |
| 1,048,579 | 5.526 | 2.067 | 85.973 | 82.148 |

[Raw construction/query samples](../experiments/mac/query-expansion/results/mccl-construction-radix-m3max-20261025.json).
At 1M the constructor benefit is much larger than the whole-query benefit;
CPU joins still dominate. This does not justify an automatic placement threshold.
The separate [index lifecycle sweep](../experiments/mac/query-expansion/results/mccl-index-lifecycle-m3max-20261025.json)
also records uniform, dense, 97-key and all-equal inputs with three samples per
constructor. CPU radix remains faster for all-equal keys, including at 1M.

Same MCCL GPU constructor, clearing inactive cache versus retaining it:

| Source pairs | Cache-cleared construction ms | Pooled construction ms |
|---:|---:|---:|
| 1,048,576 | 2.630 | 1.519 |
| 16,777,216 | 24.502 | 16.744 |
| 67,108,864 | 88.636 | 61.875 |

[Raw pool construction samples](../experiments/mac/query-expansion/results/mccl-pool-construction-m3max-20261025.json).
Cache clearing itself is outside these timings and is recorded separately.
The bounded cache was 2,147,549,200 bytes; its previous limit was restored.
The [H09 pool sweep](../experiments/mac/query-expansion/results/mccl-pool-h09-m3max-20261025.json)
used a 33,620,080-byte cache and also restored its previous limit.
These are CPU-wall completion times, not GPU timestamps or DRAM counters.

This cutover establishes ownership/correctness, **not performance parity with the
old raw-Metal workspace**. Its historical 67M pooled constructor was 40.348 ms,
lower than the current MCCL result. That is not a simultaneous controlled
comparison, but the older result must not be presented as MCCL performance.
All historical raw artifacts remain unchanged.

## 2026-10-26 controlled construction investigation

The preceding first-touch/reuse work did **not** establish a single root cause
for all slow builds. In particular, historical raw-Metal and MCCL measurements
were not a controlled backend comparison. This investigation leaves production
algorithms, compiler policy and allocator defaults unchanged.

### Completed large matrix

The expanded `index_lifecycle` schema v2 measures total rows across independent
relations, retaining **every output until all relations have been built, read and
validated**. It records individual relation sizes, cardinalities, varying key
bytes, constructor/read/release times, process minor/major faults and MLX
active/cache/peak accounting. Process-wide cache limits are restored.

- Total rows: 65,536; 262,144; 1,048,576; 4,194,304; 16,777,216; 67,108,864.
- Relations: 1, 4, 16; balanced splits and an 80%-in-first-relation skew.
- Inputs: full-width xorshift keys, dense keys, 97 keys, all equal, ascending,
  descending, clustered and 90%-hot skew.
- Separate synthetic projections: Orders primary/customer keys, Lineitem
  order/part keys, Partsupp part keys, and mixed relation sets. These reuse
  documented H03/H09 formulas, not dbgen. Partsupp's filtered/distinct projection
  is explicitly rotated and cyclically resized to the requested relation size.
- CPU comparison, CPU radix and MCCL GPU construction; 2 GiB inactive cache.
  Skewed cases additionally compare cache-cleared and retained-cache boundaries.
- Five measured samples per mode, two warmups and separate first observations.
  Every sample validates complete stable rows and directory-backed lookups.

All **240 configurations / 720 mode configurations** completed: **3,600 measured
aggregate builds, constructing 24,660 individual indexes**, excluding first
observations and warmups. Reports indicate successful cache-limit restoration:

- [Balanced sizes/distributions/relations (lossless gzip archive)](../experiments/mac/query-expansion/results/index-matrix-balanced-m3max-20261026.json.gz)
- [Individually named relation projections](../experiments/mac/query-expansion/results/index-matrix-relations-m3max-20261026.json)
- [Skewed relation sizes and cache boundaries](../experiments/mac/query-expansion/results/index-matrix-skewed-m3max-20261026.json)

Same **1,048,576 total uniform rows**, all outputs simultaneously live:

| Relations | Rows per relation | CPU comparison ms | CPU radix ms | MCCL GPU ms |
|---:|---:|---:|---:|---:|
| 1 | 1,048,576 | 15.102 | 6.074 | 2.131 |
| 4 | 262,144 | 12.426 | 6.102 | 4.205 |
| 16 | 65,536 | 12.080 | 6.518 | 11.789 |

Total row count alone therefore cannot select a constructor. Independent
synchronous index builds repeatedly pay submission/readiness and directory
costs. Input structure also changes the best CPU algorithm: at 1M rows,
all-equal CPU radix took 0.456 ms versus GPU 2.013 ms; ascending comparison
sort took 0.745 ms versus GPU 2.087 ms. CPU radix omits constant key bytes;
the current GPU path executes all four byte-radix passes.

Individually measured synthetic relation projections, 1,048,576 rows each:

| Indexed relation/key | CPU comparison ms | CPU radix ms | MCCL GPU ms |
|---|---:|---:|---:|
| Orders / primary key | 1.383 | 7.170 | 2.034 |
| Orders / customer key | 8.528 | 7.226 | 1.600 |
| Lineitem / order key | 16.133 | 6.724 | 2.148 |
| Lineitem / part key | 22.550 | 5.983 | 2.041 |
| Partsupp / part key | 2.688 | 7.324 | 2.139 |

### Matched backend and bandwidth controls

`gpu-index --bin index_compare` uses identical tuples and source order in eight
rotating variants: raw two-buffer in-place, raw three-buffer out-of-place,
single concurrent encoder with explicit buffer barriers, single serial encoder,
MCCL public constructor, combined C-ABI graph, split-evaluation diagnostic, and
GPU copy reference. Raw allocation/pipeline setup is outside warmed samples;
MCCL still pays per-call cached allocation and graph work. Raw and MCCL use the
same canonical shader source, compiled at runtime/offline respectively.

All 16 size/input configurations completed, with 640 measured samples plus
first calls and warmups. [Raw controls](../experiments/mac/gpu-index/index-compare-m3max-20261026.json).
The `workspace` distribution is the **exact historical fixture**:
`((source_row_id * 40503) & 65535) * 65537`.

At 67,108,864 rows, warmed constructor CPU-wall medians:

| Implementation | Completed constructor ms |
|---|---:|
| Raw two-buffer, per-stage encoders | 34.198 |
| Raw three-buffer, per-stage encoders | 33.532 |
| Raw three-buffer, concurrent encoder + barriers | 33.833 |
| Raw three-buffer, serial encoder | 34.461 |
| MCCL public constructor | 32.974 |
| MCCL C-ABI combined graph | 31.702 |

Neither the third row-sized buffer nor MLX's encoder topology explains a
persistent 40-versus-62-ms penalty in this matched warm run. This does not erase
the older measurements or imply that workload history is irrelevant.

The copy reference measured 3.060 ms GPU time for 1,073,741,824 logical bytes,
or 350.9 GB/s. Raw radix/directory GPU time was 22.892 ms with 6,174,802,448
modeled sort-buffer bytes, about 269.7 GB/s. The sort model excludes directory
and threadgroup traffic; neither number is a physical DRAM-counter measurement.
Comparing copy GPU time against whole-constructor wall time would conflate
different work and timing scopes.

### Actual MCCL GPU and host phases

Opt-in `mccl_index_timing` wraps/delegates to the live MCCL primitives, retaining
their actual command buffers inside `eval_gpu`. It does not replace kernels,
allocation or scheduling. It records completed
[GPU timestamps](https://developer.apple.com/documentation/metal/mtlcommandbuffer/gpustarttime)
and [CPU-driver scheduling timestamps](https://developer.apple.com/documentation/metal/mtlcommandbuffer/kernelendtime),
native primitive encoding, CPU population and process faults. Command-buffer
intervals, scheduling and wall phases are not generally additive.

The initial continuously reused 67M probe measured 21.129 ms summed GPU time
inside 21.889 ms evaluation wall time; native sort encoding was 0.033 ms and
directory encoding 0.008 ms. [Timestamp samples](../experiments/mac/mccl/benchmarks/index-timing-m3max-20261026.csv).
The matched C-ABI comparator separately measured host-source population at
9.668 ms, cached input allocation at 0.0035 ms, and graph construction in
microseconds. The native probe generates directly into storage, so its
population time must not be substituted for the comparator's source-copy cost.

### Reproduction

From the fork root, build the two diagnostic binaries normally. Run GPU work
sequentially, without Shader Validation for timing:

```sh
cargo build --release --manifest-path experiments/mac/query-expansion/Cargo.toml --bin index_lifecycle
cargo build --release --manifest-path experiments/mac/gpu-index/Cargo.toml --bin index_compare
experiments/mac/query-expansion/target/release/index_lifecycle \
  --sizes 65536,262144,1048576,4194304,16777216,67108864 \
  --relations 1,4,16 --layout balanced \
  --distributions uniform,dense,97_keys,all_equal,ascending,descending,clustered,skew90,mixed_relations \
  --cache pooled --runs 5 --warmups 2 --out /tmp/index-matrix-balanced.json
experiments/mac/query-expansion/target/release/index_lifecycle \
  --sizes 65536,262144,1048576,4194304,16777216,67108864 --relations 1 \
  --distributions orders_primary,orders_customer,lineitem_order,lineitem_part,partsupp_part \
  --cache pooled --runs 5 --warmups 2 --out /tmp/index-matrix-relations.json
experiments/mac/query-expansion/target/release/index_lifecycle \
  --sizes 1048576,16777216,67108864 --relations 4,16 --layout skewed \
  --distributions uniform,dense,skew90,mixed_relations --cache pooled,cleared \
  --runs 5 --warmups 2 --out /tmp/index-matrix-skewed.json
experiments/mac/gpu-index/target/release/index_compare \
  --sizes 65536,1048576,16777216,67108864 --runs 5 --warmups 2 \
  --distributions uniform,workspace,dense,all_equal --out /tmp/index-compare.json
cmake -S experiments/mac/mccl -B experiments/mac/mccl/build/native -DCMAKE_BUILD_TYPE=Release
cmake --build experiments/mac/mccl/build/native --target mccl_index_timing --parallel 8
experiments/mac/mccl/build/native/mccl_index_timing --sizes 67108864 --runs 5 \
  --cache pooled --out /tmp/index-timing.csv
```

Matrix checkpoints are atomic and mark incomplete/failed/complete status.
Comparator checkpoints retain complete cases and a completion flag. Both use
buffered report writes outside measured operations; per-token unbuffered JSON
writes were removed after the small diagnostic exposed excessive reporting
overhead. Every reported large sweep used the corrected writer.



### Cold scheduling and workload-history controls

The phase probe now supports `--cache pooled|cleared|paired` and `--idle-ms`.
`pooled` never clears between observations and allows all recycled roles to
become CPU-touched. `paired` deliberately mixes clear/retain boundaries and is
**not equivalent to continuously warmed storage**: retained-boundary samples
still exhibited CPU population faults after preceding clears. Do not use them
as the fully warm baseline.

At 67,108,864 pairs, identical directly generated workspace input, five measured
samples, combined evaluation:

| Storage/history | Population ms | Population minor faults | CPU-driver scheduling ms | GPU interval sum ms | Evaluation wall ms |
|---|---:|---:|---:|---:|---:|
| Continuously pooled, active B | 7.245 | 0 | 0.072 | 21.179 | 21.924 |
| Clear inactive cache each sample | 29.263 | 32,768 | 27.389 | 27.148 | 55.361 |

The immediate allocation call and native primitive encoding do not account for
the cold evaluation delay. CPU-driver scheduling does: the cleared evaluation
also consumed 27.561 ms of process system time versus 0.092 ms for active B.
This is measured deferred scheduling/first-use cost, not evidence that the
shader itself ran for the entire 55 ms. The underlying driver work could include
mapping/residency preparation; its internal subdivision was not measured.
Population writes 512 MiB, with the cold fault count matching 32,768 16-KiB pages.

An active → 250-ms-idle → active sequence with continuously pooled storage:

| History | Population ms | Population minor faults | GPU interval sum ms | CPU-driver scheduling ms |
|---|---:|---:|---:|---:|
| Active A | 7.611 | 0 | 23.321 | 0.078 |
| Add 250 ms idle before each sample | 32.190 | 0 | 29.927 | 0.139 |
| Active B | 7.245 | 0 | 21.179 | 0.072 |

Idle time is outside every reported phase. The reversible slowdown persists
without CPU first-touch faults and without a large driver-scheduling interval.
Thus first touch alone is insufficient. Frequency/core/power-state changes are
plausible explanations, **not directly observed clock telemetry**; CPU scheduling
and cache-state details are not fully isolated.

The actual Rust matrix also tested the same 67M uniform input and identical
validation with nine samples: GPU-only history gave 38.695 ms, while interleaved
CPU comparison/radix/GPU construction gave 53.996 ms. Cached bytes before each
GPU build were identical (1,644,232,740); construction faults were zero or small,
not a fresh 32,768-page population. This manipulation changes both preceding
CPU work and allocator/cache usage, so it establishes workload-history
sensitivity without uniquely attributing it to GPU clocks.

Raw evidence:
- [Active A](../experiments/mac/mccl/benchmarks/index-steady-active-a-m3max-20261026.csv),
  [idle](../experiments/mac/mccl/benchmarks/index-steady-idle-m3max-20261026.csv),
  [active B](../experiments/mac/mccl/benchmarks/index-steady-active-b-m3max-20261026.csv),
  [cache-cleared](../experiments/mac/mccl/benchmarks/index-steady-cleared-m3max-20261026.csv).
- [Paired size/scheduling probe](../experiments/mac/mccl/benchmarks/index-scheduling-m3max-20261026.csv)
  and [paired idle probe](../experiments/mac/mccl/benchmarks/index-scheduling-idle-m3max-20261026.csv);
  retained boundaries here are not the continuously warmed control.
- [GPU-only matrix history](../experiments/mac/query-expansion/results/index-history-gpu-only-m3max-20261026.json)
  and [CPU/GPU-interleaved history](../experiments/mac/query-expansion/results/index-history-interleaved-m3max-20261026.json).

For the native A/B/A control, run the final reproduction command with
`--cache pooled`, then `--cache pooled --idle-ms 250`, then `--cache pooled`,
using different output paths. `--cache cleared` measures cold scheduling.
The old timestamp CSV predates these extra columns and remains unchanged.

### What this establishes

There is no supported single “wrong allocator” diagnosis. Measured contributors
are CPU page first-touch/population, deferred Metal driver scheduling, per-index
submission/readiness overhead, CPU/GPU workload history, and input-dependent
algorithm work. Fully warmed large GPU construction and the same raw kernels
are much closer than the earlier cross-run comparison suggested. CPU comparison
and CPU radix each win on some input shapes; GPU wins strongly on large
unordered inputs, but splitting work among many small indexes can reverse that.

All diagnostics validate full stable output, with directory checks appropriate
to their exposed interface. The final raw comparator smoke and the existing
offline sort/merge/group/directory correctness executable passed after the
encoder-stage refactor. Measurements ran sequentially on AC power; `pmset`
reported no recorded thermal/performance warning before and after. These checks
do not substitute for clock telemetry or physical DRAM counters. Existing
Objective-C macro/dead-code and native deployment-target warnings remain.

## Shared-relation input cutover

GPU index construction now takes an existing immutable `Relation`, an
`IndexSpec { key_column, payload }`, and an optional selection relation. The
host-slice constructors and `upload_index_rows` path are removed rather than
wrapped. Relation ingestion is explicit and reusable:

- `Gpu::relation_u32` / `relation_i64` and the CQ equivalents allocate shared
  storage and let a loader/generator initialize rows directly once. No temporary
  key/payload vector is required. Ingesting an existing CPU collection still
  costs a copy; it is not represented as free or repeated per index.
- `Gpu::relation_from_u32` retains a contiguous `U32Array` as a row-major source
  without copying or evaluating it. A lazy GPU-produced source remains a graph
  dependency after its original handles are dropped.
- The GPU builder performs column extraction, selected-value validation, original
  source row-ID generation, stable mask compaction, radix sorting and directory
  construction. CPU construction code reads only scalar count/error metadata
  before publication; it never scans/packs source values or initializes index
  rows, directory or scratch.
- The native implementation uses one dependency graph, not a CPU count/readback
  between projection and sorting. GPU kernels read the selected count directly.
  Only the initialized selected prefix is exposed in the returned row array.
- Source arrays are never donated or modified. An already-packed unmasked u32
  key/payload relation uses the identity projection without another GPU copy.
  CPU comparison/radix baselines remain explicit CPU algorithms over the same
  retained source; they are not part of the GPU path.

This changes the construction boundary, not the read model: CPU query probes
and correctness validation may read completed indexes. Metadata/ownership and
driver work still run on the CPU. This is not a GPU-only query engine or a
parallel RadixSpline implementation.

### Actual query migration

`manual_h09::Prepared` retains all numeric Lineitem columns as i64
`[order_key, part_key, supplier_key]`. Repeated executions build directly from
that relation; GPU projection selects the part key and generates original row
IDs. CPU payload gathering reads the retained shared source, preserving signed
and wide values. The per-execution tuple-packing vector is gone. No-green demand
still avoids both ingestion and index construction.

`manual_h03::Prepared` similarly retains full numeric Orders rows. The existing
opaque CPU Customer predicate and semijoin produce a u32 selection mask directly
in shared storage. That mask is query input—not packed index keys or payloads.
GPU compaction retains original Orders positions, and selected invalid keys
are rejected by GPU validation. The original CPU join-filter operator remains.

Maintained appends now receive ingested relation handles. Flat/binary-carry
chronology, failure-atomic publication, context checks and pinned readers remain.
Source ingestion and GPU build are separately timed in the executable example.

### Verification and measurement boundaries

Release verification passed ten MCCL tests and twelve CQ tests. All eight
MCCL index tests also passed with Metal Shader Validation: multi-column reuse,
mask compaction and original IDs, selected signed-domain failures, lazy
GPU-produced inputs, ingestion unwinding, cache churn and retained ownership.
Direct C ABI smoke verified preservation of both outputs and count on failure.
The actual hybrid example reached 458,422 occurrences under each maintenance
policy while retaining old cursors. H03/H09 passed first and repeated execution
at 1,048,579 Lineitem rows with validation enabled, including empty/no-match,
duplicates/extrema, wide payloads and unsupported selected key rejection.
The non-macOS adapter also cross-compiled; no Linux runtime execution is claimed.

All active constructor benchmarks now retain shared sources across repetitions,
constructor alternatives and cache clearing. Ingestion remains a separate,
measured event. CPU oracle generation, output validation and CPU query reads
are not disguised as GPU construction. New schemas prevent mixing old boundaries:

| Driver | Current schema / boundary |
|---|---|
| `index_lifecycle` | `mccl.index-lifecycle.v3`: direct source ingestion once; optional `--source-columns 2` builds two different indexes from each same source; all outputs retained through checks. |
| `construction` | `mccl.query-construction.v2`: one H09 source ingestion, repeated complete queries, separate constructor time and ingestion-plus-first-query sums. |
| `pool` | `mccl.query-pool.v2`: clear inactive output/scratch cache versus retain it; live source is never reingested. |
| `index_compare` | `index_compare_v2_shared_source`: immutable packed source ingested once per backend; destructive two-buffer/repopulation cases removed; no CPU zero-fill of raw output/scratch. |
| `mccl_index_timing` | Shared single-column source ingested once; GPU generates payload IDs; actual fused-graph command-buffer timestamps and scalar-publication scope. |

Post-cutover raw evidence (historical files remain unchanged):

- [Source boundary smoke](../experiments/mac/query-expansion/results/shared-source-smoke-20261026.json).
- [36 size/input/relation configurations through 64M rows](../experiments/mac/query-expansion/results/shared-source-matrix-m3max-20261026.json).
- [16 two-column reuse/cache configurations](../experiments/mac/query-expansion/results/shared-source-multicolumn-m3max-20261026.json).
- [H09 CPU/GPU constructor comparison](../experiments/mac/query-expansion/results/shared-source-h09-m3max-20261026.json),
  [H09 pool cases](../experiments/mac/query-expansion/results/shared-source-pool-h09-m3max-20261026.json),
  [constructor pool cases](../experiments/mac/query-expansion/results/shared-source-pool-construction-m3max-20261026.json).
- [Matched shared-source raw/MCCL controls](../experiments/mac/gpu-index/shared-source-compare-m3max-20261026.json).
- [Native shared-source GPU timing](../experiments/mac/mccl/benchmarks/shared-source-index-timing-m3max-20261026.csv).

For the two-column uniform case, one 1,048,576-row shared relation occupied
8,388,608 logical bytes. Ingestion (including deterministic key generation)
took 3.231 ms once; both different GPU indexes together took 4.156 ms median.
At 16,777,216 rows, one ingestion took 61.572 ms and the two indexes together
took 21.512 ms. These are not before/after speedup claims: source layout and GPU
projection work differ from the old host-packed constructor.

The H09 comparison ingested its full 1,048,579-row numeric source once in
1.678 ms. Its GPU constructor median was 2.115 ms, and complete repeated-query
median 85.220 ms. Ingestion-plus-first-query fields are explicitly sums of
measured intervals, not contiguous cold-start wall times. Cold allocation,
driver scheduling, workload history and repeated per-index synchronization
still exist; removing a host copy does not remove those costs.

Current repeated multi-index command, from the fork root:

```sh
cargo run --release --locked --manifest-path experiments/mac/query-expansion/Cargo.toml \
  --bin index_lifecycle -- --sizes 1048576,16777216 --relations 1,4 \
  --source-columns 2 --distributions uniform,mixed_relations --cache pooled,cleared \
  --runs 3 --warmups 1 --out /tmp/shared-source-multicolumn.json
```

Final quality gates passed strict Clippy for all MCCL and CQ targets, the eight
index regressions after the final metadata-validation edit, all twelve CQ tests,
the actual hybrid executable, and the original offline GPU correctness driver.
The native source-only probe retained the same 64M-key relation across builds:
one-time deterministic ingestion was 15.698 ms and retained-cache construction
median 24.547 ms. This probe includes GPU row-ID projection and is not a
like-for-like speedup comparison with the old host-packed input path.


## Matched prepared-reader comparison (2026-10-27)

The `query-expansion --bin prepared` diagnostic compares actual GPU-built
`MacRun`, CPU-radix-built `MacRun`, native `RadixRun`, and runtime `HashIndex`.
All builders consume identical u32 key/payload pairs from one retained shared
relation. The two Mac constructors publish the same representation and use the
same CPU read implementation. This is not a GPU-read benchmark.

```sh
cargo run --release --locked --manifest-path experiments/mac/query-expansion/Cargo.toml \
  --bin prepared -- --keys 16384,262144 --probes 65536 --runs 7 \
  --out experiments/mac/query-expansion/results/prepared-readers-m3max-20261027.json
```

[Raw samples](../experiments/mac/query-expansion/results/prepared-readers-m3max-20261027.json)
contain 288 provider/workload/mode records and 2,016 measured samples. Hardware:
Apple M3 Max, 128 GiB, AC power; `pmset` reported no recorded thermal or
performance warning before measurement. There are two warmups and seven samples
per record, rotating provider order. All four indexes stay live together.

The matrix uses 16,384/262,144 distinct keys, 65,536 shuffled probes and requested
10/50/90% hit fractions (actual counts are recorded). Dense keys are even low
integers; sparse keys are an invertible affine permutation spread across u32;
skewed keys place seven eighths of keys in the low prefix and spread the rest
over the upper half. These are deterministic structured fixtures, not arbitrary
random-gap keys or production traces. Occurrence fanout is four, or 32 for the
duplicate-heavy sparse fixture; identical payload occurrences remain present.

Timings exclude ingestion, every constructor, full exact validation, native
lazy membership-bitmap initialization and retained-cursor buffer reservation.
Retained timing includes lookup, cursor storage and expansion. All modes consume
payload counts/checksums rather than materializing output tuples. Before timing,
every distinct key's complete payload sequence is checked against stable CPU
sorting, including occurrence order and multiplicity. Empty/singleton inputs,
u32 extrema and misses are also checked. Every timed result is checked.

Representative immediate lookup plus expansion medians, milliseconds:
262,144 distinct keys, 65,536 probes, approximately 50% hits.

| Fixture | GPU-built MacRun | CPU-built MacRun | Native RadixRun | Runtime HashIndex |
|---|---:|---:|---:|---:|
| Dense, 1,048,576 rows | 13.382 | 13.428 | 2.041 | 2.242 |
| Sparse, 1,048,576 rows | 3.896 | 4.187 | 2.196 | 2.351 |
| Skewed, 1,048,576 rows | 11.894 | 12.047 | 2.600 | 2.580 |
| Duplicate-heavy, 8,388,608 rows | 18.287 | 18.517 | 3.277 | 4.430 |

Across all 72 workload/mode pairs, the median GPU-built/CPU-built MacRun
read-time ratio is 1.000 (range 0.868–1.046). This is descriptive, not a
statistical equivalence claim. The cross-provider gap remains on CPU-built
MacRun, so it cannot be explained solely by GPU first-touch or construction.

The reader representations differ materially:

- MacRun selects `key >> 20` then binary-searches packed occurrence rows.
  The large dense fixture has one bucket containing all 1,048,576 rows; the
  skewed fixture's largest bucket contains 917,504 rows. Sparse and
  duplicate-heavy maximum buckets contain 264 and 2,112 rows respectively.
- Native RadixRun searches distinct keys through the spline, then expands a
  contiguous payload-only range. Its dense membership projection is already
  initialized in this prepared comparison. Dense membership at the same large
  50%-hit setting takes 0.0555 ms versus MacRun's 3.5873 ms and hash's 0.5178 ms.
- MacRun retains keys beside each payload during expansion; native grouped
  storage separates key search from payload enumeration. These structural
  differences identify optimization candidates, not an isolated attribution
  of timing to cache misses, branches or any one instruction.

The next representation decision should address distribution-aware directory
coverage and distinct-key/group-offset routing, while keeping GPU construction
and stable merge lawful. Parallel RadixSpline is one candidate, not a prerequisite
established by these measurements. Faster sort alone cannot remove a measured
prepared-read disadvantage. This experiment changes no runtime provider,
compiler placement, allocation policy or production lookup algorithm.

Verification: the small 8/1,024-key executable smoke and the complete matrix
passed exact checks. Strict Clippy encountered the existing `type_complexity`
diagnostic in `src/manual_h09.rs`; the unrelated source was not changed.

## GPU-built RadixRun (2026-10-27)

The GPU fork now exposes `MacContext::build_radix` and `merge_radix`, returning
`MacRadixRun`. This is not the old packed `MacRun` with a renamed directory:
the completed arrays are distinct u32 keys, native-width group offsets,
contiguous u32 payloads, key/rank spline endpoints and a min-relative adaptive
radix-12 directory. MCCL owns the buffers and completes the graph before exposing
them. The CQ adapter uses `cq_storage::RadixRunView`, the same allocation-free
lookup/range/expansion implementation now used by native `RadixRun`.

```rust
use cq_mac::{IndexSpec, MacContext, Payload};

let context = MacContext::new()?;
let source = context.relation_u32(4, |i| [(i % 2) as u32, i as u32])?;
let spec = IndexSpec { key_column: 0, payload: Payload::Column(1) };
let first = context.build_radix(&source, spec, None)?;
let second = context.build_radix(&source, spec, None)?;
let merged = context.merge_radix(&first, &second)?;
assert_eq!(merged.lookup(&1).collect::<Vec<_>>(), [1, 3, 1, 3]);
# Ok::<(), cq_mac::MacError>(())
```

### Construction and maintenance

1. Reuse existing GPU projection, selected-value validation, stable mask
   compaction and four-pass stable radix sort over the retained source.
2. GPU boundary flags, hierarchical scans and scatter produce distinct keys,
   group offsets and payload-only storage.
3. Independent 256-key chunks run the error32 greedy spline corridor using
   exact signed-64-bit orientation products. u32 key differences and chunk-local
   rank differences fit without float approximation. Every chunk retains its
   endpoints with global distinct-key ranks; adjacent chunk endpoints span
   consecutive ranks.
4. GPU scans compact spline points. A second exact pass removes interior points
   whose adjacent segments are collinear, preserving the same piecewise-linear
   function and bound. This removes redundant chunk boundaries on linear spans.
5. Parallel prefix searches build the native adaptive radix directory. Scalar
   completion metadata is read on CPU; no source scan, grouping, point scan or
   sequential CPU spline construction is hidden in publication.

The spline need not have the identical segmentation of the global CPU greedy
builder. It must satisfy the same error-bounded exact-read contract. Spline
scratch/output capacity is at most nine endpoints per 256-key capacity chunk.
Completed arrays are zero-copy initialized-prefix views; underlying capacity
allocations can exceed their logical lengths, particularly for duplicate-heavy
or selected input. They are not claimed to be physically right-sized.

Merge operates on the two distinct-key arrays and offsets, then copies payloads
into their chronological destinations and rebuilds groups/spline/directory on
GPU. It retains no second packed occurrence array in the completed index.
Lookup cursors borrowed from pinned old runs remain valid through merge and
owner/context disposal. Both inputs must belong to the same MCCL context.

The explicit GPU domain remains u32 keys and u32 payloads or original source row
IDs, capped by the current row/device limits. Selected i64 source fields must
fit u32; unsupported values are rejected. Native RadixRun's broader generic
domain is unchanged. This does not silently narrow native keys, automatically
replace packed MacRun, or change Datalog/maintenance placement. New GPU APIs are
macOS-specific. Dense membership uses the same native lazy bounded bitmap,
including its first-use initialization cost.

### Verification

The six MCCL grouped-index regressions passed normally and under Metal Shader
Validation. They cover exact grouping, spline bounds, missing keys, chunk seams,
full-width u32 values, masks/original IDs, selected signed-domain failures,
stable merges, context errors and ownership under cache churn. The three CQ
regressions additionally exercise the shared native reader, exact occurrence
order and retained cursors. All 87 storage tests, all 16 MCCL tests and all
15 CQ Mac tests passed during integration. Actual build/merge and prepared-read
executables validate full payload/group output outside timing.

Strict Clippy passed for MCCL and CQ Mac targets and the pinned storage library.
The broader storage test-target lint encountered existing `needless_borrow`
diagnostics in `provider_contract.rs` under the MCCL workspace's toolchain.
Query-expansion retains its unrelated `manual_h09` type-complexity and
`authored_advice` warnings. The native bundle retains its existing deployment
target warning. The reviewed storage public-API snapshot includes the borrowed
view and spline metadata exports.

### Final construction and prepared-read measurements

M3 Max, 128 GiB, AC power; no recorded thermal/performance warning. Shader
Validation was disabled for timing. The build sweep rotates three constructors
over one preingested source, with two warmups and five measured samples each.
Its native baseline is the existing **single-threaded**
`RadixRun<u32,u32>::from_pairs`, not an optimized multicore CPU implementation.
Completed constructor wall time includes allocation, GPU work and completion;
source ingestion, oracle preparation, validation and output destruction are
excluded and ingestion is reported separately.

| Rows | Input | Native CPU build ms | GPU RadixRun build ms | GPU merge with 10% delta ms |
|---:|---|---:|---:|---:|
| 1,048,576 | Dense, four occurrences/key | 27.576 | 2.491 | 1.070 |
| 1,048,576 | Full-width permuted unique keys | 48.215 | 1.684 | 1.259 |
| 1,048,576 | 32 occurrences/key | 18.403 | 1.705 | 0.873 |
| 16,777,216 | Dense, four occurrences/key | 627.999 | 10.684 | 4.633 |
| 16,777,216 | Full-width permuted unique keys | 987.041 | 11.027 | 10.056 |
| 16,777,216 | 32 occurrences/key | 420.905 | 8.379 | 3.130 |

The prepared-read sweep retains the earlier exact fixtures and adds actual
GPU-built RadixRun as a fifth provider: 360 records / 2,520 timed samples.
The same native CPU reader executes on both native and GPU-built grouped arrays.
Immediate lookup plus expansion, milliseconds, at 262,144 distinct keys,
65,536 probes and approximately 50% hits:

| Fixture | Packed GPU MacRun | Native RadixRun | GPU-built RadixRun |
|---|---:|---:|---:|
| Dense | 14.123 | 2.423 | 2.223 |
| Sparse | 4.190 | 2.454 | 2.509 |
| Skewed | 11.994 | 2.350 | 2.545 |
| Duplicate-heavy | 18.505 | 3.434 | 3.540 |

Across all 72 membership/immediate/retained workload pairs, the median
GPU-built/native read-time ratio is 1.035, ranging 0.877–1.387. This is not
universal parity or a statistical equivalence claim: large sparse membership
still takes 1.046 versus 0.754 ms, and sparse retained expansion takes 2.481
versus 1.970 ms. Independent chunking retains more spline points on non-collinear
spans. Exact coalescing reduces the dense model to two endpoints and removes the
earlier low-prefix skew penalty without weakening the bound. No measured
remaining gap is attributed solely to allocation, branch prediction or caches.

Raw current artifacts:

- [Completed construction/merge](../experiments/mac/query-expansion/results/radixrun-build-coalesced-m3max-20261027.json).
- [Prepared CPU reads](../experiments/mac/query-expansion/results/radixrun-prepared-coalesced-m3max-20261027.json).

The corresponding `radixrun-build-m3max-20261027.json` and
`radixrun-prepared-m3max-20261027.json` files retain the initial chunk-endpoint
version's measurements. Do not mix them with the final coalesced model.

```sh
cargo run --release --locked --manifest-path experiments/mac/query-expansion/Cargo.toml \
  --bin radix_build -- --sizes 65536,1048576,16777216 --runs 5 \
  --out /tmp/radixrun-build.json
cargo run --release --locked --manifest-path experiments/mac/query-expansion/Cargo.toml \
  --bin prepared -- --keys 16384,262144 --probes 65536 --runs 7 \
  --out /tmp/radixrun-prepared.json
```

These experiments demonstrate a completed fast GPU constructor with native-style
CPU reads and GPU merge, not automatic query adoption or an end-to-end H09
speedup. Tiny inputs still pay GPU submission cost; retained allocation
capacities, model size and broader key-domain support remain explicit tradeoffs.

## Primary-checkout consolidation and mainline sync (2026-10-27)

All active Mac implementation sources are now Git-managed in this checkout:

- `experiments/mac/mccl`: native MLX/Metal implementation and Rust ownership API.
- `experiments/mac/cq-mac`: CQ adapters, grouped RadixRun and maintained snapshots.
- `experiments/mac/gpu-index`: canonical-kernel diagnostics.
- `experiments/mac/query-expansion`: authored-query and matched build/read drivers.
- `runtime/cq-storage`: portable native storage and the shared RadixRun reader.

The Mac crates remain standalone Cargo workspaces outside the portable core.
Cargo metadata for all four Mac workspaces has no external path package.
The rebuilt `libmccl_native.dylib` resolves MCCL/MLX libraries inside
`experiments/mac/mccl/build/native`, not a sibling project. Historical source
paths embedded in archived measurement records are provenance and remain intact.
The oversized balanced matrix is preserved losslessly as
`index-matrix-balanced-m3max-20261026.json.gz`.

Mainline was fetched through `43d22720` and integrated with the local work.
The merge preserves the newer Tasks, closed-core and CUDA/Datalog work alongside
generic provider support and Mac indexing. Routed providers now implement the
same explicit `Keys` contract; native RadixRun also has the routed read adapter.
Catalog retention treats storage type and its declared support together while
independent checks still enforce all live obligations. Column identity remains
a Rust `ColumnSlot` obligation, not a provider-name blacklist.

Recovery points retain the pre-consolidation work:

- `backup/mac-before-unify-20261027`: original checkout's local work checkpoint.
- `refs/backups/gpu-fork-before-unify-20261027`: complete GPU snapshot source/evidence.
- `refs/backups/mccl-before-unify-20261027`: independent MCCL source/evidence.

Original sibling folders are untouched historical backups, not active build
dependencies. Development and verification continue only in this checkout.

Unified verification exercised the 1,066-test portable workspace, 187 rewrite
tests, 11 join-filter tests, 54 Tasks tests, 52 workload tests and 25 xtask tests.
The local MCCL and CQ Mac suites passed 16 and 15 tests. The actual canonical
Metal kernel executable, grouped GPU construction/merge driver and H03/H09
boundary/scaled executions passed exact checks. All five compiled native
RadixRun example routes returned the same 150 answers. These are integration
smokes, not new performance measurements; CUDA device execution was not exercised
on this Apple-silicon host.
