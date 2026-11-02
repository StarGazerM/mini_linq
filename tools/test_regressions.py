#!/usr/bin/env python3
"""Run independent regression targets without competing for Cargo build locks."""
import argparse
import concurrent.futures
import csv
import functools
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
# Scenarios invoke Cargo themselves, sometimes from a different working directory.
# (manifest, cwd) describes each invocation's workspace/config discovery context.
SCENARIO_CARGO = {
    "features/diverged-ra/check.py": [("features/diverged-ra/Cargo.toml", "features/diverged-ra")],
    "features/inspection/cq-inspection/check.py": [("features/inspection/Cargo.toml", "features/inspection")],
    "features/inspection/free-variables/check.py": [("features/inspection/Cargo.toml", "features/inspection")],
    "features/stacked-ir/check_expansions.py": [("features/stacked-ir/Cargo.toml", "features/stacked-ir")],
    "features/workloads/check_growth.py": [("features/workloads/Cargo.toml", ".")],
    "features/workloads/check_typed.py": [("features/workloads/e2e/Cargo.toml", ".")],
}
NON_CARGO_SCENARIOS = {
    "tools/check_dependencies.py", "tools/test_check_external.py", "tools/test_public_api.py",
}
# Small deterministic compiler/runtime contracts, not the generated workload matrix.
QUICK_TARGETS = frozenset({
    "cq-storage/access_laws", "cq-storage/representation_laws", "cq-storage/counters",
    "cq-index-policy/index_demands", "cq-lang-cq/program", "cq-lang-cq/normal",
    "cq-lang-cq/guards", "cq-print/expressions", "mini-linq-macros/macro_inputs",
    "architecture/closed-core-dependencies", "architecture/external-sibling-closure",
    "architecture/public-api-reexports",
    "cq-datalog/execution", "cq-datalog/execution@all-features", "cq-datalog/counters@counters",
})
RUN_OPTIONS = {}


def target_id(row):
    return f"{row[0]}/{row[2]}"


def interval_seconds(intervals, started, ended):
    """Union nested/concurrent build intervals, clipped to this target's wall."""
    total = 0.0
    cursor = started
    for first, last in sorted(intervals):
        first, last = max(first, started), min(last, ended)
        if last > max(cursor, first):
            total += last - max(cursor, first)
            cursor = last
    return total


def timing_environment(logdir, timings):
    directory = logdir / "cargo-bin"
    directory.mkdir(exist_ok=True)
    shim = directory / "cargo"
    try:
        shim.symlink_to(ROOT / "tools/regression_cargo.py")
    except FileExistsError:
        pass
    env = os.environ.copy()
    env["REGRESSION_REAL_CARGO"] = shutil.which("cargo") or "cargo"
    env["REGRESSION_CARGO_TIMINGS"] = str(timings)
    env["PATH"] = str(directory) + os.pathsep + env.get("PATH", "")
    env.setdefault("CARGO_TERM_COLOR", "never")
    return env


def load_evidence(path):
    value = json.loads(path.read_text())
    if value.get("version") != 1 or not isinstance(value.get("run_id"), str):
        raise ValueError("reuse evidence requires version=1 and run_id")
    records = {}
    for record in value.get("targets", []):
        if record.get("status") != 0 or record.get("outcome") != "passed" or not record.get("identity"):
            continue
        log = path.parent / record["log"]
        if not log.is_file() or hashlib.sha256(log.read_bytes()).hexdigest() != record.get("log_sha256"):
            continue
        records[tuple(record["row"])] = {**record, "run_id": value["run_id"]}
    return records


def latest_evidence(directory):
    paths = list(directory.rglob("evidence.json"))
    if not paths:
        return None
    return max(paths, key=lambda path: (path.stat().st_mtime_ns, str(path)))


def save_baseline(logdir, directory, revision, records):
    """Never publish failed, empty, or revisionless runs as baselines."""
    if not revision or not records or any(record["status"] != 0 for record in records):
        return None
    destination = directory / revision[:12]
    directory.mkdir(parents=True, exist_ok=True)
    # Refuse to mix evidence from separate runs at the same revision.
    shutil.copytree(logdir, destination)
    return destination


def command_for(package, kind, target, source):
    cargo = ["cargo", "test", "--locked"]
    if kind in ("test", "example"):
        return cargo + ["-p", package, "--all-features", f"--{kind}", target]
    if kind == "lib":
        return cargo + ["-p", package, "--lib"]
    if kind == "scenario":
        return ["python3", source]
    if kind in ("manifest-run", "oracle"):
        cargo = ["cargo", "run", "--locked"]
    elif kind not in ("manifest-test", "oracle-test", "manifest-target-test", "manifest-feature-test"):
        raise ValueError(f"Unknown regression kind: {kind}")
    cargo += ["--manifest-path", source]
    if kind == "manifest-target-test":
        cargo += ["--test", target]
    elif kind == "manifest-feature-test":
        test, feature = target.rsplit("@", 1)
        cargo += ["-p", package, "--test", test]
        cargo += ["--all-features"] if feature == "all-features" else ["--features", feature]
    elif kind in ("oracle", "oracle-test"):
        cargo += ["--features", "oracle"]
        if kind == "oracle":
            cargo += ["--", target]
    return cargo


@functools.cache
def cargo_resources(manifest, cwd):
    result = subprocess.run(
        ["cargo", "metadata", "--locked", "--offline", "--no-deps",
         "--format-version", "1", "--manifest-path", str(ROOT / manifest)],
        cwd=ROOT / cwd, capture_output=True, text=True,
    )
    if result.returncode:
        # Still execute the real command and preserve its error/log/status. Without
        # metadata we cannot safely assume that its build directory is independent.
        return frozenset({"*"})
    metadata = json.loads(result.stdout)
    return frozenset(str(Path(metadata[key]).resolve())
                     for key in ("workspace_root", "target_directory"))


def resources_for(row):
    _, kind, _, source = row
    if not (ROOT / source).is_file():
        return frozenset()
    if kind == "scenario":
        if source in NON_CARGO_SCENARIOS:
            return frozenset()
        contexts = SCENARIO_CARGO.get(source)
        if contexts is None:
            return frozenset({"*"})
        resources = frozenset().union(*(cargo_resources(*context) for context in contexts))
        if source == "features/stacked-ir/check_expansions.py":
            # This scenario overrides Cargo's configured target directory.
            directory = Path(os.environ.get("CARGO_TARGET_DIR", "target"))
            resources |= {str((ROOT / "features/stacked-ir" / directory).resolve())}
        return resources
    manifest = "Cargo.toml" if kind in ("test", "example", "lib") else source
    return cargo_resources(manifest, ".")


def conflicts(resources, active):
    return bool(active) and ("*" in resources or "*" in active or bool(resources & active))


def execute(row, logdir):
    package, kind, target, source = row
    log = logdir / f"{package}-{target.replace('*', 'all')}.log"
    timings = log.with_suffix(".cargo.jsonl")
    started = time.monotonic()
    intervals = []
    identity = None
    identity_reason = "identity recording not requested"
    outcome = "passed"
    identity_engine = None
    with log.open("w") as output:
        if not (ROOT / source).is_file():
            output.write(f"Missing regression source: {source}\n")
            status = 1
        else:
            if RUN_OPTIONS.get("record_identities") or "reuse" in RUN_OPTIONS:
                from regression_identity import Identity
                identity_engine = Identity(ROOT)
                identity = identity_engine.fingerprint(row)
                intervals.append((started, time.monotonic()))
                identity_reason = identity_engine.last_reason if identity is None else "identity established"
                if identity is None:
                    output.write(f"Identity unavailable: {identity_reason}\n")
            prior = RUN_OPTIONS.get("reuse", {}).get(tuple(row))
            if identity is not None and prior and identity == prior["identity"]:
                status = 0
                outcome = f"reused {prior['run_id']}"
                identity_reason = "identical expansion, runtime, test and build inputs"
                output.write(f"{outcome}: expanded Rust, linked runtime and test inputs identical\n")
            else:
                if "reuse" in RUN_OPTIONS and identity is not None:
                    identity_reason = ("no intact executed baseline identity" if not prior
                                       else "expansion, runtime, test or build inputs changed")
                    output.write(f"Reuse unavailable: {identity_reason}\n")
                try:
                    status = subprocess.run(command_for(*row), cwd=ROOT, stdout=output,
                                            stderr=subprocess.STDOUT,
                                            env=timing_environment(logdir, timings)).returncode
                    if status < 0:
                        status = 128 - status
                except OSError as error:
                    output.write(f"{error}\n")
                    status = 127 if isinstance(error, FileNotFoundError) else 126
                if status == 0 and identity is not None:
                    before = time.monotonic()
                    if identity_engine.fingerprint(row) != identity:
                        output.write("Inputs changed during execution; not recording reusable identity\n")
                        identity_reason = "inputs changed during execution"
                        identity = None
                    intervals.append((before, time.monotonic()))
    ended = time.monotonic()
    if timings.exists():
        for line in timings.read_text().splitlines():
            event = json.loads(line)
            intervals.append((event["start"], event["end"]))
    build = interval_seconds(intervals, started, ended)
    wall = ended - started
    if status:
        outcome = "failed"
        identity = None
    record = {"row": list(row), "status": status, "outcome": outcome,
              "identity": identity, "identity_reason": identity_reason, "log": log.name,
              "log_sha256": hashlib.sha256(log.read_bytes()).hexdigest(),
              "build_seconds": build, "test_seconds": wall - build, "wall_seconds": wall}
    log.with_suffix(".json").write_text(json.dumps(record, indent=2) + "\n")
    return (*row[:3], status, str(log), build, wall - build, wall, outcome)


def run(rows, jobs, logdir):
    pending = [(index, row, resources_for(row)) for index, row in enumerate(rows)]
    results = [None] * len(rows)
    active = {}
    with concurrent.futures.ThreadPoolExecutor(max_workers=jobs) as pool:
        while pending or active:
            occupied = frozenset().union(*(item[1] for item in active.values()))
            for item in pending[:]:
                if len(active) == jobs:
                    break
                index, row, resources = item
                if conflicts(resources, occupied):
                    continue
                print(f"RUN  {row[0]}/{row[2]}", flush=True)
                active[pool.submit(execute, row, logdir)] = (index, resources)
                occupied |= resources
                pending.remove(item)
            completed, _ = concurrent.futures.wait(active, return_when=concurrent.futures.FIRST_COMPLETED)
            for future in completed:
                index, _ = active.pop(future)
                results[index] = future.result()
    return results


def positive(value):
    number = int(value)
    if number < 1:
        raise argparse.ArgumentTypeError("jobs must be positive")
    return number


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--jobs", type=positive, default=2,
                        help="maximum concurrent independent targets (default: 2; 1 is sequential)")
    parser.add_argument("--tier", choices=("full", "quick", "fuzz"), default="full",
                        help="deterministic inventory (default), quick contracts, or opt-in fuzz targets")
    parser.add_argument("--changed-since", help="select only targets potentially affected since this revision")
    parser.add_argument("--changed-until", help="end revision for a historical impact range (default: working tree)")
    parser.add_argument("--list", action="store_true", help="print selected targets and reasons without executing")
    reuse = parser.add_mutually_exclusive_group()
    reuse.add_argument("--reuse-from", type=Path, help="explicit passing evidence.json with expansion identities")
    reuse.add_argument("--reuse-latest", type=Path, help="use newest evidence.json recursively under DIR")
    parser.add_argument("--save-baseline", type=Path, help="copy a passing run to DIR/HEAD-short-sha")
    parser.add_argument("--record-identities", action="store_true",
                        help="record expanded Rust/runtime identities for a later --reuse-from run")
    parser.add_argument("--target-dirs", choices=("workspace", "shared"), default="workspace",
                        help="workspace Cargo defaults or a worktree-local shared directory")
    parser.add_argument("--output", type=Path, help="new run directory (default: target/regression-logs/RUN-ID)")
    parser.add_argument("selectors", nargs="*", help="exact package/target pairs; omitted runs the full inventory")
    args = parser.parse_args()
    if args.changed_until and not args.changed_since:
        parser.error("--changed-until requires --changed-since")
    started = time.monotonic()
    inventory = []
    tiers = {}
    with (ROOT / "tools/regression-targets.tsv").open() as source:
        for row in csv.reader(source, delimiter="\t"):
            if not row or not row[0] or row[0].startswith("#"):
                continue
            if len(row) not in (4, 5) or (len(row) == 5 and row[4] != "fuzz"):
                parser.error(f"invalid regression row: {row!r}")
            tier = row[4] if len(row) == 5 else "full"
            row = row[:4]
            try:
                command_for(*row)
            except ValueError as error:
                parser.error(str(error))
            inventory.append(row)
            tiers[target_id(row)] = tier
    unknown = set(args.selectors) - {target_id(row) for row in inventory}
    if unknown:
        parser.error("unknown targets: " + ", ".join(sorted(unknown)))
    if args.tier == "quick" and set(args.selectors) - QUICK_TARGETS:
        parser.error("explicit selectors outside the quick tier; use --tier full")
    rows = [row for row in inventory if
            (target_id(row) in args.selectors if args.selectors else
             tiers[target_id(row)] == ("fuzz" if args.tier == "fuzz" else "full"))
            and (args.tier != "quick" or target_id(row) in QUICK_TARGETS)]
    selection = {target_id(row): ["explicit selector" if args.selectors else f"{args.tier} tier"]
                 for row in rows}
    if args.changed_since:
        from regression_impact import Impact
        try:
            impact = Impact(ROOT, inventory, args.changed_since, args.changed_until)
        except (ValueError, RuntimeError, subprocess.CalledProcessError) as error:
            parser.error(str(error))
        selection = {target_id(row): impact.reasons(row) for row in rows}
        rows = [row for row in rows if selection[target_id(row)]]
        print("Changed files: " + (", ".join(impact.changed_files) or "(none)"))
    for row in rows:
        print(f"SELECT {target_id(row)}: {'; '.join(selection[target_id(row)])}")
    print(f"Selected {len(rows)}/{len(inventory)} targets")
    if args.reuse_latest:
        try:
            args.reuse_from = latest_evidence(args.reuse_latest)
        except OSError as error:
            parser.error(f"cannot discover reuse evidence: {error}")
        if args.reuse_from is None:
            print(f"Reuse unavailable: no evidence.json under {args.reuse_latest}; execute selected targets")
    if args.target_dirs == "shared":
        os.environ.setdefault("CARGO_TARGET_DIR", str(ROOT / "target/regression-shared"))
    # Relative overrides otherwise resolve differently for nested scenario cwd's.
    if "CARGO_TARGET_DIR" in os.environ:
        os.environ["CARGO_TARGET_DIR"] = str(Path(os.environ["CARGO_TARGET_DIR"]).absolute())
    cargo_resources.cache_clear()
    RUN_OPTIONS.clear()
    RUN_OPTIONS["record_identities"] = args.record_identities or bool(args.save_baseline)
    if args.reuse_from:
        try:
            RUN_OPTIONS["reuse"] = load_evidence(args.reuse_from.resolve())
        except (OSError, ValueError, KeyError, TypeError) as error:
            parser.error(f"invalid reuse evidence: {error}")
        print(f"Validated {len(RUN_OPTIONS['reuse'])} reusable passing records from {args.reuse_from}")
    if args.list:
        if args.reuse_from:
            from regression_identity import Identity
            engine = Identity(ROOT)
            for row in rows:
                prior = RUN_OPTIONS["reuse"].get(tuple(row))
                if not prior:
                    print(f"EXECUTE {target_id(row)}: no intact executed baseline identity")
                    continue
                identity = engine.fingerprint(row)
                if identity is not None and identity == prior["identity"]:
                    print(f"REUSE {target_id(row)}: identical identity from {prior['run_id']}")
                else:
                    reason = engine.last_reason if identity is None else "identity mismatch"
                    print(f"EXECUTE {target_id(row)}: {reason}")
        print(f"Selection wall time: {time.monotonic() - started:.3f} seconds")
        return 0
    run_id = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime()) + "-" + uuid.uuid4().hex[:8]
    logdir = (args.output or ROOT / "target/regression-logs" / run_id).resolve()
    if (logdir / "results.tsv").exists():
        parser.error(f"refusing to overwrite an existing run: {logdir}")
    logdir.mkdir(parents=True, exist_ok=True)
    revision = subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT,
                              capture_output=True, text=True)
    results = run(rows, args.jobs, logdir)
    summary = logdir / "results.tsv"
    with summary.open("w") as output:
        writer = csv.writer(output, delimiter="\t", lineterminator="\n")
        writer.writerow(("package", "kind", "target", "status", "log",
                         "build_seconds", "test_seconds", "wall_seconds", "outcome"))
        for result in results:
            writer.writerow((*result[:5], *(f"{number:.6f}" for number in result[5:8]), result[8]))
    passed = 0
    reused = 0
    for package, _, target, status, log, build, test, wall, outcome in results:
        if status == 0:
            passed += 1
            reused += outcome.startswith("reused ")
            print(f"PASS {package}/{target} ({outcome}; build {build:.3f}s, test {test:.3f}s)")
        else:
            print(f"FAIL {package}/{target} (exit {status}): {log}")
    failed = len(results) - passed
    elapsed = time.monotonic() - started
    evidence = {"version": 1, "run_id": run_id, "selection": selection,
                "revision": revision.stdout.strip() if revision.returncode == 0 else None,
                "changed_since": args.changed_since, "changed_until": args.changed_until,
                "tier": args.tier, "target_dirs": args.target_dirs, "wall_seconds": elapsed,
                "targets": [json.loads(Path(result[4]).with_suffix(".json").read_text())
                            for result in results]}
    (logdir / "evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")
    if args.save_baseline and not failed:
        try:
            destination = save_baseline(logdir, args.save_baseline.resolve(),
                                        evidence["revision"], evidence["targets"])
        except OSError as error:
            parser.error(f"cannot save baseline: {error}")
        print(f"Saved baseline: {destination}" if destination else
              "Baseline not saved: empty selection or unavailable HEAD")
    print(f"{passed} passed, {failed} failed, {reused} reused; results: {summary}")
    print(f"Target totals: build {sum(r[5] for r in results):.3f}s; "
          f"test {sum(r[6] for r in results):.3f}s (overlapping targets are summed)")
    print(f"Wall time: {elapsed:.3f} seconds; jobs: {args.jobs}; run: {run_id}")
    return int(failed != 0)


if __name__ == "__main__":
    raise SystemExit(main())
