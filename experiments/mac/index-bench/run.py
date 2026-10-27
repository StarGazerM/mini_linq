#!/usr/bin/env python3
"""Fetch pinned artifacts, build the native harness, and retain raw measurements."""
import argparse
import csv
from datetime import datetime, timezone
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import shlex
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parent
TARGET = ROOT / "target"
ARTIFACTS = TARGET / "artifacts"


def run(command, **kwargs):
    print("+ " + shlex.join(map(str, command)), file=sys.stderr)
    return subprocess.run(list(map(str, command)), check=True, text=True, **kwargs)


def fetch(manifest):
    ARTIFACTS.mkdir(parents=True, exist_ok=True)
    for name, spec in manifest.items():
        checkout = ARTIFACTS / name
        if not checkout.exists():
            run(["git", "init", checkout])
            run(["git", "-C", checkout, "fetch", "--depth", "1", spec["url"], spec["revision"]])
            run(["git", "-C", checkout, "checkout", "--detach", "FETCH_HEAD"])
        actual = run(["git", "-C", checkout, "rev-parse", "HEAD"], capture_output=True).stdout.strip()
        if actual != spec["revision"]:
            raise RuntimeError(f"{checkout}: expected {spec['revision']}, found {actual}; refusing to change checkout")
        # Include local patch provenance rather than silently calling modified code upstream.
        patch = run(["git", "-C", checkout, "diff", "HEAD", "--"], capture_output=True).stdout
        if patch:
            raise RuntimeError(f"{checkout}: tracked artifact modifications; refusing an unrecorded variant")


def prepare_pgm():
    """Apply the recorded bounds fix to a generated copy, not the checkout."""
    source = ARTIFACTS / "bench_search" / "exp_pgm"
    destination = TARGET / "pgm-safe"
    destination.mkdir(parents=True, exist_ok=True)
    for name in ("pgm_index.h", "piecewise_linear_model.h", "search_algo.h"):
        shutil.copyfile(source / name, destination / name)
    run(["patch", "--batch", "-p1", "-i", ROOT / "pgm-safe-projection.patch"], cwd=destination)
    return destination


def build(sanitize=False):
    compiler = shlex.split(os.environ.get("CXX", "clang++"))
    flags = ["-std=c++20", "-O3", "-DNDEBUG"]
    flags += ["-mcpu=native"] if platform.machine() == "arm64" else ["-march=native"]
    if sanitize:
        flags = ["-std=c++20", "-O1", "-g", "-fsanitize=address,undefined",
                 "-fno-sanitize-recover=all", "-fno-omit-frame-pointer"]
    includes = [prepare_pgm()]
    includes += [ARTIFACTS / name / "include" for name in ("RadixSpline", "CHT", "unordered_dense")]
    binary = TARGET / ("index-bench-sanitize" if sanitize else "index-bench")
    command = compiler + flags + ["-I" + str(path) for path in includes]
    command += [str(ROOT / "benchmark.cpp"), "-o", str(binary)]
    run(command)
    version = run(compiler + ["--version"], capture_output=True).stdout
    return binary, command, version


def scenarios(quick):
    if quick:
        return [
            (4096, 8192, "uniform", 0.5, 4),
            (4096, 8192, "clustered", 0.1, 8),
        ]
    return [
        (65536, 131072, "uniform", 0.5, 1),
        (1048576, 262144, "uniform", 0.5, 4),
        (1048576, 262144, "uniform", 0.1, 4),
        (1048576, 262144, "uniform", 0.9, 4),
        (1048576, 262144, "clustered", 0.5, 4),
        (4194304, 262144, "clustered", 0.5, 4),
        (1048576, 262144, "uniform", 0.5, 32),
    ]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--quick", action="store_true")
    parser.add_argument("--self-test", action="store_true", help="only run correctness scenarios")
    parser.add_argument("--sanitize", action="store_true", help="ASan/UBSan self-test only")
    parser.add_argument("--reps", type=int, default=5)
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--output", type=Path, help="new results directory; existing paths are not overwritten")
    args = parser.parse_args()
    if args.reps < 1:
        parser.error("--reps must be positive")
    manifest = json.loads((ROOT / "artifacts.json").read_text())
    fetch(manifest)
    binary, command, compiler = build(args.sanitize)
    run([binary, "--self-test"])
    if args.self_test or args.sanitize:
        return
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    output = args.output or ROOT / "results" / (stamp + ("-quick" if args.quick else ""))
    output.mkdir(parents=True, exist_ok=False)
    hardware = {}
    if platform.system() == "Darwin":
        for key in ("machdep.cpu.brand_string", "hw.memsize", "hw.logicalcpu"):
            hardware[key] = run(["sysctl", "-n", key], capture_output=True).stdout.strip()
    metadata = dict(timestamp_utc=stamp, platform=platform.platform(), machine=platform.machine(),
                    hardware=hardware, compiler=compiler, compile_command=command,
                    artifacts=manifest, seed=args.seed, reps=args.reps,
                    scope="Single-thread CPU index construction and joins; no GPU timing or saturation claim",
                    source_sha256={name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest()
                                   for name in ("benchmark.cpp", "indexes.hpp", "run.py", "artifacts.json",
                                                "pgm-safe-projection.patch")},
                    cases=[], complete=False)
    meta_path = output / "metadata.json"
    meta_path.write_text(json.dumps(metadata, indent=2) + "\n")
    combined = []
    for index, (keys, probes, distribution, hit_rate, fanout) in enumerate(scenarios(args.quick)):
        command = [binary, "--keys", keys, "--probes", probes, "--distribution", distribution,
                   "--hit-rate", hit_rate, "--fanout", fanout, "--reps", args.reps,
                   "--seed", args.seed, "--filters", "bloom"]
        stem = f"{index:02d}-{distribution}-k{keys}-q{probes}-h{hit_rate}-f{fanout}"
        try:
            result = run(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        except subprocess.CalledProcessError as error:
            (output / (stem + ".failed.csv")).write_text(error.stdout or "")
            (output / (stem + ".failed.log")).write_text(error.stderr or "")
            sys.stderr.write(error.stderr or "")
            raise
        sys.stderr.write(result.stderr)
        (output / (stem + ".csv")).write_text(result.stdout)
        (output / (stem + ".log")).write_text(result.stderr)
        rows = list(csv.DictReader(io.StringIO(result.stdout)))
        if not rows:
            raise RuntimeError(f"{stem}: benchmark returned no measurements")
        combined.extend(dict(case=stem, **row) for row in rows)
        metadata["cases"].append(dict(name=stem, command=list(map(str, command)), measurements=len(rows)))
        meta_path.write_text(json.dumps(metadata, indent=2) + "\n")
    with (output / "measurements.csv").open("w", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=list(combined[0]))
        writer.writeheader()
        writer.writerows(combined)
    metadata["complete"] = True
    meta_path.write_text(json.dumps(metadata, indent=2) + "\n")
    print(f"Recorded {len(combined)} measurements in {output}")


if __name__ == "__main__":
    main()
