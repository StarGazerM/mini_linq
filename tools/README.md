# Regression gate protocol

## Iteration and final gate

Use the pinned toolchain and keep heavy builds bounded:

```sh
export CARGO_INCREMENTAL=0
export CARGO_PROFILE_DEV_DEBUG=line-tables-only CARGO_PROFILE_TEST_DEBUG=line-tables-only
export CARGO_BUILD_JOBS=6
# Quick tier: 15 bounded compiler/runtime/architecture contracts.
nice -n 10 bash tools/test_regressions.sh --tier quick
# Or just the named regression(s) covering the change.
nice -n 10 bash tools/test_regressions.sh mini-linq/physical_query
# Inspect the final selection, including each reason.
python3 tools/test_regressions.py --changed-since BASE_REV --list
# Final behavioral gate: every potentially affected inventory target.
nice -n 10 bash tools/test_regressions.sh --changed-since BASE --reuse-latest target/baseline-gate
# Main also gates packaged consumers outside the checkout once per merge batch.
nice -n 10 python3 tools/check_external.py
# Full inventory remains the default.
nice -n 10 bash tools/test_regressions.sh
```

The root workspace shares build artifacts between the portable compiler, feature
consumers, documentation driver and CPU benchmarks. The behavioral gate is the
regression suite plus root `cargo test`; do not additionally sweep every consumer
package and repeat the same tests through the inventory. Use `cargo test -p NAME`
for changed-package coverage absent from the inventory. `cargo test --workspace`
unifies instrumentation features across consumers and is **not a gate**.
Use root `cargo test` (the non-instrumented default members), plus the explicit
package regression rows for counter-enabled packages. CI follows that selection;
its separate Clippy step retains the original core/course scope.
A track's gate is its changed crates, the tests covering its fix, the quick tier,
and any named downstream target it affects. Main runs the impact-selected suite
once per merge batch with `--changed-since BASE --reuse-latest target/baseline-gate`.
Main additionally runs `python3 tools/check_external.py` once per merge batch:
in-tree success must not hide broken packaged API consumers or stale external
acceptance fixtures.
Run separate structural, API, formatting or documentation gates only where the change
requires them. Do not rerun behavioral gates for a docs-only rebase.
An explicitly named selector narrows even an impact-selected run; omit selectors
for the final gate. Unknown selectors are errors, not successful empty gates.

**Merge protocol (2026-11-06).** Before Main merges a branch that changes a
compiler pass, checker, printer, runtime or planner: (1) the branch's census
lists every recorded plan it moves (old/new/reason) and every generated-code
target it changes, and its realization-level review (providers, phases,
scans, probes per row) shows only intended changes; (2) an independent
reviewer reads the diff against the contract (one decision per IR, printers
decide nothing, checkers verify without rerunning, no type/syntax
inspection, no avoidable allocation) and the census, and Main reads the
expanded code of every changed target itself; (3) the gates above. A branch
whose generated code changes in kind where its census predicted only a
reorder is sent back, not merged with the change explained after the fact.
Scope (P7, design-notes/ir-redesign.md): a branch declares the IR it owns
and runs `python3 tools/check_ir_scope.py --owner <cq-lang-crate> [--allow
<renamed path>]` after `check_public_api.py --update`; in every other IR
crate it may delete or rename but not add or change grammar. An order-only
change adds nothing below VO. Seam data a checker needs is a side table the
lowering returns, never an IR field.
Benchmark datasets outside the repository (read-only, exact paths only, never searched): TPC-H GPU fixtures `/data/tpch-gpu/sf1` and `/data/tpch-gpu/sf10` (manifests/dictionaries by exact filename), DOOP xalan `/data/doop-bench/doop-data/xalan`, prepared field_flow `/data/doop-bench/doop-vs-srdatalog-20261026/fresh-corpus/prepared`.

Lock scope (enforced by review): a measurement driver takes the CPU-exclusive timing lock around ONE sample at a time and releases it between samples; a GPU campaign holds only the GPU lock and releases it between kernels. Holding any exclusive lock across a whole campaign blocks every build on the machine.

**Development style (2026-11-07).** (1) One IR per branch, small: a step is
mergeable in hours, not days; split otherwise. (2) Rules before code: the
`\Rule` for every decision or refusal the step introduces is written in
doc/formal first, with a mathematical premise, and the implementing code
carries its `Formal rule NAME:` marker one-to-one; a contract exists only as
a checker that rejects or as a type (a lowering is total on well-formed
input). (3) Deletion-only outside the owner, and an order-only change adds no
grammar anywhere (`check_ir_scope.py`). (4) Tests are expected-output
fixtures of the owner IR (input program -> expected printed program) plus
checker refusals; never wording, never downstream. (5) Seam data is a side
table returned by the lowering. (6) Read before you measure applies to merge
batches, where the full gates and the baseline run once.
Per-step gate for a single-IR change (owner declared, scope check zero):
the owner crate's tests with expected-output fixtures (a step that changes the stage protocol, Structures or Sequence APIs also runs the GPU host-side checks: `cargo build --lib -p cq-gpu-tpch` (its build script generates and checks all advised queries; binaries and lib tests need oxide finalization and run only in gpu_gate.sh) and the GPU Datalog/Sequence crates' host tests) (input program ->
expected printed program, and the checker), the IR-level census (re-lower
every recorded plan and compare printed plans against the base; no
codegen), and `check_ir_scope.py`. Nothing else: a later layer cannot have
changed. Expanded gate, root/package suites, compile-tpch and the GPU gate
run once per merge batch, by Main, and the batch includes `cargo test -p cq-workload-corpus` (its registrations follow the current physical plans, so an upstream policy change can make a registered configuration ineligible).
After each merge batch Main runs the performance baseline: CPU TPC-H SF1
default/par, CPU DOOP xalan mini/mini-par, and (GPU available) GPU TPC-H SF1
advised and `doop-gpu` xalan, paired against the previous batch's main under
the timing protocol, so a regression is attributed to one batch.


The regression inventory includes GPU compiler host-only tests, but performs no
GPU device execution. The constant-capture test lives with the GPU compiler;
core has no GPU dev-dependency. When a merge batch can affect
GPU code (any shared compiler crate, `experiments/gpu`, or the Datalog
pipelines), Main also runs `bash tools/gpu_gate.sh` once for the batch: the GPU
Datalog, Sequence, TPC-H, `cq-gpu-runtime` and `gpu-device-columns` crate suites, a separate untimed
`gpu-datalog-runtime` suite with `--features gpu-datalog-runtime/radix-counters`,
the advised H05 SF10 benchmark (including its existing 10.35 ms ceiling), the
`doop-gpu`/`srd-doop-gpu` builds, and `doop-gpu` on Cassandra, which compares all
26 complete output tuple sets against `doop-mini`. The remote bundle preserves
the same default-feature and counter-enabled test steps as distinct harnesses;
counter instrumentation is not enabled for benchmark binaries.
It holds `/tmp/mini_linq-gpu.lock` for the whole run. H05 timing and exactness
evidence is written to `target/gpu-gate-h05-sf10.json`; a ceiling failure fails
the batch even when every answer is exact.
The H05 step selects the executable with `oxide run --bin cq-gpu-tpch`;
arguments after `--` belong to the executable (output JSON, query, scale, mode),
not to Cargo. In particular, Cargo's `--release -p` flags cannot follow that
separator.

Builds, tests and timings share one machine through two lock files:

```sh
# Builds and tests: ONE cargo/target command per shared-lock acquisition.
# Multi-command runners release and reacquire between commands.
flock -s /tmp/mini_linq-timing.lock CMD
# Timing measurements: exclusive. Holding the gate blocks new builds, so a
# measurement cannot be starved by a stream of shared holders.
flock /tmp/mini_linq-timing-gate.lock flock -x /tmp/mini_linq-timing.lock CMD
```

Take the shared lock around ONE command (one cargo build/test, one target's
verification), never around a runner that executes many commands: a waiting
measurement holds the gate, so a long shared holder stalls every build behind
it (2026-11-03: a 30-minute expanded gate under one shared lock queued all
agents while CPU and GPU sat idle). Multi-command runners re-acquire the shared
lock per command.

GPU execution holds `/tmp/mini_linq-gpu.lock`. GPU timing holds only the GPU
lock, not the CPU timing locks: samples interleave A/B in rotating order and
record load average, which controls for background builds. Check for foreign
tenants before each sample and discard contaminated samples. A GPU claim whose
CI lies within 5% of 1 is rerun with the CPU exclusive lock taken after the GPU
lock. CPU timing holds the exclusive lock only for the measurement itself.

**Read before you measure.** Before building a benchmark or timing anything,
print the recorded plans (checked VO, Structures, Sequence, GPU realization)
and read the expanded Rust of every kernel or rule the change touches, for both
the baseline and the candidate. Write down what changed and why each change is
right given the other planning decisions (evidence roots, L&E phases, Masks,
join filters, blocked scans, admission/canon, providers). Measure only after that
review finds nothing wrong; a timing result on an unreviewed plan carries no
information. Plan-level changes (join order, delta plans, reductions, filters)
are produced by the planner from recorded evidence together with the other
join-planning decisions, never imported or authored in isolation and measured.

Verification follows what a change can break: docs-only changes run nothing;
test tooling runs its own unit tests and `python3 tools/check_external.py`;
an oracle/infrastructure swap checks only that the oracle is unchanged;
compiler/runtime changes run the track gate above.
Every agent and track report states its gate time: total wall-clock spent
on gates, tests and benchmarks since the previous report, per command, plus time
waiting on the GPU/timing locks, measured (runner timing columns or
`/usr/bin/time -f %e`), never estimated.

Impact selection compares tracked working-tree changes, deletions, renames and
nonignored untracked files with the supplied revision. It maps files to local
packages and resolves each actual test package's dependency closure using offline,
locked Cargo metadata, including dev/build/proc-macro dependencies. Sharing a
workspace alone does not select a target. A scenario or manifest command that
actually executes several packages keeps that executed-package union. Historical
local package dependency edges are also
read from lockfiles for `--changed-until END_REV` comparisons. Missing metadata,
unknown ownership and configuration changes select conservatively. Repository-wide
architecture scenarios run on code/configuration changes. `doc/**`, Markdown and
changelog-only changes select no behavioral targets. A proven-empty selection is
a successful gate; uncertainty is never treated as an unaffected target.

## Timing and scheduling

### Timing-sample answer validation

Timing uses the shared host-only `testkit/bench-check` library/CLI; Python
controllers use the transport in `tools/bench_check.py`. Build the CLI with
`cargo build --release -p bench-check` under the build locks above. Remote
benchmark bundles include it; `BENCH_CHECK_BIN` selects an explicit executable.

The **first measured sample of each binary/configuration** receives a complete
exact comparison. A warmup or separate correctness command does not substitute
for that first sample. Every later sample must match the exact count of **each**
output relation/query and check up to **1,000 seeded, uniformly selected actual
tuples per relation** for membership in the reference (all tuples if smaller).
Relation names and empty relations are checked too. Counts preserve the existing
logical set semantics; an arbitrary bag length is not a distinct count.
Sampling is not a full exactness proof and is never reported as one.

References are indexed once per immutable dataset/query identity in the
controller, or once outside a native worker's sample loop. A controller retains
first-full state across fresh processes and keys it by binary identity, dataset,
query, engine/plan, threads and storage configuration. The native library owns
the stable sampling algorithm; Python does not rebuild full tuple sets for later
samples. Producers emit full-first or compact count/sample witnesses. External
SRDatalog currently exposes only full TSV export: its adapter streams that export
through the native checker and still reports the export cost separately.

Record verification mode, seed, per-relation counts/sample sizes, reference and
first-full provenance, and validation seconds. Reference preparation,
canonicalization, checking, serialization and destruction remain outside the
existing query timers, but count toward command wall time. Keep interleaved
fresh-process A/B pairs; do not replace them with a persistent query worker.
`BENCH_CHECK_MODE`, `BENCH_CHECK_SEED`, `BENCH_CHECK_SIZE` and
`BENCH_CHECK_SAMPLE` are controller-to-worker timing protocol fields, not a way
to bypass a correctness gate.
Raw/prepared DOOP timing producers write a full or sampled semantic witness to
`BENCH_CHECK_WITNESS`; raw GPU DOOP can separately export its full CPU reference
with `BENCH_CHECK_REFERENCE`. GPU TPC-H uses `BENCH_CHECK_WITNESS_DIR`, with files
`sfSCALE/MODE/QUERY-LOCAL_ORDINAL.json` to keep scales and modes distinct.
Its schema-3 results retain per-sample `check` evidence and distinguish
`validated` from the stronger all-samples-full `exact` flag. CPU workers retain
`check_policy`, `warm_check` and execution-ordered `samples_checks`; their
validation durations include typed canonicalization and selected-row formatting.

This policy applies to the CPU TPC-H plain/profile workers, raw and prepared
DOOP timing harnesses, GPU TPC-H timing loop and paired pod driver. Tests,
explicit correctness commands, independently counted checks and conformance
gates keep complete comparisons. Mac, compiled-triangle and storage
microbenchmarks retain their current full checks pending separate migration.
Dated benchmark reports describe the policy used when recorded; their historical
full-exactness claims are not evidence of sampling or of current-run validation.

Dataset loading is a separate concern. CPU TPC-H workers already load once per
process. Process-wide immutable test fixtures, a prepared mapped P5/S2 columnar
cache and context-owned GPU base-column residency are proposals, not implemented
by this timing-check cutover. Loader benchmarks must continue measuring real
fresh loads; never hide `Dataset::clone` or remove loading from an upload-inclusive
metric and call it a cache speedup.

Measured checkpoint (2026-11-03, shared-data worktree): real SF1 default H03
validated 11,620 rows in 2.096 ms for the first full sample, then 0.605/0.677 ms
for two 1,000-tuple samples. H13 validated 150,000 rows in 26.472 ms, then
4.052/4.020 ms. These phases include typed canonicalization and formatting;
they are full-versus-sampled phase observations, not old/new query speedups.
Worker evidence is under `features/workloads/target/bench/runs/sf-1/`.

Fixture-cache admission was measured separately: `kernel_written_coarse` spent
0.148373 s in 336 dataset parses; `sequence_conformance_kernel_written_coarse`
spent 0.065304 s in 156 parses. Extrapolating their means to 6,018 avoided
parses gives **2.617 s of sequential parsing work**, below the approximately
30 s gate-wall threshold. The OnceLock registry was therefore not implemented;
temporary instrumentation was removed. Raw logs and command/lock timing ledger
are in `target/shared-data-gates/`. Root, five counters packages, quick, formal,
API, complete TPC-H catalog compilation, GPU-workspace compile-only and the two
real SF1 benchmark commands passed; GPU device execution is not claimed.

Every run writes a new `target/regression-logs/RUN-ID/` directory, or the fresh
location passed to `--output`. `results.tsv` retains package/kind/target/status/log
and adds `build_seconds`, `test_seconds`, `wall_seconds`, and `outcome`.
`evidence.json` records the selection, run ID and per-target records. Existing run
results are never overwritten.

A transparent Cargo shim preserves command arguments, output and exit status.
Build time runs until Cargo's `Finished ... profile` boundary; Cargo build/check/
expansion-only commands count entirely as build time. Nested Cargo build intervals
are unioned, not double-counted. Test time is the remaining target wall time,
including scenario Python work, Cargo dispatch and any doctest work after the
initial build boundary. Quiet commands without a boundary are conservatively
counted as build time. Identity acquisition, when requested, counts as build time.
This is a wall-time split, not summed CPU time or a profiler of individual rustc
phases. End-of-run totals sum target times; elapsed wall time also includes metadata
and scheduling and can be smaller because independent targets overlap.

`--jobs` limits independently schedulable targets (default 2). Targets sharing a
workspace or effective Cargo target directory never overlap. `CARGO_BUILD_JOBS`
limits compiler parallelism within each Cargo process. `--target-dirs workspace`
preserves existing Cargo configuration; `--target-dirs shared` explicitly uses
`target/regression-shared/` for the entire worktree. An explicit `CARGO_TARGET_DIR`
takes precedence and is normalized to an absolute path for nested scenarios.

## Explicit expansion-identity reuse

```sh
# Record identities and save a fully passing baseline under HEAD's 12-digit SHA.
nice -n 10 bash tools/test_regressions.sh --save-baseline target/baseline-gate
# Main's default batch command discovers the newest baseline automatically.
nice -n 10 bash tools/test_regressions.sh --changed-since BASE --reuse-latest target/baseline-gate
# Preview actual identity-checked reuse decisions without executing test targets.
nice -n 10 bash tools/test_regressions.sh --changed-since BASE --reuse-latest target/baseline-gate --list
```

Reuse is the default in Main's batch command above, but remains opt-in for other runs.
`--reuse-latest DIR` recursively chooses the `evidence.json` with newest modification
time (path breaks ties), including `DIR/*/evidence.json`; it is mutually exclusive
with `--reuse-from FILE`. An empty/missing directory means normal execution.
Malformed evidence is an error; failed, unrecorded, reused or damaged entries are
never accepted. `--list` reports `REUSE` or `EXECUTE` after checking current
identities; expansion checks can compile even though test targets are not executed.
`--save-baseline DIR` enables identity recording and copies the complete evidence
directory only for a nonempty, fully passing run to `DIR/<12-digit HEAD SHA>`.
Existing baseline directories are never overwritten, and failed runs are not saved.
The baseline destination must be outside the run's log directory (`--output`),
including after resolving symlinks; copying a run into itself is rejected.
A selected target is marked `reused BASELINE-RUN-ID` only when a
recorded, actually executed passing target has an intact hashed log and its current
identity is equal. Identities use actual expanded Rust normalized by
`expanded_gate.py`, linked runtime source hashes, unchanged target/test inputs,
resolved dependencies/features, toolchain and build configuration. Runtime-crate
or test/input changes invalidate reuse even if the emitted query is identical.
The record is discarded if inputs change during execution. Reused records cannot
be used to build a chain of new attestations. Unsupported or unproven expansion
modes, missing evidence or any identity failure execute normally; an expansion by
itself is never proof that a test passed. `--record-identities` is optional because
recording identities has a real expansion cost. Markdown/changelogs and `doc/**`
are excluded from expansion source/runtime keys, not from executable Rust bodies.

Every manifest inventory command explicitly selects its package with `-p`; moving
a package into the root workspace never implicitly expands a row to unrelated
workspace members. Rows formerly testing several default members now enumerate
those packages separately, preserving their tests. Named feature-specific tests
use `manifest-feature-test` and `TEST@FEATURE` (`@all-features` selects
`--all-features`). Execution, impact selection and identity expansion use the same
package, test and feature flags.

## Performance comparison protocol

Compare one warmed workspace-layout run with one warmed shared-layout run on the
same source snapshot, preserving the same target set, job limit and environment.
Record start/end load averages on shared hosts, and do not claim a cold cache is
warm. Change the runner's default only after an observed wall-time improvement.

Before consolidation, feature-local Cargo configuration shared `features/target`
only for commands whose working directory discovered it; most inventory commands
used distinct workspace targets. The root now owns the portable build directory
regardless of which member manifest selects the package. Nested feature target
overrides and lockfiles are removed. Workload semantic benchmark/data caches are
still independent of Cargo's build directory. The preexisting workload corpus/e2e
`debug=0` settings are retained, not credited as a new optimization.

## Workspace ownership

- **Root:** core, all portable `features/` packages, the formal example, the
  correspondence-performance fixture and native CPU benchmarks. One lockfile
  resolves their compatible dependency constraints; workspace membership is not
  an architectural dependency edge. `check_dependencies.py` still rejects every
  core-to-feature dependency, including build, dev, optional and renamed edges.
- **`experiments/gpu`:** the independent pinned Rust/oxide/CUDA toolchain,
  dependency setup and GPU build profiles remain unchanged.
- **`experiments/mac`:** Metal/framework-dependent consumers share one workspace.
  They cannot build on the Linux CPU gate host. The portable native index
  benchmark is a root member even though its source remains under `mac/`.
- **External packaging fixtures:** standalone positive, negative and sibling
  workspaces are deliberate input to the relocation/isolation tests, not
  production build groups. The external gate reconstructs consumer-only
  workspaces outside the checkout and substitutes Cargo-packaged core crates.

Root `default-members` contains 114 of the 119 packages (including the host-only
benchmark checker), excluding these five whose
**dev-dependencies** explicitly enable `cq-storage/counters`:
`mini-linq`, `cq-load`, `own-dialect`, `cq-rewrites`, and `cq-sequence-tasks`.
No normal dependency or default feature enables another instrumentation feature.
Their package-wide regression rows use `-p`, preserving instrumented coverage
without changing the feature sets of unrelated generated query tests. Do not
replace this split with `--workspace`, a larger global thread stack, or altered
query optimization profiles.
The `architecture/workspace-split` regression scenario and CI both run
`tools/test_check_dependencies.py`, including the default-member split invariant.
As a repository-wide architecture scenario, it is selected on code/configuration
changes, including the root `Cargo.toml`.

For `cq-workload-corpus`, `cargo tree -e features -i cq-storage` with explicit
package/member selection is identical before and after consolidation after
normalizing only the checkout path. Neither tree enables counters, and the
current root default-member tree also contains no counters feature.
Evidence: `/tmp/ws-consolidate-feature-trees/comparison.json` (normalized tree
SHA-256 `7adf78f8af1d1c2c2bf880d933f422e1d47568e0d1982fb04dc0ee9d061ba95f`).

Root build-time compiler/proc-macro dependencies use the existing optimized
consumer build setting (`opt-level=3`). Workload binary debug overrides are kept.
CPU storage/native-index benchmarks retain thin LTO and one codegen unit through
`--profile cpu-bench`, without changing other root release binaries' LTO settings.
Package versions, editions and MSRVs are retained when inherited settings differ.
GPU and Metal runs remain separate; a shared target directory is not a substitute
for their toolchain and platform boundaries.

## Consolidation measurements (2026-11-03)

Measured in `/data/mini_linq-wt/ws-consolidate` using the environment above.
Each cold quick run starts with empty worktree build directories; its warm run
immediately repeats the same 15 targets. Both final before/after pairs passed
15/15. These short measurements held the exclusive timing lock; lock wait is
excluded from gate wall time. Disk is allocated bytes summed across worktree
`target/` trees, with no preexisting benchmark data in this worktree.

| Layout / cache | Gate wall (s) | Summed build (s) | Target bytes |
| --- | ---: | ---: | ---: |
| Before / cold | 101.389 | 117.755 | 1,500,934,144 |
| Before / warm | 6.157 | 0.725 | 1,500,934,144 |
| Consolidated / cold | 106.269 | 105.302 | 1,066,496,000 |
| Consolidated / warm | 8.330 | 1.074 | 1,066,496,000 |

Quick-tier wall time did **not** improve: cold was 4.8% slower and warm 35.3%
slower. Cold summed build time fell 10.6%, and disk fell 28.9%. These results
must not be described as a quick-tier speedup. Before/after lock waits were
1,150.573 s and 295.130 s respectively; `--list` took 0.039 s in each layout.
Evidence: `/tmp/ws-consolidate-before/` and
`/tmp/ws-consolidate-after-final/` (`measurements.json`, per-run `results.tsv`).

The separate cold full-baseline attempt was interrupted by the coordinator to
avoid monopolizing the timing lock; it has no completed suite totals. The
approved existing full reference is the integration run at `44bf82bc`:
2,189 s wall (`--jobs 3`), 3,588.020 summed build seconds and 385.451 summed
test seconds, 123/124 passing. Its planner fixture failure predates this change.
Evidence: `/data/mini_linq-wt/integ/target/regression-logs/20261103T042755Z-1cdb584b/results.tsv`.
This different-revision reference is not a controlled cold/warm comparison.

The pre-rebase consolidation inventory (`--jobs 3`) passed **132/132**, with no reuse:
**1,706.434 s gate wall**, **1,498.500 s summed build**, and **351.001 s summed
test** (runner wall 1,706.378 s; lock wait 303.484 s). It followed the other
verification gates and used the shared build/test lock, so this is not a cold
measurement or an isolated performance comparison with the historical reference.
Evidence: `/tmp/ws-consolidate-full-after/{results.tsv,evidence.json}`.

Final worktree build trees used **24,736,546,816 bytes**: root `target/`
23,680,925,696 bytes plus GPU host-test artifacts 1,055,621,120 bytes. This total
includes preceding core, API, isolation and external-consumer caches. No matching
completed full-baseline disk measurement exists; only the quick disk pair above
is a controlled before/after comparison.

| Additional gate | Gate time (s) | Lock wait (s) |
| --- | ---: | ---: |
| Tooling unit tests | 0.916 | 96.700 |
| Original root core/course package test set | 59.173 | 74.194 |
| Dependency checks + isolated `mini-linq` tests | 42.740 | 71.139 |
| Formal correspondence inventory | 0.315 | 130.180 |
| Public API snapshots | 117.569 | 123.511 |
| GPU compiler host tests, Rust 1.99.1 release | 112.377 | 50.300 |
| Packaged sequence-rewrites + mandatory positive/negative fixtures | 53.107 | 191.870 |

All listed gates passed. The packaged check is a selected consumer gate, not a
claim that every external feature group was executed. Full command records,
including superseded failed attempts and H01 compilation steps, are retained in
`/tmp/ws-consolidate-results.json`; per-command logs are in
`/tmp/ws-consolidate-gates/`. No GPU device execution was performed.

The actual retail H01 release worker was built before and after applying the
manifest/lock/config cutover to a same-path archive of the original source.
Its benchmark-owned release profile (`build-override.opt-level=3`) was retained.
The generated consumer lock, expanded Rust, LLVM IR and linked executable were
all **byte-identical**, without normalizing symbols, paths or instructions:

| H01 artifact | Bytes | SHA-256 (both sides) |
| --- | ---: | --- |
| Expanded Rust | 170,754 | `cc2de4dcd4b5d8c71e4c8484d6367eda4920a05741bcdc4aa0088fff09039865` |
| LLVM IR | 11,529,975 | `3cf1adcc532be7911bf26855caf453b8d414202e6b04aa9cdd057371519d36a5` |
| Release executable | 9,178,464 | `7179c30b5581d9aa657695a1899849cf066ad908045598b7bedf36ee4639cca6` |

This comparison caught an unwanted `serde_json` update from 1.0.151 to 1.0.154;
the portable lock retains 1.0.151. Evidence:
`/tmp/ws-consolidate-h01/comparison.json`. This is compiled-output preservation,
not a new runtime-performance or query-execution measurement.

## Post-rebase default-member gates

After rebasing onto the newer rejection tests and key-partition fix, the accepted
gate is root `cargo test` plus explicit instrumented-package rows, not
`cargo test --workspace`. The temporary recursion-limit adjustments were removed;
no global stack setting or per-test stack wrapper was added.

| Final gate | Result | Gate time (s) | Lock wait (s) |
| --- | --- | ---: | ---: |
| Tooling unit tests | 124 passed | 2.169 | 68.246 |
| Dependency checks | passed | 21.255 | 249.931 |
| Root `cargo test --locked`, 113 default members | passed | 1,054.913 | 432.384 |
| Five explicit instrumented-package regression rows | 5/5 passed | 177.072 | 295.095 |
| Quick tier | 15/15 passed | 25.010 | 1,237.104 |
| Full packaged external-consumer gate | all 15 groups and mandatory fixtures passed | 1,323.516 | 787.169 |

The root-selected KP-1/KP-2 regression also passed (11.434 s gate, 100.270 s
lock wait). Feature-tree comparison passed in 8.236 s, excluding 857.417 s lock
wait. These shared-lock completion gates are not cold/warm timing experiments.
Final requested gates total 2,603.935 s, with 3,069.930 s summed lock waits.
Complete command records and superseded failures:
`/tmp/ws-consolidate-rebase-results.json`.

## Measured quick tier

Historical measurements below are retained from committed documentation:
`7e187597` (2026-11-02) records the expanded quick tier and its timings;
`f82091ce` (2026-11-01) records the impact-selection, layout and reuse
experiments. These are checkpoint observations, not current-run results.
Their `target/` paths identify original local evidence, not committed result
artifacts; raw logs have not been revalidated by this documentation audit.

The 15-target quick tier passed in **21.939 seconds** (external gate time
**21.98 seconds**): 21.383 summed build seconds and 6.841 summed test/scenario
seconds. Build settings were the environment above with workspace target directories.
Evidence: `target/regression-logs/20261102T080342Z-c3d8ecfd/results.tsv`.

Three named Datalog targets are now in both the default inventory and quick tier:

| Target | Cargo test selection | Warm wall time |
| --- | --- | --- |
| `cq-datalog/execution` | `--test execution` (default features) | 0.103 s |
| `cq-datalog/execution@all-features` | `-p cq-datalog --test execution --all-features` | 0.104 s |
| `cq-datalog/counters@counters` | `-p cq-datalog --test counters --features counters` | 0.184 s |

Each is below the approximately 30-second warm cutoff. The three-target warm
gate took **0.51 seconds**; evidence:
`target/regression-logs/20261102T080317Z-cae1a4ae/results.tsv`.
The initial build/warmup gate took 98.21 seconds and is not claimed as warm.
`--tier quick --list` prints the complete inventory: these three Datalog targets,
storage access/representation/counter laws, index demands, macro inputs, print
expressions, CQ program/normal/guard tests, and the three architecture scenarios.

## Impact-selection measurements

Selection was evaluated against the real commits using package-level Cargo
closures. These are **selection** timings, not three repeated suite executions:

| Change range | Selected / inventory | Selection wall | 1-minute load before → after |
| --- | ---: | ---: | ---: |
| `17a14b93..c82e9abf` | 84 / 119 | 7.621 s | 28.08 → 27.84 |
| `c82e9abf..6b7b1097` | 119 / 119 | 7.753 s | 27.84 → 26.65 |
| `c0f79500^..c0f79500` | 85 / 119 | 7.329 s | 26.65 → 27.86 |

The second range also changes public-API snapshot inputs outside a crate, so its
unknown-input fallback conservatively selects every target. Exact selections and
reasons are retained in
`target/regression-measurements/impact/*-package-closure.log`.

A synthetic changed-path input restricted to
`features/datalog/passes/fork/src/lib.rs`, resolved with actual Cargo metadata,
selected 22 Datalog/DOOP targets and the three repository-wide architecture
scenarios in 8.892 s (load 26.24 → 28.64). It selected **zero root Rust or CPU
workload targets**, and no unrelated Ascent engines. No source file was changed
for this isolated changed-path experiment; existing tooling edits were not
misrepresented as Datalog-only changes. The exact subset/reasons are in
`target/regression-measurements/impact/datalog-only.json`.

## Opt-in fuzz registry

The optional fifth column in `regression-targets.tsv` is `fuzz`; omitted means
deterministic/default. The command kind remains unchanged. Fuzz-tagged rows run
only through `--tier fuzz` or an exact selector, never implicitly through the
default, quick or `--changed-since` gate:

```sh
bash tools/test_regressions.sh --tier fuzz
bash tools/test_regressions.sh cq-xtask/fuzz-quick
```

The old `cq-xtask/fuzz` oracle row is replaced by `cq-xtask/fuzz-quick` (64
fixed-seed cases × 4 CPU routes) and `cq-xtask/fuzz-full` (2,000 cases, rotating
logged seed, every CPU route), both in this opt-in tier. `cq-rewrites/rewrite_stages`
runs the generated stage differential and checker mutants through the six Physical
rewrites; its Cargo target is `test = false`, so only an explicit `--test` runs
it. The consolidated default inventory has **137 deterministic targets**, and
140 rows including fuzz. Seven explicit helper-package rows preserve tests
previously selected through nested workspace default members; one host-only GPU
compiler row preserves the constant-capture test relocated from the core. The historical
impact/quick measurements above predate these inventory additions and tier
migrations. Test-layer additions must keep randomly generated consumers out
of ordinary `cargo test` and the quick/default suites.

The runner's kinds cannot pass `--ignored`, so the core-pipeline generated
sweeps (through Physical lowering) are explicit commands rather than rows:

```sh
cargo test -p mini-linq --test stage_differential -- --ignored
cargo test -p mini-linq --test checker_mutants -- --include-ignored
cargo test --manifest-path features/datalog/oracle/Cargo.toml --test random_programs random_programs_quick -- --ignored
cargo test -p cq-testkit --features duckdb -- --ignored
cargo run --manifest-path experiments/gpu/Cargo.toml -p cq-gpu-fuzz -- SEED CASES
```

The GPU lane needs CUDA and stays a manual command. The harness takes the GPU
lock itself; do not wrap it in `flock`. Generated consumers build through
`cargo +1.99.1 oxide build --arch sm_89 --materialize-cubin -- --release`,
then execute under the same GPU lock. CQ compares both the CPU Physical route
and explicitly downloaded GPU rows with the source oracle. Datalog is attempted
even when CQ fails, and each lane reports its result, generated source directory,
and gate time before the harness returns failure if either lane failed.

## Warm shared-directory indicator

Historical checkpoint: `f82091ce` (2026-11-01), as retained in this file.

After warming each layout, the identical 12-target quick tier passed in both:

| Layout | Wall | Summed build | Summed test | 1-minute load before → after |
| --- | ---: | ---: | ---: | ---: |
| Existing workspace directories | 7.945 s | 1.016 s | 9.078 s | 18.69 → 19.41 |
| One worktree-local shared directory | 8.487 s | 0.958 s | 9.606 s | 19.41 → 19.14 |

Order was workspace warm-up, shared warm-up, workspace measured, shared measured.
All four runs passed 12/12. To honor the temporary prohibition on `--all-features`
and DuckDB builds, this experiment listed the exact quick-package features
explicitly: storage `counters,operator-timing`; the other quick packages have no
features. Thus quick-tier feature coverage was unchanged, and no DuckDB was built.
Evidence and load averages: `target/regression-measurements/quick-layouts/summary.json`.

This small indicator shows **no wall-time improvement**, and does not measure
cross-workspace generated-query savings. Shared target directories remain opt-in;
no default or test-profile settings were changed. The full-suite warm A/B and
large-generated-crate codegen/debuginfo measurements are explicitly deferred.

## Reuse demonstration and remaining gate evidence

Historical checkpoint: `f82091ce` (2026-11-01), as retained in this file.
The temporary build prohibitions and pending gates below describe that
experiment, not today's gate policy.

The requested full baseline was interrupted immediately when DuckDB and
`--all-features` builds were prohibited. Before interruption, 72 targets completed
and all 72 passed. This is **not** a complete passing suite. Their logs remain in
`target/regression-measurements/baseline-identity/`; that interrupted directory has
no complete `evidence.json` and is not presented as reusable baseline evidence.

A DuckDB-free composition target then demonstrated the complete reuse protocol:

| Run | Result | Wall | Build | Test | 1-minute load before → after |
| --- | --- | ---: | ---: | ---: | ---: |
| Recorded baseline | 1 executed PASS | 51.612 s | 51.511 s | 0.040 s | 18.52 → 17.11 |
| Compiler-only comment in `passes/cq-print/src/lib.rs` | 1 reused, 0 executed | 8.094 s | 8.057 s | <0.001 s | 15.93 → 15.67 |

The comment changed the compiler crate's source bytes but not its emitted Rust.
The runner validated the baseline's hashed passing log, re-established matching
expansion/runtime/test inputs, and recorded
`reused 20261102T042129Z-1ca287ac`. The temporary comment was removed immediately
afterward. Evidence is in `target/regression-measurements/reuse-safe-baseline/`
and `reuse-safe-working/`. Baseline time includes initially populating the isolated
identity build directory; the ratio must not be extrapolated to a warm full suite.

Thus **1 target is experimentally proven reusable**, not 118 or 119. A complete
passing baseline and full-suite reuse count remain pending until the DuckDB CLI
cutover allows the affected gates. Unsupported doctest/dynamic-compiler/native
inputs still execute normally rather than claiming expansion proof. Final tooling
checks passed: 68 regression-runner/impact/identity/timing tests and 14
expanded-gate tests.

## Execution-only remote GPU gates

`remote_gpu/` separates workstation builds from rented GPU execution. Nothing in
`bundle.sh`, `make_data_manifest.sh`, or local verification provisions a pod.
Only `session.sh` creates a billable resource; invoke it explicitly after the
private dataset upload and bundle builds have finished.

```sh
wt=/data/mini_linq-wt/remote-gpu
bash "$wt/tools/remote_gpu/bundle.sh" "$wt" "$wt/target/remote-gpu/gate.tar.zst" \
  --bench cq-gpu-tpch doop-gpu srd-doop-gpu
# Once, before uploading the dataset (or reuse an already generated SHA256SUMS):
bash "$wt/tools/remote_gpu/make_data_manifest.sh" "$wt/target/remote-gpu/SHA256SUMS" /data
```

The builder pins `cargo +1.99.1`, `sm_89`, release mode, build jobs 6 and the
documented HPC SDK environment. It takes the timing gate/shared timing locks and
uses `nice -n 10`; it never takes the GPU lock or runs a built executable.
`oxide test --no-run --message-format=json` enumerates exact test harnesses;
`oxide build --message-format=json` enumerates the gate binaries and additional
`--bench` binary names. Rustdoc's separate `--no-run --persist-doctests` path
compiles doctests without execution. Compile-fail checks happen locally.

For a benchmark-only revision whose tests need not be built, add `--no-tests`:

```sh
bash "$wt/tools/remote_gpu/bundle.sh" /path/to/reference-worktree \
  "$wt/target/remote-gpu/reference-bench.tar.zst" --no-tests \
  --bench cq-gpu-tpch doop-gpu srd-doop-gpu
```

This skips both test-harness and doctest compilation; the three default binaries,
additional named binaries, source snapshot, generated runtime fixtures and
runtime dependencies are still included. Its manifest records empty `tests` and
`steps` arrays: it is not a GPU-gate bundle. Use `--gate-bundles none`, or exclude
that bundle's index when mixing it with full-gate bundles. Session preflight
rejects a selected full gate for a benchmark-only archive before provisioning;
the pod runner independently rejects full-gate execution rather than treating
missing tests as a pass.

The archive contains `manifest.json` (commit, commands, working directories,
timings, per-file SHA256), `bin/`, `lib/`, the complete nonignored source/fixture
snapshot in `source/`, local build logs, downloader dependencies, and the pod
scripts. Dirty working-tree inputs are copied, not replaced with `git show HEAD`.
Cargo caches and the CUDA compiler are not shipped. Selected packages' exact
`build-script-executed` OUT_DIRs contribute generated source/data fixtures at
their original paths, including TPC-H's `string-guard-fixture`; compiler objects
and libraries in those directories are excluded. `ldd` resolves
transitive non-system shared libraries; these are copied by SONAME. System glibc,
the loader, and `libcuda` remain supplied by the pod OS/driver.

**Cubin mechanism:** the pinned cuda-oxide materializer embeds native target
cubins inside the final ELF `.oxart` section; generic kernels use an ahead-of-time
cross-crate linked cubin. The loader reads the executable / original mapped ELF
(Linux `/proc/self/maps` and `/proc/self/map_files`) and submits the cubin to the
CUDA driver. No sidecar `.cubin`, `CUDA_OXIDE_PTX_DIR`, libNVVM, nvJitLink, `nsys`
or `ncu` is needed for these materialized kernels. Keep the bundled ELF readable
and do not strip its artifact sections.

### Dataset and path mirrors

The private HF dataset has `SHA256SUMS` at its root, with ordinary sha256sum
lines whose filenames are relative to the dataset root:

* `tpch-gpu/{sf1,sf10,synthetic}/...`
* `doop-bench/doop-data/{cassandra,xalan,batik}/...`

The initial private dataset is `ysun67/mini-linq-gpu-data`, pinned at
`c2e07f9a10a37fe0c6984c66e9eec6080a82110e` (829 repository files including
SHA256SUMS). Supply that commit to `seed_volume.sh`; use a new immutable revision
and regenerate the trusted manifest when dataset contents change.

`seed_volume.sh REPO REVISION TRUSTED_SHA256SUMS [DATA_ROOT]` uses `hf download
--repo-type dataset --revision REVISION --local-dir /workspace/data`. The revision
must be an immutable 40-character commit. It checks that downloaded SHA256SUMS
matches the locally produced trusted copy, checks every file and the complete
six-root inventory, then atomically writes `.seeded-REVISION`. Interrupted seeds
are resumed and never marked successful. A marker is reused only for the same
repo, revision and manifest digest. The downloader's Python dependencies are
prepared locally in the bundle; there is no pod-side pip install.

On a fresh pod `pod_gate.sh --results DIR` links the embedded worktree path to
the unpacked `source/`, `/data/tpch-gpu` to `/workspace/data/tpch-gpu`, and
`/data/doop-bench/doop-data` to `/workspace/data/doop-bench/doop-data`. Existing
unrelated directories/links are never overwritten. A previous bundle's worktree
symlink can be switched to the next bundle. Test harnesses run from their
original package directories, preserving runtime source/fixture reads.

The pod runner validates bundle hashes, an sm_89 device reported by `nvidia-smi`,
and driver CUDA >= 13.0, then executes `crates`, `tpch-h05-sf10-ceiling` and
`doop-cassandra-exact`. The DOOP build is already complete locally. It prints the
same `gpu-gate PASS|FAIL NAME Ns` lines as `gpu_gate.sh`; per-step logs,
`gpu-gate-h05-sf10.json`, device information and `status.json` survive in DIR.
Failures exit nonzero. `--bench NAME ARGS...` instead executes one packaged
binary with literal arguments (no shell evaluation). The alternative
`--bench python3 source/PATH.py ARGS...` runs a Python driver from the bundle's
hashed source snapshot. The interpreter is exactly `python3`; the script must
be a manifest-listed `.py` file below `source/`, not an absolute/traversing path.
Script arguments remain literal argv, with `{results}` expanded as for binaries.

### One bounded rental session

```sh
wt=/data/mini_linq-wt/remote-gpu
bash "$wt/tools/remote_gpu/session.sh" \
  --volume dp2a7xuy9t --data-center US-WA-1 \
  --gpu-ids 'NVIDIA RTX 6000 Ada Generation' \
  --revision c2e07f9a10a37fe0c6984c66e9eec6080a82110e \
  --data-manifest /data/hf-stage/mini-linq-gpu-data/SHA256SUMS \
  --ssh-key "$HOME/.ssh/id_ed25519" \
  --bench 'cq-gpu-tpch {results}/h06-sf10.json --query h06 --scale sf10 --mode advised' \
  "$wt/target/remote-gpu/gate.tar.zst"
```

Pass additional bundle paths for paired revisions, and repeat `--bench` for
additional commands. Every benchmark command runs once per bundle.
`--gate-bundles all` (the default) runs the full GPU gate for every bundle;
`--gate-bundles 0,2` gates only those zero-based indices; `--gate-bundles none`
makes the session benchmark-only. Out-of-range indices fail before provisioning.
Skipped gates are omitted from the planned steps, not reported as passing.

All bundles are unpacked before any gate/benchmark runs, into stable sibling
directories `/workspace/remote-gpu/TIMESTAMP/bundle-0`, `bundle-1`, etc. Benchmark
processes receive `BUNDLE_DIR` (their current absolute bundle path), `BUNDLE_DIRS`
(a JSON array of all absolute bundle paths in CLI order), and
`CUDA_VISIBLE_DEVICES` (the checked sm_89 GPU). A paired driver can interleave
binaries from those paths, setting each binary's `LD_LIBRARY_PATH` to its bundle's
`lib/` plus the NVIDIA driver directories and preparing its worktree mirror.
For a single interleaving run under the per-bundle dispatch contract, a driver
can return immediately unless `BUNDLE_DIR == json.loads(BUNDLE_DIRS)[-1]`.
For example, add these options before two bundle arguments:

```sh
--gate-bundles 0 \
--bench 'python3 source/experiments/gpu/compare/pod_bench_2026_11_02.py --results {results} --budget 2400 --pairs 5'
```

The driver must be present in every supplied bundle. Rebuild bundles with the
updated tooling to use interpreted benchmarks; previously built archives retain
their original pod runner. Gate selection does not change seeding, result
retrieval, the 60-minute watchdog, or teardown.

The GPU id list is comma-separated and tried in priority order; the volume's DC
must offer those devices (US-WA-1 currently has RTX 6000 Ada, not L40). The dataset
repo defaults to `<hf auth whoami>/mini-linq-gpu-data`.
The token is read from `HF_TOKEN_PATH`, or the standard `HF_HOME/token` /
`XDG_CACHE_HOME/huggingface/token`, and passed as pod environment without logging
it. The SSH key must exist locally; its `.pub` is injected as `PUBLIC_KEY`.
All local prerequisites are checked before provisioning.

The image is digest-pinned `runpod/base:1.6.0-ubuntu2204`, whose startup implements
RunPod's PUBLIC_KEY/sshd convention and includes Python, tar/zstd and rsync. Its
653 MB compressed amd64 manifest was checked on Docker Hub; CUDA 13 runtime
libraries come from the bundle. There is no published
`runpod/base:*cuda1300-ubuntu2204` tag; the available CUDA 13 RunPod images use
Ubuntu 24.04 and include multi-GB development toolkits. The plain
`nvidia/cuda:13.0.0-runtime-ubuntu22.04` tag exists but does not include sshd.
Using the smaller Ubuntu 22.04 RunPod base plus bundled runtime avoids installing
or compiling anything on paid GPU time. Registry evidence:
[RunPod base tag](https://hub.docker.com/v2/repositories/runpod/base/tags/1.6.0-ubuntu2204),
[NVIDIA runtime tag](https://hub.docker.com/v2/repositories/nvidia/cuda/tags/13.0.0-runtime-ubuntu22.04),
[RunPod SSH startup](https://github.com/runpod/containers/blob/main/container-template/start.sh).

`session.sh` uses the runpodctl 2.12.0 help-confirmed `pod create --wait
--wait-timeout 10m`, `ssh info`, `pod list --name`, and `pod delete` commands.
It copies archives over public SSH, seeds once, executes selected gates and all
requested benchmarks, and retrieves results under `target/remote-gpu/TIMESTAMP/`. The
`finally` cleanup and SIGINT/SIGTERM handlers delete the pod on success, failure
or Ctrl-C. An independent 60-minute watchdog deletes it even while SSH is blocked
in a running GPU step. No stop-only operation is used and the volume is retained.
The final summary reports uncompleted steps and create-to-delete billed-minute
upper bound (wall clock, not a claim about invoice rounding). A control-plane
outage can prevent any client from deleting a pod; deletion is retried and a
critical manual-delete message includes the pod id if all attempts fail.
Completed results are copied after each step; at the hard cap any remaining
partial logs stay on the volume at the printed remote results path.

For local bundle verification, unpack under `/tmp`, use a private mount
namespace to bind `source/` over the original worktree, and pass
`--existing-mirrors --data /data`. Hide/move the original build target directories
before running, restore them afterward, and hold `/tmp/mini_linq-gpu.lock` (GPU
timing rule above). On a VRAM-constrained shared workstation,
`RUST_TEST_THREADS=1` serializes the same tests without filtering any out; leave
the pod's default unchanged. `--allow-sf10-oom` is only for this workstation:
it first attempts SF10, records an explicit `SKIP-OOM` only for a CUDA
out-of-memory failure, and never records that step as PASS. Do not pass it to a
rented pod. A ceiling failure, crash, or any other failure still fails the gate.

The crate suites also include an unignored `h01_gpu_matches_duckdb_sf10` test.
It can OOM even with serialized tests when a foreign GPU process occupies most
VRAM; the runner does not skip or suppress that failure. Likewise a runnable H05
on a shared GPU may exceed its performance ceiling. Such local evidence proves
bundle execution/isolation, not a passing isolated GPU gate.

Offline lifecycle tests (including success/failure/Ctrl-C/cap deletion and secret
redaction) use test doubles and never invoke RunPod:

```sh
flock /tmp/mini_linq-timing-gate.lock true &&
  flock -s /tmp/mini_linq-timing.lock \
  python3 tools/remote_gpu/test_remote_gpu.py
```
