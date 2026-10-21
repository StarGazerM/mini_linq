#!/usr/bin/env python3
"""Check Fx and keyed full answers, then compare isolated interleaved processes.

Build doop-parallel in release mode without counters/timing features first.
Keep datalog-parallel-doop's default engine features enabled: --check requires
all six (mini,mini-keyed,ascent,mini-par,mini-par-keyed,ascent-par).
Each feature selects its own engine crate; phase-timing is an optional forward
to enabled engines and must remain disabled for these measurements.
Run with no other benchmark or build workload on the machine:
  python3 doop/hash_sweep.py --binary /tmp/target-hashers/release/doop-parallel
The same executable supplies sequential and parallel engines; facts loading is
excluded from compute time. Three samples per configuration alternate AB/BA/AB.
mini/mini-par now use Fx; mini-keyed/mini-par-keyed use keyed aHash.
The checked-in hasher CSV/JSON files retain historical dadaed6 measurements:
there mini/mini-par meant aHash and mini-fx/mini-par-fx meant opt-in Fx.
This script produces new evidence and does not relabel those historical files.
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--cache", type=Path, default=Path("/tmp/doop-data"))
    parser.add_argument("--output", type=Path, default=Path("/tmp") / (
        "doop-hashers-" + datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")))
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    cache = args.cache.resolve(strict=True)
    for app in APPS:
        if not (cache / app / "MainClass.facts").is_file():
            parser.error(f"missing facts: {cache / app}")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    (output / "environment.txt").write_text(
        f"platform={platform.platform()}\nbinary={binary}\ncache={cache}\n"
        "repetitions=3\norder=AB,BA,AB\nthreads=1,15\n"
        "engines=mini:fx,mini-keyed:ahash,mini-par:fx,mini-par-keyed:ahash\n"
        "affinity=OS scheduled; no pinning\nfeatures=no counters or phase/hole timing\n")
    environment = dict(os.environ, RAYON_NUM_THREADS="15")

    def invoke(app, label, engine=None):
        command = [str(binary), str(cache / app), "--threads", "15"]
        command += ["--engine", engine] if engine else ["--check"]
        with (output / f"{label}.stdout").open("w") as stdout, \
                (output / f"{label}.stderr").open("w") as stderr:
            subprocess.run(command, env=environment, stdout=stdout, stderr=stderr, check=True)
        text = (output / f"{label}.stdout").read_text()
        if engine is None:
            for name, threads in (("mini", 1), ("mini-keyed", 1),
                                  ("mini-par", 15), ("mini-par-keyed", 15)):
                marker = (f"check=ok engine={name} threads={threads} "
                          "relations=26 comparison=full_tuple_sets")
                if marker not in text:
                    raise RuntimeError(f"missing equality marker: {label}: {name}")
            return None
        measured = re.search(r"compute_seconds=([0-9.]+) peak_rss_bytes=(\d+)", text)
        if measured is None:
            raise RuntimeError(f"missing timing/RSS: {label}")
        return float(measured[1]), int(measured[2])

    for app in APPS:
        invoke(app, f"check-{app}")
        print(f"exactness {app}: Fx and keyed, sequential and 15 workers, all 26 relations", flush=True)
    measurements = {}
    with (output / "samples.csv").open("w", newline="") as samples:
        writer = csv.writer(samples)
        writer.writerow(("app", "engine", "threads", "repetition", "compute_seconds", "rss_bytes"))
        for app in APPS:
            for base, threads in (("mini", 1), ("mini-par", 15)):
                pair = (base + "-keyed", base)
                for repetition in range(3):
                    for engine in pair if repetition % 2 == 0 else reversed(pair):
                        seconds, rss = invoke(app, f"{app}-{engine}-r{repetition}", engine)
                        measurements.setdefault((app, engine, threads), []).append(seconds)
                        writer.writerow((app, engine, threads, repetition, seconds, rss))
                        samples.flush()
                print(f"measured {app}: {threads} workers", flush=True)
    with (output / "summary.csv").open("w", newline="") as summary:
        writer = csv.writer(summary)
        writer.writerow(("app", "threads", "ahash_median_seconds", "fx_median_seconds", "fx_speedup"))
        for app in APPS:
            for base, threads in (("mini", 1), ("mini-par", 15)):
                ahash = statistics.median(measurements[app, base + "-keyed", threads])
                fx = statistics.median(measurements[app, base, threads])
                row = (app, threads, ahash, fx, ahash / fx)
                writer.writerow(row)
                print("\t".join(map(str, row)))
    print(f"raw logs, samples and summary: {output}")


if __name__ == "__main__":
    main()
