#!/usr/bin/env python3
"""Reject handpicked invalid programs; pin each construct/trait obligation."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

from consumer import WORKSPACE as ROOT, dependencies
UI_ROOT = ROOT / "target/workload-ui"
UI_ROOT.mkdir(parents=True, exist_ok=True)
# Keep manifests and diagnostics private to each invocation.
DIRECTORY = Path(tempfile.mkdtemp(prefix="fixtures-", dir=UI_ROOT))
CARGO = os.environ.get("CARGO", "cargo")
# Resolve relative overrides in the caller's cwd, before Cargo runs in DIRECTORY.
target = Path(os.environ.get("CARGO_TARGET_DIR", UI_ROOT / "target"))
if not target.is_absolute():
    target = Path.cwd() / target
target = target / "ui"
env = dict(os.environ, CARGO_TARGET_DIR=str(target), CARGO_TERM_COLOR="never")
manifest = '[package]\nname = "workload-ui"\nversion = "0.0.0"\nedition = "2024"\n[workspace]\n[dependencies]\n'
manifest += dependencies(CARGO, ["mini-linq", "cq-storage", "cq-load", "cq-workloads", "cq-planners", "cq-rewrites", "cq-workload-passes", "own-push"], env)
fixtures = [(name, Path(__file__).parent / f"{name}.rs", needle)
            for name, needle in [
                ("ordered", "OrderedRead"),
                ("leading_prefix", "leading declaration prefix"),
                ("push_merge", "own_push: merge is unsupported"),
                ("push_smallest", "own_push: probe smallest is unsupported"),
                ("extension", "name!"),
                ("provider", "IndexRead"),
                ("comparison_ord", "Ord"),
                ("comparison_literal_eq", "mismatched types"),
                ("smallest_layout", "smallest requires a two-level candidate structure on each retained side"),
            ]]
for name, path, _ in fixtures:
    manifest += f'[[bin]]\nname = "{name}"\npath = {json.dumps(str(path))}\n'
(DIRECTORY / "Cargo.toml").write_text(manifest)
shutil.copy2(ROOT / "Cargo.lock", DIRECTORY / "Cargo.lock")
subprocess.run([CARGO, "metadata", "--offline", "--format-version=1"], cwd=DIRECTORY, env=env, check=True,
               stdout=subprocess.PIPE, stderr=subprocess.PIPE)



def diagnostic_text(error):
    # Custom on_unimplemented messages can put the trait in the primary label
    # or a note. Do not match paths, source excerpts or complete wording.
    return "\n".join([error["message"], *(s.get("label") or "" for s in error["spans"]),
                       *(diagnostic_text(c) for c in error["children"])])


total = 0
for name, path, needle in fixtures:
    result = subprocess.run([CARGO, "check", "--offline", "--locked", "--bin", name, "--message-format=json"],
                            cwd=DIRECTORY, env=env, capture_output=True, text=True)
    (DIRECTORY / f"{name}.jsonl").write_text(result.stdout)
    errors = [m["message"] for line in result.stdout.splitlines()
              if (m := json.loads(line)).get("reason") == "compiler-message" and m["message"]["level"] == "error"]
    if result.returncode == 0 or not errors:
        raise SystemExit(f"{name}: expected a rustc error\n{result.stderr}")
    if not any(needle in diagnostic_text(error) for error in errors):
        raise SystemExit(f"{name}: expected {needle!r}; see {DIRECTORY / (name + '.jsonl')}")
    total += 1
    print(f"PASS: {name}: {needle}", flush=True)
print(f"PASS: {total} handpicked compile-fail cases; logs: {DIRECTORY}", flush=True)
