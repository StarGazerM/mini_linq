#!/usr/bin/env python3
"""Reject every negative configuration; pin only construct/trait substrings."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[4]
OUT = Path(sys.argv[1]).resolve()
UI_ROOT = ROOT / "target/workload-ui"
UI_ROOT.mkdir(parents=True, exist_ok=True)
# Gates may invoke this test concurrently with different generated OUT_DIRs.
# Keep manifests and diagnostics private to a run; Cargo locks the shared cache.
DIRECTORY = Path(tempfile.mkdtemp(prefix=OUT.parent.name + "-", dir=UI_ROOT))
CARGO = os.environ.get("CARGO", "cargo")
env = dict(os.environ, CARGO_TARGET_DIR=str(UI_ROOT / "target"), CARGO_TERM_COLOR="never")
manifest = '[package]\nname = "workload-ui"\nversion = "0.0.0"\nedition = "2024"\n[workspace]\n[dependencies]\n'
for name, path in {
    "mini-linq": "facade/mini-linq",
        "cq-storage": "runtime/cq-storage",
        "cq-load": "runtime/cq-load",
    "cq-state-providers": "runtime/cq-state-providers",
    "cq-workloads": "workloads",
    "cq-workload-e2e": "workloads/e2e",
}.items():
    manifest += f'{name} = {{ path = {json.dumps(str(ROOT / path))} }}\n'
fixtures = [(name, Path(__file__).parent / f"{name}.rs", needle)
            for name, needle in [("ordered", "OrderedRead"), ("extension", "name!"), ("provider", "IndexRead")]]
generated = [(p.stem, p.with_suffix(".rs"), None)
             for p in sorted(OUT.glob("rejected_*.tsv")) if p.read_text().strip()]
for name, path, _ in fixtures + generated:
    manifest += f'[[bin]]\nname = "{name}"\npath = {json.dumps(str(path))}\n'
(DIRECTORY / "Cargo.toml").write_text(manifest)
shutil.copy2(ROOT / "Cargo.lock", DIRECTORY / "Cargo.lock")
subprocess.run([CARGO, "metadata", "--offline", "--format-version=1"], cwd=DIRECTORY, env=env, check=True,
               stdout=subprocess.PIPE, stderr=subprocess.PIPE)


def spans(span):
    yield span
    expansion = span.get("expansion")
    if expansion:
        yield from spans(expansion["span"])


def diagnostic_text(error):
    # Custom on_unimplemented messages can put the trait in the primary label
    # or a note. Do not match paths, source excerpts or complete wording.
    return "\n".join([error["message"], *(s.get("label") or "" for s in error["spans"]),
                       *(diagnostic_text(c) for c in error["children"])])


total = 0
for name, path, needle in fixtures + generated:
    result = subprocess.run([CARGO, "check", "--offline", "--locked", "--bin", name, "--message-format=json"],
                            cwd=DIRECTORY, env=env, capture_output=True, text=True)
    (DIRECTORY / f"{name}.jsonl").write_text(result.stdout)
    errors = [m["message"] for line in result.stdout.splitlines()
              if (m := json.loads(line)).get("reason") == "compiler-message" and m["message"]["level"] == "error"]
    if result.returncode == 0 or not errors:
        raise SystemExit(f"{name}: expected a rustc error\n{result.stderr}")
    if needle is not None:
        if not any(needle in diagnostic_text(error) for error in errors):
            raise SystemExit(f"{name}: expected {needle!r}; see {DIRECTORY / (name + '.jsonl')}")
        total += 1
        continue
    expected = [line.split("\t") for line in path.with_suffix(".tsv").read_text().splitlines()]
    for line, label, needle in expected:
        if not any(needle in diagnostic_text(error) and any(
                Path(s["file_name"]).resolve() == path and s["line_start"] <= int(line) <= s["line_end"]
                for span in error["spans"] for s in spans(span)) for error in errors):
            raise SystemExit(f"{label}: no attributed {needle!r} error; see {DIRECTORY / (name + '.jsonl')}")
    total += len(expected)
    print(f"PASS: {name}: {len(expected)} independently attributed rejections", flush=True)
print(f"PASS: {total} compile-fail cases (including three minimal fixtures); logs: {DIRECTORY}", flush=True)
