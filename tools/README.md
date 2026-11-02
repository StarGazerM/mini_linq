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
# Full inventory remains the default.
nice -n 10 bash tools/test_regressions.sh
```

Do not run every feature workspace's `cargo test --workspace` and then repeat
those same tests through the regression inventory. The behavioral gate is the
regression suite plus root `cargo test`; run a feature-workspace sweep only when
the track changed that workspace and it covers tests absent from the inventory.
A track's gate is the tests of the crates it changed, the tests covering its fix,
root `cargo test --workspace` when core, testkit or facade code changed, the quick
tier, and any named downstream target it knows it affects. Tracks do not run the
impact-selected suite: a change to any compiler crate selects every target. Main
runs it once per merge batch with `--changed-since BASE --reuse-latest target/baseline-gate`. Run
separate structural, API, formatting or documentation gates only where the change
requires them. Do not rerun behavioral gates for a docs-only rebase.
An explicitly named selector narrows even an impact-selected run; omit selectors
for the final gate. Unknown selectors are errors, not successful empty gates.

The regression inventory contains no GPU targets. When a merge batch can affect
GPU code (any shared compiler crate, `experiments/gpu`, or the Datalog
pipelines), Main also runs `bash tools/gpu_gate.sh` once for the batch: the GPU
Datalog, Sequence and TPC-H crate suites, the `doop-gpu`/`srd-doop-gpu` builds,
and `doop-gpu` on Cassandra, which compares all 26 complete output tuple sets
against `doop-mini`. It holds `/tmp/mini_linq-gpu.lock` for the whole run.

Verification follows what a change can break: docs-only changes run nothing;
test tooling runs its own unit tests; an oracle/infrastructure swap checks only
that the oracle is unchanged; compiler/runtime changes run the track gate above.
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

Named feature-specific manifest tests use the `manifest-feature-test` inventory
kind and `TEST@FEATURE` target spelling (`@all-features` selects `--all-features`).
Both execution and identity expansion use the same package, test and feature flags.

## Performance comparison protocol

Compare one warmed workspace-layout run with one warmed shared-layout run on the
same source snapshot, preserving the same target set, job limit and environment.
Record start/end load averages on shared hosts, and do not claim a cold cache is
warm. Change the runner's default only after an observed wall-time improvement.

The preexisting `features/.cargo/config.toml` already shares a feature build
directory for commands whose working directory discovers that configuration;
workload commands have their own configuration. The corpus and workload e2e
packages already set `debug=0`. These settings belong to the baseline and must
not be credited to this change. Package-specific codegen/debuginfo changes likewise
require measured compile-time improvement and passing tests before adoption.

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
it. The default is therefore **118 deterministic targets** and the inventory
contains 121 rows. The impact/quick measurements above predate this intentional
tier migration. Test-layer additions must keep randomly generated consumers out
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
