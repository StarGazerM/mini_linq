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
Datalog, Sequence and TPC-H crate suites, the advised H05 SF10 benchmark (including
its existing 10.35 ms ceiling), the `doop-gpu`/`srd-doop-gpu` builds, and `doop-gpu`
on Cassandra, which compares all 26 complete output tuple sets against `doop-mini`.
It holds `/tmp/mini_linq-gpu.lock` for the whole run. H05 timing and exactness
evidence is written to `target/gpu-gate-h05-sf10.json`; a ceiling failure fails
the batch even when every answer is exact.

Builds, tests and timings share one machine through two lock files:

```sh
# Builds and tests: run concurrently with each other, never during a measurement.
flock /tmp/mini_linq-timing-gate.lock true && flock -s /tmp/mini_linq-timing.lock CMD
# Timing measurements: exclusive. Holding the gate blocks new builds, so a
# measurement cannot be starved by a stream of shared holders.
flock /tmp/mini_linq-timing-gate.lock flock -x /tmp/mini_linq-timing.lock CMD
```

GPU execution additionally holds `/tmp/mini_linq-gpu.lock`, taken before the
timing locks. Hold the exclusive lock only for the measurement itself.

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
before running, restore them afterward, and take `/tmp/mini_linq-gpu.lock` before
the exclusive timing locks. On a VRAM-constrained shared workstation,
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
