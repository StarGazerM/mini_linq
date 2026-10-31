#!/usr/bin/env python3
"""Run independent regression targets without competing for Cargo build locks."""
import argparse
import concurrent.futures
import csv
import functools
import json
import os
from pathlib import Path
import subprocess
import time

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
    elif kind not in ("manifest-test", "oracle-test", "manifest-target-test"):
        raise ValueError(f"Unknown regression kind: {kind}")
    cargo += ["--manifest-path", source]
    if kind == "manifest-target-test":
        cargo += ["--test", target]
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
    with log.open("w") as output:
        if not (ROOT / source).is_file():
            output.write(f"Missing regression source: {source}\n")
            status = 1
        else:
            try:
                status = subprocess.run(command_for(*row), cwd=ROOT, stdout=output,
                                        stderr=subprocess.STDOUT).returncode
                if status < 0:
                    status = 128 - status
            except OSError as error:
                output.write(f"{error}\n")
                status = 127 if isinstance(error, FileNotFoundError) else 126
    return (*row[:3], status, str(log))


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
    parser.add_argument("selectors", nargs="*", help="exact package/target pairs; omitted runs the full inventory")
    args = parser.parse_args()
    started = time.monotonic()
    rows = []
    with (ROOT / "tools/regression-targets.tsv").open() as source:
        for row in csv.reader(source, delimiter="\t"):
            if not row or not row[0] or row[0].startswith("#"):
                continue
            if not args.selectors or f"{row[0]}/{row[2]}" in args.selectors:
                try:
                    command_for(*row)  # Reject unknown kinds before starting any target.
                except ValueError as error:
                    parser.error(str(error))
                rows.append(row)
    logdir = ROOT / "target/regression-logs"
    logdir.mkdir(parents=True, exist_ok=True)
    results = run(rows, args.jobs, logdir)
    summary = logdir / "results.tsv"
    with summary.open("w") as output:
        writer = csv.writer(output, delimiter="\t", lineterminator="\n")
        writer.writerow(("package", "kind", "target", "status", "log"))
        writer.writerows(results)
    passed = 0
    for package, _, target, status, log in results:
        if status == 0:
            passed += 1
            print(f"PASS {package}/{target}")
        else:
            print(f"FAIL {package}/{target} (exit {status}): {log}")
    failed = len(results) - passed
    print(f"{passed} passed, {failed} failed; results: {summary}")
    print(f"Wall time: {time.monotonic() - started:.3f} seconds; jobs: {args.jobs}")
    return int(failed != 0 or passed == 0)


if __name__ == "__main__":
    raise SystemExit(main())
