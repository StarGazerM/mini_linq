#!/usr/bin/env python3
"""Run the macro package's real test targets under supported Cargo feature sets.

Unlike check_dependencies.py's external macro probes, these commands exercise
the package's own tests, examples and doctests. Cases come from Cargo metadata:
no defaults, individual completed features, default/completed bundles, and each
test/example's declared prerequisites. This is not every possible feature subset.
"""
from __future__ import annotations

import argparse
import json
import os
import re
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
PACKAGE = "mini-linq"


def feature_cases(package):
    features = package["features"]

    def closure(selected):
        active = set()
        pending = list(selected)
        while pending:
            name = pending.pop()
            if name in active:
                continue
            if name not in features:
                raise ValueError(f"unknown local feature: {name}")
            active.add(name)
            pending.extend(value for value in features[name]
                           if not value.startswith("dep:") and "/" not in value)
        return frozenset(active)

    cases = []
    covered = set()

    def add(label, selected):
        active = closure(["default"] if selected is None else selected)
        if active not in covered:
            covered.add(active)
            cases.append((label, selected, active))

    add("no-default", [])
    for feature in sorted(set(features) - {"default"}):
        add(feature, [feature])
    add("default", None)
    add("completed", sorted(set(features) - {"default"}))
    add("execution-alternatives", ["access-to-pipeline", "access-to-set-loops", "pipeline-distinct-iter", "pipeline-set"])
    for target in package["targets"]:
        if not set(target["kind"]) & {"test", "example"}:
            continue
        required = target.get("required-features", [])
        add(f'{target["kind"][0]}-{target["name"]}', required)
    return cases


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--list", action="store_true", help="list cases without running Cargo tests")
    parser.add_argument("--only", action="append", default=[], help="run only a named case (repeatable)")
    args = parser.parse_args()
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--format-version=1", "--locked"],
        cwd=ROOT, text=True))
    package = next(p for p in metadata["packages"] if p["name"] == PACKAGE)
    cases = feature_cases(package)
    unknown = set(args.only) - {label for label, _, _ in cases}
    if unknown:
        parser.error(f"unknown cases: {', '.join(sorted(unknown))}; use --list")
    if args.only:
        cases = [case for case in cases if case[0] in args.only]
    logdir = ROOT / "target/feature-logs"
    if not args.list:
        logdir.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env["CARGO_TERM_COLOR"] = "never"
    for label, selected, active in cases:
        command = ["cargo", "test", "-p", PACKAGE, "--locked"]
        if selected is not None:
            command.append("--no-default-features")
            if selected:
                command.extend(["--features", ",".join(selected)])
        if args.list:
            print(f"{label}: {' '.join(command)}")
            continue
        logpath = logdir / f"{label}.log"
        print(f"RUN: {label}", flush=True)
        with logpath.open("w") as log:
            result = subprocess.run(command, cwd=ROOT, env=env,
                                    stdout=log, stderr=subprocess.STDOUT)
        if result.returncode:
            print("\n".join(logpath.read_text().splitlines()[-60:]), flush=True)
            raise SystemExit(f"FAIL: {label}; full output: {logpath}")
        targets = [t for t in package["targets"]
                   if set(t["kind"]) & {"test", "example"}
                   and set(t.get("required-features", [])) <= active]
        output = logpath.read_text()
        tested = sum(map(int, re.findall(r"test result: ok\. (\d+) passed", output)))
        for target in targets:
            if "test" not in target["kind"]:
                continue
            source = Path(target["src_path"]).relative_to(Path(package["manifest_path"]).parent)
            if f"Running {source} (" not in output:
                raise SystemExit(f"FAIL: {label} did not execute eligible test target {target['name']}; see {logpath}")
        print(f"PASS: {label} ({tested} tests executed; {len(targets)} eligible test/example targets)", flush=True)
    if not args.list:
        print(f"PASS: {len(cases)} Cargo feature cases; logs: {logdir}", flush=True)


if __name__ == "__main__":
    main()
