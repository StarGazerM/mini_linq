#!/usr/bin/env python3
"""Execution-only, fresh-process comparison of the three default DOOP engines.

Requires packaged binaries/libraries, Python 3, zstd and nvidia-smi, not compilers
or SRDatalog Python packages. Run only the last BUNDLE_DIRS invocation. Results
are count-only validation, never a claim of tuple-set equivalence. Each sample
owns the GPU lock separately; --parent-held-gpu-lock is only for a locking parent.
"""
from __future__ import annotations

import argparse
import contextlib
import csv
from datetime import datetime, timezone
import fcntl
import hashlib
import json
import math
import os
from pathlib import Path, PurePosixPath
import shutil
import signal
import statistics
import subprocess
import sys
import tarfile
import tempfile
import threading
import time

from doop_srd_worker import TIMING_BOUNDARY, dataset_schema, relative_path, save_json


DATASETS = ("bloat", "xalan", "batik", "soot", "javac", "jruby")
ENGINES = ("ours-written", "ours-advised", "SRDatalog")
GPU_LOCK = Path("/tmp/mini_linq-gpu.lock")


def utc() -> str:
    return datetime.now(timezone.utc).isoformat()


def remaining(deadline: float) -> float:
    seconds = deadline - time.monotonic()
    if seconds <= 0:
        raise TimeoutError("Total comparison budget exhausted")
    return seconds


def checksum(path: Path, deadline: float) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            remaining(deadline)
            digest.update(chunk)
    return digest.hexdigest()


@contextlib.contextmanager
def locked(path: Path, deadline: float, parent_held: bool = False):
    started = time.monotonic()
    with path.open("a") as stream:
        if parent_held:
            try:
                fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                yield 0.0
                return
            fcntl.flock(stream, fcntl.LOCK_UN)
            raise RuntimeError("--parent-held-gpu-lock requires an already-held GPU lock")
        while True:
            remaining(deadline)
            try:
                fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except BlockingIOError:
                time.sleep(min(0.05, remaining(deadline)))
        try:
            yield time.monotonic() - started
        finally:
            fcntl.flock(stream, fcntl.LOCK_UN)


def kill_group(process: subprocess.Popen) -> None:
    with contextlib.suppress(ProcessLookupError):
        os.killpg(process.pid, signal.SIGKILL)


def extract_archive(archive: Path, destination: Path, log: Path, deadline: float) -> None:
    """Stream zstd into a regular-file-only extractor; never apply tar ownership/modes."""
    with log.open("xb") as errors:
        process = subprocess.Popen(["zstd", "-dc", "--", str(archive)], stdout=subprocess.PIPE,
                                   stderr=errors, start_new_session=True)
        timer = threading.Timer(remaining(deadline), kill_group, args=(process,))
        timer.daemon = True
        timer.start()
        try:
            with tarfile.open(fileobj=process.stdout, mode="r|") as members:
                for member in members:
                    remaining(deadline)
                    name = PurePosixPath(member.name)
                    if (name.is_absolute() or ".." in name.parts or "\\" in member.name
                            or name == PurePosixPath(".verified.json")):
                        raise ValueError(f"Unsafe archive member: {member.name!r}")
                    if not member.isdir() and not member.isfile():
                        raise ValueError(f"Archive links/special files are forbidden: {member.name!r}")
                    if name == PurePosixPath("."):
                        if not member.isdir():
                            raise ValueError("Archive root is not a directory")
                        continue
                    target = relative_path(destination, member.name)
                    if member.isdir():
                        target.mkdir(parents=True, exist_ok=True)
                        continue
                    target.parent.mkdir(parents=True, exist_ok=True)
                    with members.extractfile(member) as source, target.open("xb") as output:
                        while True:
                            remaining(deadline)
                            chunk = source.read(1024 * 1024)
                            if not chunk:
                                break
                            output.write(chunk)
            # Consume any trailing compressed data, so zstd verifies its checksum
            # and cannot block on a full pipe after tar's end-of-archive marker.
            while process.stdout.read(1024 * 1024):
                remaining(deadline)
            if process.wait(timeout=remaining(deadline)) != 0:
                raise RuntimeError(f"zstd extraction failed; see {log}")
        except BaseException:
            kill_group(process)
            process.wait()
            raise
        finally:
            timer.cancel()
            timer.join()
            process.stdout.close()


def verify_prepared(root: Path, manifest: dict, deadline: float, full: bool) -> None:
    for dataset, entry in manifest["datasets"].items():
        dataset_schema(entry)
        facts = relative_path(root, entry["prepared_dir"])
        provenance = entry["prepared_metadata"]
        original = provenance["manifest"]
        if checksum(facts / "manifest.json", deadline) != provenance["manifest_sha256"]:
            raise ValueError(f"{dataset}: prepared manifest checksum mismatch")
        if json.loads((facts / "manifest.json").read_text()) != original:
            raise ValueError(f"{dataset}: prepared manifest provenance mismatch")
        if checksum(facts / "meta.json", deadline) != provenance["metadata_sha256"]:
            raise ValueError(f"{dataset}: prepared metadata checksum mismatch")
        for name, count in entry["input_cardinalities"].items():
            relation = original["relations"][name]
            if relation["rows"] != count or relation["arity"] != entry["relation_arities"][name]:
                raise ValueError(f"{dataset}: compiled input schema/cardinality differs for {name}")
        for item in [original["metadata"], original["symbols"], *original["relations"].values()]:
            path = relative_path(facts, item["path"])
            if not path.is_file() or path.stat().st_size != item["bytes"]:
                raise ValueError(f"{dataset}: prepared file missing or wrong size: {path}")
            if full and checksum(path, deadline) != item["sha256"]:
                raise ValueError(f"{dataset}: prepared file checksum mismatch: {path}")


def prepared_cache(bundle: Path, manifest: dict, args, deadline: float) -> tuple[Path, dict]:
    spec = manifest["prepared_archive"]
    expected = spec["sha256"]
    if len(expected) != 64 or any(c not in "0123456789abcdef" for c in expected):
        raise ValueError("Invalid prepared archive SHA256")
    archive = (args.prepared_archive.resolve() if args.prepared_archive else
               relative_path(bundle / "doop", spec["path"]))
    started = time.monotonic()
    if checksum(archive, deadline) != expected:
        raise ValueError("Prepared archive SHA256 mismatch")
    cache_parent = args.cache_root.resolve() / "doop-prepared"
    cache_parent.mkdir(parents=True, exist_ok=True)
    root = cache_parent / expected
    marker = {"schema_version": 1, "archive_sha256": expected,
              "prepared_manifests": {name: entry["prepared_metadata"]["manifest_sha256"]
                                     for name, entry in manifest["datasets"].items()}}
    reused = False
    with locked(cache_parent / (expected + ".lock"), deadline):
        if root.exists():
            if not (root / ".verified.json").is_file():
                raise ValueError(f"Existing cache has no verified marker: {root}")
            if json.loads((root / ".verified.json").read_text()) != marker:
                raise ValueError(f"Existing cache marker does not match manifest: {root}")
            verify_prepared(root, manifest, deadline, full=False)
            reused = True
        else:
            staging = Path(tempfile.mkdtemp(prefix=expected + ".", dir=cache_parent))
            try:
                extract_archive(archive, staging, args.results / "extract-zstd.stderr.log", deadline)
                verify_prepared(staging, manifest, deadline, full=True)
                save_json(staging / ".verified.json", marker)
                staging.rename(root)
            finally:
                if staging.exists():
                    shutil.rmtree(staging)
    return root, {"path": str(root), "archive": str(archive), "archive_sha256": expected,
                  "reused_verified_marker": reused, "seconds": time.monotonic() - started,
                  "verification": "archive SHA256; full file hashes on extraction; provenance and sizes on reuse"}


def tenant_snapshot(deadline: float) -> dict:
    command = ["nvidia-smi", "--query-compute-apps=pid,process_name,used_memory",
               "--format=csv,noheader,nounits"]
    visible = os.environ.get("CUDA_VISIBLE_DEVICES")
    if visible:
        command.append("--id=" + visible)
    result = subprocess.run(command, capture_output=True, text=True,
                            timeout=min(5.0, remaining(deadline)))
    snapshot = {"utc": utc(), "argv": command, "exit_code": result.returncode,
                "stdout": result.stdout, "stderr": result.stderr, "processes": []}
    if result.returncode == 0:
        for row in csv.reader(result.stdout.splitlines()):
            if not row or not any(value.strip() for value in row):
                continue
            if len(row) != 3 or not row[0].strip().isdigit():
                snapshot["error"] = "Cannot parse nvidia-smi tenant row: " + repr(row)
                break
            snapshot["processes"].append({"pid": int(row[0]), "name": row[1].strip(),
                                          "used_memory_mib": row[2].strip()})
    else:
        snapshot["error"] = "nvidia-smi tenant query failed"
    return snapshot


def record_tenants(record: dict, phase: str, deadline: float, own_pids=()) -> bool:
    snapshot = tenant_snapshot(deadline)
    snapshot["phase"] = phase
    snapshot["foreign_processes"] = [p for p in snapshot["processes"] if p["pid"] not in own_pids]
    record["tenants"].append(snapshot)
    if snapshot.get("error"):
        raise RuntimeError(snapshot["error"])
    contaminated = bool(snapshot["foreign_processes"])
    if contaminated:
        record["contamination"] = True
    return contaminated


def process_pids(pid: int) -> set[int]:
    pids = {pid}
    # Include outer namespace PIDs when exposed by procfs in a container.
    try:
        for line in Path(f"/proc/{pid}/status").read_text().splitlines():
            if line.startswith("NSpid:"):
                pids.update(map(int, line.split()[1:]))
    except FileNotFoundError:
        pass
    return pids


def execute_sample(record: dict, command: list[str], env: dict, directory: Path,
                   args, deadline: float) -> None:
    record.update(argv=command, started_utc=utc(), tenants=[], contamination=False,
                  exit_code=None, process_seconds=None, peak_rss_kib=None,
                  stdout=str(directory / "stdout.log"), stderr=str(directory / "stderr.log"))
    with locked(GPU_LOCK, deadline, args.parent_held_gpu_lock) as waited:
        record["gpu_lock_wait_seconds"] = waited
        record["load_average_before"] = list(os.getloadavg())
        with Path(record["stdout"]).open("xb") as out, Path(record["stderr"]).open("xb") as err:
            if record_tenants(record, "before", deadline) and not args.allow_contaminated:
                record.update(status="contaminated", error="Foreign GPU tenants before sample")
                return
            started = time.monotonic()
            sample_deadline = min(deadline, started + args.timeout)
            record["effective_timeout_seconds"] = sample_deadline - started
            process = subprocess.Popen(command, cwd=directory, env=env, stdout=out, stderr=err,
                                       start_new_session=True)
            record["pid"] = process.pid
            stop = threading.Event()
            contaminated = threading.Event()
            monitor_errors = []
            own_pids = process_pids(process.pid)

            def monitor() -> None:
                while not stop.is_set():
                    try:
                        if record_tenants(record, "during", sample_deadline, own_pids):
                            contaminated.set()
                    except Exception as error:
                        monitor_errors.append(f"{type(error).__name__}: {error}")
                        return
                    if stop.wait(0.5):
                        return

            thread = threading.Thread(target=monitor, daemon=True)
            thread.start()
            status = usage = None
            try:
                while True:
                    pid, status, usage = os.wait4(process.pid, os.WNOHANG)
                    if pid:
                        break
                    if time.monotonic() >= sample_deadline:
                        record.update(status="timeout", error="Per-sample timeout or total budget exhausted")
                        kill_group(process)
                    elif monitor_errors:
                        record.update(status="failed", error="GPU tenant monitoring failed")
                        kill_group(process)
                    elif contaminated.is_set() and not args.allow_contaminated:
                        record.update(status="contaminated", error="Foreign GPU tenant during sample")
                        kill_group(process)
                    else:
                        time.sleep(0.02)
                        continue
                    _, status, usage = os.wait4(process.pid, 0)
                    break
            except BaseException:
                kill_group(process)
                _, status, usage = os.wait4(process.pid, 0)
                raise
            finally:
                record["process_seconds"] = time.monotonic() - started
                if status is not None and usage is not None:
                    process.returncode = os.waitstatus_to_exitcode(status)
                    record.update(exit_code=process.returncode, wait_status=status,
                                  peak_rss_kib=usage.ru_maxrss,
                                  resources={"scope": "fresh child process; Linux wait4 ru_maxrss (KiB)",
                                             "minor_page_faults": usage.ru_minflt,
                                             "major_page_faults": usage.ru_majflt})
                stop.set()
                thread.join()
            if monitor_errors:
                record["monitor_errors"] = monitor_errors
                if record["status"] == "running":
                    record.update(status="failed", error="GPU tenant monitoring failed")
            if record_tenants(record, "after", deadline) and not args.allow_contaminated:
                record.update(status="contaminated", error="Foreign GPU tenants after sample")
            if record["status"] == "running":
                record["status"] = "passed" if process.returncode == 0 else "failed"
                if process.returncode:
                    record["error"] = f"Worker exited {process.returncode}"
        record["timing_eligible"] = record["status"] == "passed" and not record["contamination"]


def parse_sample(record: dict, directory: Path) -> None:
    if record["engine"] == "SRDatalog":
        report_path = directory / "native.json"
        if not report_path.is_file():
            raise ValueError("SRDatalog did not write a native report")
        native = json.loads(report_path.read_text())
        record["native_report"] = str(report_path)
        record["native"] = native
        record["relation_counts"] = native.get("relation_counts", {})
        record["derived_counts"] = native.get("derived_counts", {})
        record["fixpoint_seconds"] = native.get("fixpoint_seconds")
        if native.get("status") != "native_completed":
            raise ValueError("SRDatalog native execution/shutdown did not complete")
    else:
        counts = []
        phases = {}
        with Path(record["stdout"]).open(errors="replace") as source:
            for line in source:
                if line.startswith("counts\t"):
                    counts.append(json.loads(line.split("\t", 1)[1]))
        with Path(record["stderr"]).open(errors="replace") as source:
            for line in source:
                parts = line.strip().split("\t")
                if len(parts) == 3 and parts[0] == "phase" and parts[1] in ("setup", "fixpoint", "output"):
                    if parts[1] in phases:
                        raise ValueError(f"Repeated GPU phase: {parts[1]}")
                    phases[parts[1]] = float(parts[2])
        if len(counts) != 1:
            raise ValueError(f"Expected one complete counts record, got {len(counts)}")
        record.update(relation_counts=counts[0], derived_counts=counts[0], phases_seconds=phases,
                      fixpoint_seconds=phases.get("fixpoint"))
    seconds = record["fixpoint_seconds"]
    if type(seconds) not in (int, float) or not math.isfinite(seconds) or seconds < 0:
        raise ValueError("Missing/invalid synchronized fixpoint seconds")


def compare_counts(record: dict, expected: list[str], references: dict) -> None:
    counts = record["derived_counts"]
    if not isinstance(counts, dict) or any(type(n) is not int or n < 0 for n in counts.values()):
        raise ValueError("Invalid derived cardinality map")
    missing, extra = sorted(set(expected) - counts.keys()), sorted(counts.keys() - set(expected))
    if missing or extra or len(counts) != 40:
        record["mismatch"] = {"missing_relations": missing, "unexpected_relations": extra,
                              "expected_relation_count": 40, "actual_relation_count": len(counts)}
        raise ValueError("Derived relation names do not match the default 40-relation program")
    reference = references.setdefault(record["dataset"], {"sample": record["id"], "counts": counts})
    different = {name: {"reference": reference["counts"][name], "actual": counts[name]}
                 for name in expected if reference["counts"][name] != counts[name]}
    record["count_reference_sample"] = reference["sample"]
    if different:
        record["mismatch"] = {"reference_sample": reference["sample"], "relations": different}
        raise ValueError(f"{len(different)} derived relation cardinalities differ")
    record["count_validation"] = "all 40 derived names/counts agree; not tuple-set equality"


def positive_float(value: str) -> float:
    number = float(value)
    if not math.isfinite(number) or number <= 0:
        raise argparse.ArgumentTypeError("must be finite and positive")
    return number


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--results", type=Path, required=True)
    parser.add_argument("--datasets", nargs="+", choices=DATASETS, default=list(DATASETS))
    parser.add_argument("--repetitions", type=int, default=3)
    parser.add_argument("--timeout", type=positive_float, default=100.0,
                        help="hard child lifetime limit per sample, in seconds (default: 100)")
    parser.add_argument("--budget", type=positive_float, default=3600.0,
                        help="total seconds, including cache preparation and lock waits")
    parser.add_argument("--cache-root", type=Path, default=Path("/workspace/data"))
    parser.add_argument("--prepared-archive", type=Path,
                        help="existing volume archive; still checked against manifest SHA256")
    parser.add_argument("--parent-held-gpu-lock", action="store_true")
    parser.add_argument("--allow-contaminated", action="store_true",
                        help="explicit correctness-only override; retain tenants and invalidate timing comparisons")
    parser.add_argument("--profile", action="store_true",
                        help="set GPU_DATALOG_PROFILE=1: per-phase records in each sample's stderr; "
                             "boundaries synchronize, so fixpoint seconds are not timing samples")
    args = parser.parse_args()
    if args.repetitions < 1 or len(set(args.datasets)) != len(args.datasets):
        parser.error("repetitions must be positive and datasets must not repeat")
    if not os.environ.get("BUNDLE_DIR"):
        parser.error("BUNDLE_DIR must identify the execution bundle")
    bundle = Path(os.environ["BUNDLE_DIR"]).resolve()
    directories = json.loads(os.environ.get("BUNDLE_DIRS", json.dumps([str(bundle)])))
    if not isinstance(directories, list) or not directories:
        parser.error("BUNDLE_DIRS must be a nonempty JSON array")
    if bundle != Path(directories[-1]).resolve():
        print("doop-pod: deferred to the last bundle invocation", flush=True)
        return 0
    started = time.monotonic()
    deadline = started + args.budget
    args.results = args.results.resolve()
    args.results.mkdir(parents=True, exist_ok=True)
    result_path = args.results / "doop-pod.json"
    if result_path.exists():
        parser.error(f"Refusing to overwrite existing results: {result_path}")
    data = {"schema_version": 1, "status": "running", "complete": False,
            "started_utc": utc(), "bundle": str(bundle), "bundle_dirs": directories,
            "datasets": args.datasets, "repetitions": args.repetitions,
            "timeout_seconds": args.timeout, "budget_seconds": args.budget,
            "validation": "count-only; exactly 40 derived relations; not tuple-set equivalence",
            "timing_boundary": TIMING_BOUNDARY,
            "order": "repetition-major, dataset-major; engine order rotates by repetition + dataset index",
            "gpu_lock": str(GPU_LOCK), "parent_held_gpu_lock": args.parent_held_gpu_lock,
            "allow_contaminated": args.allow_contaminated, "contamination": False,
            "samples": [], "references": {}, "summary": {}}

    def save() -> None:
        data["elapsed_seconds"] = time.monotonic() - started
        data["contamination"] = any(s.get("contamination", False) for s in data["samples"])
        data["timings_comparable"] = not data["contamination"] and data["complete"]
        save_json(result_path, data)

    def interrupted(signum, frame) -> None:
        raise InterruptedError(f"Received signal {signum}")

    signal.signal(signal.SIGTERM, interrupted)
    save()
    try:
        manifest_path = bundle / "doop" / "manifest.json"
        manifest = json.loads(manifest_path.read_text())
        if manifest["schema_version"] != 1:
            raise ValueError("Unsupported DOOP manifest schema")
        data["manifest"] = manifest
        data["manifest_sha256"] = checksum(manifest_path, deadline)
        expected = {name: dataset_schema(manifest["datasets"][name]) for name in args.datasets}
        if any(names != expected[args.datasets[0]] for names in expected.values()):
            raise ValueError("Datasets disagree about the 40 default-program derived relation names")
        binaries = {engine: bundle / "bin" / engine for engine in ENGINES[:2]}
        for binary in binaries.values():
            if not binary.is_file() or not os.access(binary, os.X_OK):
                raise ValueError(f"Missing executable: {binary}")
        data["binary_sha256"] = {name: checksum(path, deadline) for name, path in binaries.items()}
        facts_root, data["cache"] = prepared_cache(bundle, manifest, args, deadline)
        env = {key: value for key, value in os.environ.items() if not key.startswith("BENCH_CHECK_")}
        env.pop("DOOP_SET_MANIFEST", None)
        env.update(GPU_DATALOG_PHASES="1", PYTHONDONTWRITEBYTECODE="1")
        if args.profile:
            env["GPU_DATALOG_PROFILE"] = "1"
        env.setdefault("SRDATALOG_RMM_RESOURCE", "cuda_async")
        env["LD_LIBRARY_PATH"] = ":".join([str(bundle / "doop" / "lib"), str(bundle / "lib"),
                                          "/usr/local/nvidia/lib64", "/usr/local/nvidia/lib"])
        data["environment"] = {key: env[key] for key in
                               ("CUDA_VISIBLE_DEVICES", "LD_LIBRARY_PATH", "GPU_DATALOG_PHASES", "GPU_DATALOG_PROFILE",
                                "SRDATALOG_RMM_RESOURCE", "RAYON_NUM_THREADS") if key in env}
        info = subprocess.run(["nvidia-smi"], capture_output=True, text=True,
                              timeout=min(10.0, remaining(deadline)))
        data["nvidia_smi"] = {"stdout": info.stdout, "stderr": info.stderr, "exit_code": info.returncode}
        if info.returncode:
            raise RuntimeError("nvidia-smi failed")
        save()
        for repetition in range(args.repetitions):
            for dataset_index, dataset in enumerate(args.datasets):
                shift = (repetition + dataset_index) % len(ENGINES)
                for engine in ENGINES[shift:] + ENGINES[:shift]:
                    remaining(deadline)
                    sample_id = f"{repetition:02d}-{dataset}-{engine}"
                    directory = args.results / "samples" / sample_id
                    directory.mkdir(parents=True, exist_ok=False)
                    record = {"id": sample_id, "dataset": dataset, "engine": engine,
                              "repetition": repetition, "status": "running"}
                    data["samples"].append(record)
                    save()
                    entry = manifest["datasets"][dataset]
                    facts = relative_path(facts_root, entry["prepared_dir"])
                    if engine == "SRDatalog":
                        command = [sys.executable, "-B", str(Path(__file__).with_name("doop_srd_worker.py").resolve()),
                                   "--manifest", str(manifest_path), "--dataset", dataset,
                                   "--facts", str(facts), "--report", str(directory / "native.json")]
                    else:
                        command = [str(binaries[engine]), str(facts), "--gpu-only"]
                    try:
                        execute_sample(record, command, env, directory, args, deadline)
                        if record.get("exit_code") is not None:
                            try:
                                parse_sample(record, directory)
                                if record["status"] == "passed":
                                    compare_counts(record, expected[dataset], data["references"])
                            except Exception as error:
                                record["validation_error"] = f"{type(error).__name__}: {error}"
                                if record["status"] == "passed":
                                    record["status"] = "mismatch" if "mismatch" in record else "failed"
                    except BaseException as error:
                        record.update(status="failed", error=f"{type(error).__name__}: {error}")
                        raise
                    finally:
                        record["timing_eligible"] = (record["status"] == "passed"
                                                      and not record.get("contamination", False))
                        save()
                    print(f"{sample_id}: {record['status']} fixpoint={record.get('fixpoint_seconds')} "
                          f"wall={record.get('process_seconds')} exit={record.get('exit_code')}", flush=True)
                    if record["status"] == "contaminated":
                        raise RuntimeError("Refusing contaminated GPU comparison; raw tenant evidence retained")
        remaining(deadline)
        for dataset in args.datasets:
            summary = data["summary"][dataset] = {}
            for engine in ENGINES:
                samples = [s for s in data["samples"] if s["dataset"] == dataset and s["engine"] == engine]
                valid = [s for s in samples if s["status"] == "passed"]
                summary[engine] = {"successful_repetitions": len(valid), "requested_repetitions": args.repetitions}
                if len(valid) == args.repetitions and not any(s["contamination"] for s in valid):
                    summary[engine].update(
                        median_fixpoint_seconds=statistics.median(s["fixpoint_seconds"] for s in valid),
                        median_process_seconds=statistics.median(s["process_seconds"] for s in valid),
                        max_peak_rss_kib=max(s["peak_rss_kib"] for s in valid))
        data["complete"] = (len(data["samples"]) == len(args.datasets) * args.repetitions * len(ENGINES)
                            and all(s["status"] == "passed" for s in data["samples"]))
        data["status"] = ("passed-contaminated" if data["contamination"] else "passed") if data["complete"] else "failed"
    except BaseException as error:
        data.update(status="failed", complete=False, error=f"{type(error).__name__}: {error}")
        print(data["error"], file=sys.stderr, flush=True)
    finally:
        data["finished_utc"] = utc()
        data["unsuccessful_samples"] = [s["id"] for s in data["samples"] if s["status"] != "passed"]
        data["missing_sample_count"] = len(args.datasets) * args.repetitions * len(ENGINES) - len(data["samples"])
        save()
    print(f"doop-pod: {data['status']}; {result_path}", flush=True)
    return 0 if data["complete"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
