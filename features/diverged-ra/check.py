#!/usr/bin/env python3
"""Execute theta_join and prove the production dependency boundary.

The public cq-testkit brings CQ/VO into tests. Only normal/build edges count
for the downstream production boundary, which still strictly excludes them.
The current core RA -> VO -> CQ dependency is a core request, not an exemption.
"""
import json
from pathlib import Path
import subprocess

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
EXPECTED = {
    "cq-lang-ra": "languages", "cq-lang-ix": "languages",
    "cq-lang-physical": "languages", "cq-lang-structures": "languages",
    "cq-syntax": "languages", "cq-stage": "facade",
    "cq-physical-lowering": "passes", "cq-structure-selection": "passes",
    "cq-physical-realization": "passes", "cq-print": "passes",
    "cq-storage": "runtime",
}
TEST_ONLY = {"cq-testkit": "testkit", "cq-lang-cq": "languages", "cq-lang-vo": "languages"}
LOCAL = {"ra-theta": HERE / "ra-theta", "theta-passes": HERE / "theta-passes", "diverged-ra": HERE}
FORBIDDEN = {
    "cq-lang-cq", "cq-lang-vo", "cq-order", "cq-reduce", "cq-elaborate",
    "cq-index-policy", "mini-linq", "mini-linq-macros", "cq-testkit",
}


def main():
    subprocess.run(["cargo", "test", "--workspace", "--locked", "--offline"], cwd=HERE, check=True)
    subprocess.run(["cargo", "run", "--locked", "--offline"], cwd=HERE, check=True)
    data = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--format-version=1", "--locked", "--offline"], cwd=HERE, text=True))
    packages = {p["id"]: p for p in data["packages"]}
    nodes = {n["id"]: n for n in data["resolve"]["nodes"]}
    production = set()
    queue = list(data["workspace_members"])
    while queue:
        package_id = queue.pop()
        if package_id in production:
            continue
        production.add(package_id)
        for dependency in nodes[package_id]["deps"]:
            if any(kind["kind"] in (None, "build") for kind in dependency["dep_kinds"]):
                queue.append(dependency["pkg"])
    for package in data["packages"]:
        if package["source"] is not None:
            continue
        name = package["name"]
        if name in LOCAL:
            expected_manifest = LOCAL[name] / "Cargo.toml"
        elif name in EXPECTED | TEST_ONLY:
            expected_manifest = ROOT / (EXPECTED | TEST_ONLY)[name] / name / "Cargo.toml"
        else:
            raise SystemExit(f"FAIL: unexpected local package {name}")
        if Path(package["manifest_path"]).resolve() != expected_manifest.resolve():
            raise SystemExit(f"FAIL: substituted local package {name}: {package['manifest_path']}")
    names = {packages[p]["name"] for p in production}
    violations = names & FORBIDDEN
    if violations:
        raise SystemExit("FAIL: forbidden production dependencies: " + ", ".join(sorted(violations)))
    core = {name for name in names if name.startswith("cq-") or name.startswith("mini-linq")}
    if core != set(EXPECTED):
        raise SystemExit(f"FAIL: core dependency set differs: {core ^ set(EXPECTED)}")
    print("PASS: theta_join; explicit downstream pass queue; no CQ/VO/planner/facade production dependencies")


if __name__ == "__main__":
    main()
