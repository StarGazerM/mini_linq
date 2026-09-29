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
    "cq-lang-ra": "languages", "cq-lang-ix": "languages", "cq-lang-physical": "languages",
    "cq-lang-structures": "languages", "cq-syntax": "languages",
    "cq-physical-lowering": "passes", "cq-structure-selection": "passes",
    "cq-physical-realization": "passes", "cq-print": "passes", "cq-storage": "runtime",
}
lock = tomllib.loads((HERE / "Cargo.lock").read_text())
allowed = set(EXPECTED) | {"ra-theta", "diverged-ra"}
forbidden = {"cq-lang-cq", "cq-lang-vo", "cq-order", "cq-reduce", "cq-elaborate", "cq-index-policy", "mini-linq"}
assert not {p["name"] for p in lock["package"]} & forbidden
assert {p["name"] for p in lock["package"] if "source" not in p} == allowed
data = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--format-version=1", "--locked", "--offline"], cwd=HERE, text=True))
by_id = {p["id"]: p for p in data["packages"]}
for node in data["resolve"]["nodes"]:
    package = by_id[node["id"]]
    name = package["name"]
    if package["source"] is None:
        assert name in allowed, name
    if name == "ra-theta":
        assert Path(package["manifest_path"]).resolve() == HERE / "ra-theta/Cargo.toml"
    if name in EXPECTED:
        assert Path(package["manifest_path"]).resolve() == ROOT / EXPECTED[name] / name / "Cargo.toml", name
subprocess.run(["cargo", "test", "--workspace", "--locked", "--offline"], cwd=HERE, check=True)
subprocess.run(["cargo", "run", "--locked", "--offline"], cwd=HERE, check=True)
print("PASS: independent theta_join; original downstream crates only; complete IX program")
