#!/usr/bin/env python3
"""Verify that public facades have no Cargo feature switches, then test them."""
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--no-deps", "--format-version=1", "--locked"], cwd=ROOT))
for package in metadata["packages"]:
    if Path(package["manifest_path"]).parent.parent.name == "facade":
        if package["features"]:
            raise SystemExit(f"facade feature flags are forbidden: {package['name']}")
        subprocess.run(["cargo", "test", "-p", package["name"], "--locked"], cwd=ROOT, check=True)
print("PASS: facades use explicit IR pipelines without Cargo feature flags")
