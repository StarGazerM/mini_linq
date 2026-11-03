#!/usr/bin/env python3
"""Check invalid recorded contracts and ordered-fallback compatibility.

An actual native window checks its declared OrderedRead capability. Merge
fixtures instead compile and execute their selected ordered fallbacks.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

from consumer import dependencies, workspace_metadata

CARGO = os.environ.get("CARGO", "cargo")
metadata = workspace_metadata(CARGO, os.environ)
# Keep generated sources under Cargo's workspace configuration even when its
# build directory is external (as in the packaged-consumer gate).
UI_ROOT = Path(metadata["workspace_root"]) / "target/workload-ui"
UI_ROOT.mkdir(parents=True, exist_ok=True)
# Keep manifests and diagnostics private to each invocation.
DIRECTORY = Path(tempfile.mkdtemp(prefix="fixtures-", dir=UI_ROOT))
# Resolve relative overrides in the caller's cwd, before Cargo runs in DIRECTORY.
target = Path(os.environ.get("CARGO_TARGET_DIR", UI_ROOT / "target"))
if not target.is_absolute():
    target = Path.cwd() / target
target = target / "ui"
env = dict(os.environ, CARGO_TARGET_DIR=str(target), CARGO_TERM_COLOR="never")
manifest = '[package]\nname = "workload-ui"\nversion = "0.0.0"\nedition = "2024"\n[workspace]\n[dependencies]\n'
manifest += dependencies(metadata, ["mini-linq", "cq-storage", "cq-load", "cq-workloads", "cq-planners", "cq-rewrites", "cq-workload-passes", "own-push"])
fixtures = [(name, Path(__file__).parent / f"{name}.rs", executes)
            for name, executes in [
                ("ordered", False),
                ("le_ordered", True),
                ("le_runs_ordered", True),
                ("leading_prefix", True),
                ("push_merge", False),
                ("push_smallest", False),
                ("extension", False),
                ("provider", False),
                ("comparison_ord", False),
                ("comparison_literal_eq", False),
                ("smallest_layout", False),
            ]]
for name, path, _ in fixtures:
    manifest += f'[[bin]]\nname = "{name}"\npath = {json.dumps(str(path))}\n'
(DIRECTORY / "Cargo.toml").write_text(manifest)
shutil.copy2(Path(metadata["workspace_root"]) / "Cargo.lock", DIRECTORY / "Cargo.lock")
subprocess.run([CARGO, "metadata", "--offline", "--format-version=1"], cwd=DIRECTORY, env=env, check=True,
               stdout=subprocess.PIPE, stderr=subprocess.PIPE)

total = 0
for name, path, executes in fixtures:
    command = "run" if executes else "check"
    result = subprocess.run([CARGO, command, "--offline", "--locked", "--bin", name, "--message-format=json"],
                            cwd=DIRECTORY, env=env, capture_output=True, text=True)
    (DIRECTORY / f"{name}.jsonl").write_text(result.stdout)
    if executes:
        if result.returncode != 0:
            raise SystemExit(f"{name}: expected successful execution\n{result.stderr}")
        total += 1
        print(f"PASS: {name}: ordered fallback executes", flush=True)
        continue
    errors = [m["message"] for line in result.stdout.splitlines()
              if (m := json.loads(line)).get("reason") == "compiler-message" and m["message"]["level"] == "error"]
    if result.returncode == 0 or not errors:
        raise SystemExit(f"{name}: expected a rustc error\n{result.stderr}")
    total += 1
    print(f"PASS: {name}: compilation rejected", flush=True)
print(f"PASS: {total} handpicked contract cases; logs: {DIRECTORY}", flush=True)
