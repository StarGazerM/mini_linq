#!/usr/bin/env python3
"""Execute the independent surface and enforce its downstream-only boundary."""
import json
from pathlib import Path
import subprocess
try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
EXPECTED = {
    "cq-lang-ix": "languages", "cq-lang-physical": "languages",
    "cq-lang-structures": "languages", "cq-syntax": "languages",
    "cq-physical-lowering": "passes", "cq-structure-selection": "passes",
    "cq-physical-realization": "passes", "cq-print": "passes", "cq-storage": "runtime",
}
lock = tomllib.loads((HERE / "Cargo.lock").read_text())
assert {p["name"] for p in lock["package"] if p["name"].startswith("cq-")} == set(EXPECTED)
data = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--format-version=1", "--locked", "--offline"], cwd=HERE, text=True))
by_id = {p["id"]: p for p in data["packages"]}
for node in data["resolve"]["nodes"]:
    package = by_id[node["id"]]
    name = package["name"]
    if name in EXPECTED:
        assert Path(package["manifest_path"]).resolve() == ROOT / EXPECTED[name] / name / "Cargo.toml", name
        assert not set(node["features"]) & {"reference", "reference-prefix"}, (name, node["features"])
subprocess.run(["cargo", "test", "--workspace", "--locked", "--offline"], cwd=HERE, check=True)
subprocess.run(["cargo", "run", "--locked", "--offline"], cwd=HERE, check=True)
print("PASS: independent theta_join; original downstream crates only; no reference prefix")
