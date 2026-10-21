#!/usr/bin/env python3
"""Check all six DOOP applications, then measure isolated processes on macOS.

Build binaries separately; this runner never builds or modifies cached facts.
Build datalog-parallel-doop with its default engine features: this runner uses
--check, which requires mini,mini-keyed,ascent,mini-par,mini-par-keyed,ascent-par.
A --no-default-features single-engine binary cannot perform this comparison.
The CLI selects separate engine crates and shares doop-shared with the base CLI;
it does not depend on the base datalog-doop CLI package.
Example (from features/datalog-parallel):
  python3 doop/sweep.py --binary /tmp/target-ext/release/doop-parallel \
      --baseline /tmp/target-baseline/release/doop

The baseline must be the sequential pre-change binary from the archived source.
Current mini/mini-par measurements use Fx by default; the archived baseline's
hashing policy belongs to its source revision. Use hash_sweep.py for a paired
Fx/keyed comparison within one current binary.
Use an uninstrumented release binary for timing; use a separate phase-timing
build/output directory to collect per-round phase lines in the retained stderr.
5P+10E is a heterogeneous CPU description, not an affinity guarantee: Rayon
worker counts do not pin workers to performance or efficiency cores.
"""

import argparse
import csv
from datetime import datetime, timezone
import os
from pathlib import Path
import platform
import re
import statistics
import subprocess

APPS = ("cassandra", "avrora", "lusearch", "xalan", "tomcat", "zxing")
BASELINE_APPS = ("cassandra", "lusearch", "xalan")
THREADS = (1, 2, 4, 5, 8, 15)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--cache", type=Path, default=Path("/tmp/doop-data"))
    parser.add_argument("--output", type=Path, default=Path("/tmp") / (
        "doop-par-" + datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")))
    parser.add_argument("--repetitions", type=int, default=5)
    parser.add_argument("--topology", default="5P+10E")
    args = parser.parse_args()
    if platform.system() != "Darwin":
        parser.error("this RSS runner requires macOS /usr/bin/time -l")
    if args.repetitions < 1:
        parser.error("--repetitions must be positive")
    binary = args.binary.expanduser().resolve(strict=True)
    baseline = args.baseline.expanduser().resolve(strict=True)
    cache = args.cache.expanduser().resolve(strict=True)
    for app in APPS:
        if not (cache / app / "MainClass.facts").is_file():
            parser.error(f"missing facts: {cache / app}; fetch separately with ../datalog/doop/fetch.py")
    output = args.output.expanduser().resolve()
    if output == cache or cache in output.parents:
        parser.error("--output must be outside the external facts cache")
    output.mkdir(parents=True, exist_ok=False)
    (output / "environment.txt").write_text(
        f"platform={platform.platform()}\ntopology={args.topology}\n"
        f"binary={binary}\nbaseline={baseline}\ncache={cache}\n"
        "threads=1,2,4,5,8,15\naffinity=OS scheduled; no P/E pinning\n"
        f"repetitions={args.repetitions}\n")

    def invoke(executable, app, engine, threads, label, check=False):
        command = ["/usr/bin/time", "-l", str(executable), str(cache / app)]
        if executable == binary:
            command += ["--threads", str(threads)]
        command += ["--check"] if check else ["--engine", engine]
        environment = dict(os.environ, RAYON_NUM_THREADS=str(threads))
        with (output / f"{label}.stdout").open("w") as stdout, \
                (output / f"{label}.stderr").open("w") as stderr:
            subprocess.run(command, env=environment, stdout=stdout, stderr=stderr, check=True)
        text = (output / f"{label}.stdout").read_text()
        if check:
            for name, width in (("mini", 1), ("mini-keyed", 1),
                                ("mini-par", threads), ("mini-par-keyed", threads),
                                ("ascent-par", threads)):
                expected = (f"check=ok engine={name} threads={width} "
                            "relations=26 comparison=full_tuple_sets")
                if expected not in text:
                    raise RuntimeError(f"missing full tuple equality marker: {label}: {name}")
            return None
        seconds = re.search(r"compute_seconds=([0-9.]+)", text)
        rss = re.search(r"(\d+)\s+maximum resident set size",
                        (output / f"{label}.stderr").read_text())
        if seconds is None or rss is None:
            raise RuntimeError(f"missing compute/RSS measurement in {label}")
        return float(seconds[1]), int(rss[1])

    # Equality is a separate phase: never mix its multi-engine RSS with timings.
    for app in APPS:
        for threads in THREADS:
            invoke(binary, app, "", threads, f"check-{app}-t{threads}", check=True)
    print("exact equality: all 26 relations, six apps, six thread counts", flush=True)
    measurements = {}
    with (output / "samples.csv").open("w", newline="") as samples:
        writer = csv.writer(samples)
        writer.writerow(("app", "engine", "threads", "repetition", "compute_seconds", "rss_bytes"))
        for app in APPS:
            jobs = [("mini", 1, binary), ("ascent", 1, binary)]
            if app in BASELINE_APPS:
                jobs.insert(0, ("baseline", 1, baseline))
            jobs += [(engine, n, binary) for engine in ("mini-par", "ascent-par") for n in THREADS]
            for engine, threads, executable in jobs:
                values = []
                for repetition in range(args.repetitions):
                    value = invoke(executable, app, "mini" if engine == "baseline" else engine,
                                   threads, f"{app}-{engine}-t{threads}-r{repetition}")
                    values.append(value)
                    writer.writerow((app, engine, threads, repetition, *value))
                    samples.flush()
                measurements[app, engine, threads] = (
                    statistics.median(v[0] for v in values), max(v[1] for v in values))
    with (output / "summary.csv").open("w", newline="") as summary:
        writer = csv.writer(summary)
        writer.writerow(("topology", "app", "engine", "threads", "median_seconds", "max_rss_bytes",
                         "baseline_speedup", "sequential_speedup", "par1_speedup", "par1_efficiency"))
        for (app, engine, threads), (seconds, rss) in measurements.items():
            old = measurements.get((app, "baseline", 1))
            sequential = measurements[app, "ascent" if engine.startswith("ascent") else "mini", 1][0]
            par1 = measurements[app, "ascent-par" if engine.startswith("ascent") else "mini-par", 1][0]
            row = (args.topology, app, engine, threads, seconds, rss,
                   old[0] / seconds if old and seconds else "NA",
                   sequential / seconds if seconds else "NA",
                   par1 / seconds if seconds else "NA",
                   par1 / seconds / threads if seconds else "NA")
            writer.writerow(row)
            print("\t".join(map(str, row)))
    print(f"raw logs, samples, and summary: {output}")


if __name__ == "__main__":
    main()
