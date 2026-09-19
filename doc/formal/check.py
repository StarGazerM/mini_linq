#!/usr/bin/env python3
"""Check specification inventory and source identity, never mathematical proofs."""
from pathlib import Path
import csv
import hashlib
import re
import argparse
import subprocess
import json
import os

BASE = Path(__file__).resolve().parent
ROOT = BASE.parent.parent


def tex_sources():
    """Follow the literal input/include closure used by both published roots."""
    visited = set()
    active = set()

    def visit(path):
        path = path.resolve()
        assert path not in active, f"recursive TeX inclusion: {path}"
        if path in visited:
            return
        assert path.is_file(), f"missing included TeX source: {path}"
        active.add(path)
        # The documents compile with doc/formal as their working directory.
        # LaTeX resolves input names from that directory, including nested inputs.
        text = re.sub(r"(?m)(?<!\\)%.*$", "", path.read_text())
        for name in re.findall(r"\\(?:input|include)\s*\{([^}]+)\}", text):
            child = BASE / name
            if not child.suffix:
                child = child.with_suffix(".tex")
            visit(child)
        active.remove(path)
        visited.add(path)

    for root in ("compiler.tex", "physical_design.tex"):
        visit(BASE / root)
    return sorted(visited)


def check_rule_inventory():
    occurrences = [
        (rule, str(path.relative_to(BASE)))
        for path in tex_sources()
        for rule in re.findall(r"\\Rule\{([^}]+)\}", path.read_text())
    ]
    labels = [rule for rule, _ in occurrences]
    assert len(labels) == len(set(labels)), "duplicate inference rule names"
    with (BASE / "rules.tsv").open() as manifest:
        entries = list(csv.DictReader(manifest, delimiter="\t"))
    mapped = [entry["rule"] for entry in entries]
    assert len(mapped) == len(set(mapped)), "duplicate manifest entries"
    assert set(labels) == set(mapped), "rule/manifest mismatch: " + str(set(labels) ^ set(mapped))
    for entry in entries:
        assert entry["status"], entry
        if entry["kind"] == "abstract":
            assert entry["path"] == entry["marker"] == "-", entry
            continue
        assert entry["kind"] == "implementation", entry
        source = ROOT / entry["path"]
        assert source.is_file(), source
        assert source.read_text().count(entry["marker"]) == 1, (
            "source marker must identify exactly one location", entry
        )
    # Inventory of executable rule variants, independent of LaTeX rendering.
    rule_source = (ROOT / "crates/cq-query-lowering/src/model.rs").read_text()
    enum = re.search(r"pub enum Rule \{(.*?)^\}", rule_source, re.S | re.M).group(1)
    variants = set(re.findall(r"^    (\w+)", enum, re.M))
    paired = {
        "Introduce": "P-Start", "Select": "P-Select", "Begin": "P-Begin",
        "Up": "Y-Up", "Down": "Y-Down", "Freeze": "Y-Freeze",
        "Combine": "P-Combine", "Trim": "P-Trim", "Output": "P-Output",
        "Match": "C-Match", "Guard": "C-Guard", "Bind": "C-Bind",
        "Constant": "C-Constant", "Finish": "C-Finish",
    }
    assert variants == set(paired), "administrative enum inventory changed"
    assert set(paired.values()) <= set(labels)
    proposed = {entry["rule"] for entry in entries if entry["status"].startswith("PROPOSED:")}
    with (BASE / "physical_rules.tsv").open() as manifest:
        physical = list(csv.DictReader(manifest, delimiter="\t"))
    assert len(physical) == len({entry["rule"] for entry in physical})
    assert {entry["rule"] for entry in physical} == proposed
    locations = dict(occurrences)
    for entry in physical:
        assert entry["specification"] == locations[entry["rule"]], entry
        assert entry["status"].startswith("PROPOSED:"), entry
    return len(labels), len(proposed)


def check_implementation(trace, target):
    """Verify current sources and actual boundaries of the documented fixture."""
    trace = trace.resolve()
    assert (trace / "verification.txt").is_file(), "trace did not complete its source-identity check"
    hashes = json.loads((trace / "source-hashes.json").read_text())
    for name, expected in hashes.items():
        source = ROOT / name
        assert source.is_file(), f"captured source was removed: {name}"
        assert hashlib.sha256(source.read_bytes()).hexdigest() == expected, f"stale capture: {name}; capture current sources again"
    env = os.environ.copy()
    env["CQ_REALIZATION_TRACE"] = str(trace)
    features = "written-order,index-demands,physical-selection,physical-realization,access-to-set-loops"
    test = "captured_boundaries_equal_authored_ix_physical_and_access"
    if target == "physical_pull":
        features = "written-order,index-demands,physical-selection,physical-realization,access-to-pipeline,pipeline-distinct-iter"
        test = "captured_boundaries_equal_authored_ix_physical_access_and_pipeline"
    result = subprocess.run([
        "cargo", "test", "-p", "mini-linq", "--offline", "--locked", "--no-default-features",
        "--features", features,
        "--test", target, test, "--", "--ignored", "--exact",
    ], cwd=ROOT, env=env, check=True, text=True, stdout=subprocess.PIPE)
    print(result.stdout, end="")
    assert "test result: ok. 1 passed;" in result.stdout, f"boundary test was not executed: {target}::{test}"
    print(f"PASS: current-source boundary agreement for {target}.")
    print("This does not admit stored intermediates, block/YA, or arbitrary external providers.")


def check_stage_fixtures():
    """Check authored document assembly, not parsing or compiler conformance."""
    for directory in ["", "auxiliary"]:
        fixtures = BASE.parent / "ix-physical" / directory
        names = [
            "input.ra", "environment.txt", "expected.indexes", "expected.ix",
            "expected.selection", "expected.physical", "expected.choose.rs", "expected.rs",
        ]
        for name in names:
            assert (fixtures / name).is_file(), fixtures / name
        read = lambda name: (fixtures / name).read_text()
        ix = read("input.ra").rstrip() + "\n" + read("expected.indexes")
        assert read("expected.ix") == ix, f"IX must preserve RA: {fixtures}"
        physical = ix + read("expected.selection")
        assert read("expected.physical") == physical, f"Physical must preserve IX: {fixtures}"
        call = "realize_physical! {\n" + "".join(
            "    " + line + "\n" for line in physical.splitlines()
        ) + "}\n"
        assert read("expected.choose.rs") == call, f"selection must deliver Physical: {fixtures}"
    contract = (BASE / "ix_physical_contract.tex").read_text()
    for fixture in ["input.ra", "expected.ix", "expected.choose.rs", "expected.rs"]:
        assert "\\lstinputlisting{../ix-physical/" + fixture + "}" in contract, fixture


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--implementation", action="store_true", help="check actual boundaries for the documented implemented domain")
    parser.add_argument("--trace", type=Path, default=BASE.parent / "ix-physical/realization-trace", help="current real proc-macro capture used by --implementation")
    parser.add_argument("--query-trace", type=Path, default=BASE.parent / "ix-physical/query-trace", help="separate full-CQ capture used by --implementation")
    parser.add_argument("--pull-trace", type=Path, default=BASE.parent / "ix-physical/pull-trace", help="full-CQ pull capture including named Pipeline, used by --implementation")
    args = parser.parse_args()
    rules, proposed = check_rule_inventory()
    check_stage_fixtures()
    print(f"Documentation inventory: {rules} rules, including {proposed} proposed/unimplemented rules.")
    print("The papers share authored fixture files. This does not establish compiler conformance.")
    print("Both local stage sequences preserve their inputs and deliver the exact Physical payload.")
    if args.implementation:
        check_implementation(args.trace, "physical_realization")
        check_implementation(args.query_trace, "physical_query")
        check_implementation(args.pull_trace, "physical_pull")


if __name__ == "__main__":
    main()
