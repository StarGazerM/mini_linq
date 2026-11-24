#!/usr/bin/env python3
"""Execute one prebuilt default DOOP library using only Python's standard library.

The ABI sequence and fixedpoint boundary match doop_suite/gpu.py::_execute at
SRDatalog fab849f. A report is not success until the worker exits normally.
"""
from __future__ import annotations

import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import time


TIMING_BOUNDARY = (
    "entire unlimited fixedpoint on a prepared device database + checked device "
    "synchronization; excludes initialization, CSV loading, database construction/H2D, "
    "cardinality collection and teardown"
)


def save_json(path: Path, value: dict) -> None:
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True, allow_nan=False) + "\n",
                         encoding="utf-8")
    temporary.replace(path)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def relative_path(root: Path, name: str) -> Path:
    relative = Path(name)
    if not name or relative.is_absolute() or ".." in relative.parts:
        raise ValueError(f"Unsafe relative asset path: {name!r}")
    path = (root / relative).resolve()
    if path == root.resolve() or root.resolve() not in path.parents:
        raise ValueError(f"Asset escapes its root: {name!r}")
    return path


def dataset_schema(entry: dict) -> list[str]:
    arities, inputs = entry["relation_arities"], entry["input_cardinalities"]
    if not isinstance(arities, dict) or not isinstance(inputs, dict) or not inputs:
        raise ValueError("Expected relation_arities and nonempty input_cardinalities maps")
    if any(not isinstance(name, str) or not name or type(arity) is not int or arity < 1
           for name, arity in arities.items()):
        raise ValueError("Invalid relation arity")
    if any(name not in arities or type(count) is not int or count < 0
           for name, count in inputs.items()):
        raise ValueError("Invalid input cardinality")
    derived = sorted(arities.keys() - inputs.keys())
    if len(derived) != 40:
        raise ValueError(f"Expected exactly 40 default-program derived relations, got {len(derived)}")
    return derived


def execute(manifest_path: Path, dataset: str, facts: Path, report_path: Path) -> None:
    report = {
        "status": "running", "dataset": dataset, "stages_seconds": {},
        "relation_counts": {}, "derived_counts": {},
        "memory_resource": os.environ.get("SRDATALOG_RMM_RESOURCE") or "pool",
        "measurement_mode": "fixedpoint-only", "timing_boundary": TIMING_BOUNDARY,
    }
    save_json(report_path, report)
    initialized = False
    library = None
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        if manifest["schema_version"] != 1:
            raise ValueError("Unsupported DOOP manifest schema")
        entry = manifest["datasets"][dataset]
        derived = dataset_schema(entry)
        provenance = entry["prepared_metadata"]
        for filename, expected in (("manifest.json", provenance["manifest_sha256"]),
                                   ("meta.json", provenance["metadata_sha256"])):
            if sha256(facts / filename) != expected:
                raise ValueError(f"Prepared {filename} differs from the compiled library")
        library_path = relative_path(manifest_path.parent, entry["library"])
        if sha256(library_path) != entry["library_sha256"]:
            raise ValueError("SRDatalog library checksum mismatch")
        # Do not dlclose: generated function-local GPU scratch/streams live until exit.
        library = ctypes.CDLL(str(library_path), mode=ctypes.RTLD_GLOBAL)
        signatures = {
            "init": [], "load_all": [ctypes.c_char_p], "prepare": [],
            "run": [ctypes.c_ulonglong], "synchronize": [], "shutdown": [],
            "get_size": [ctypes.c_char_p, ctypes.POINTER(ctypes.c_ulonglong)],
        }
        for name, arguments in signatures.items():
            function = getattr(library, "srdatalog_" + name)
            function.argtypes = arguments
            function.restype = ctypes.c_int

        def checked(name: str, *arguments) -> None:
            code = getattr(library, "srdatalog_" + name)(*arguments)
            if code != 0:
                raise RuntimeError(f"srdatalog_{name} returned {code}")

        started = time.perf_counter()
        checked("init")
        initialized = True
        checked("synchronize")
        report["stages_seconds"]["init"] = time.perf_counter() - started
        started = time.perf_counter()
        checked("load_all", os.fsencode(facts))
        checked("synchronize")
        report["stages_seconds"]["load"] = time.perf_counter() - started
        started = time.perf_counter()
        checked("prepare")
        report["stages_seconds"]["prepare"] = time.perf_counter() - started
        # prepare already synchronizes database construction/H2D. Zero is unlimited.
        started = time.perf_counter()
        checked("run", 0)
        checked("synchronize")
        report["fixpoint_seconds"] = time.perf_counter() - started
        started = time.perf_counter()
        for name in entry["relation_arities"]:
            count = ctypes.c_ulonglong()
            checked("get_size", name.encode(), ctypes.byref(count))
            report["relation_counts"][name] = count.value
        report["stages_seconds"]["counts"] = time.perf_counter() - started
        report["derived_counts"] = {name: report["relation_counts"][name] for name in derived}
        for name, expected in entry["input_cardinalities"].items():
            actual = report["relation_counts"][name]
            if actual != expected:
                raise RuntimeError(f"Loaded input cardinality mismatch for {name}: {actual} != {expected}")
    except BaseException as error:
        report.update(status="failed", error=f"{type(error).__name__}: {error}")
        raise
    finally:
        try:
            if initialized:
                started = time.perf_counter()
                checked("shutdown")
                report["stages_seconds"]["shutdown"] = time.perf_counter() - started
        except BaseException as error:
            report.update(status="failed", shutdown_error=f"{type(error).__name__}: {error}")
            raise
        finally:
            save_json(report_path, report)
    report["status"] = "native_completed"
    save_json(report_path, report)
    print("counts\t" + json.dumps(report["derived_counts"], sort_keys=True), flush=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--dataset", required=True)
    parser.add_argument("--facts", required=True, type=Path)
    parser.add_argument("--report", required=True, type=Path)
    args = parser.parse_args()
    execute(args.manifest.resolve(), args.dataset, args.facts.resolve(), args.report.resolve())


if __name__ == "__main__":
    main()
