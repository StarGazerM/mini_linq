#!/usr/bin/env python3
"""Check specification inventory and source identity, never mathematical proofs."""
from pathlib import Path
import csv
import hashlib
import re
import ast

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
    # Every implemented Access refinement rule has a source and algorithm link.
    physical = (BASE / "access_refinement.tex").read_text()
    physical_labels = set(re.findall(r"\\Rule\{([^}]+)\}", physical))
    with (BASE / "physical_rules.tsv").open() as manifest:
        physical_entries = list(csv.DictReader(manifest, delimiter="\t"))
    physical_names = [entry["rule"] for entry in physical_entries]
    assert len(physical_names) == len(set(physical_names))
    assert set(physical_names) == physical_labels, "physical rule mapping mismatch"
    algorithms = dict(re.findall(
        r"\\begin\{algorithm\}.*?\\label\{([^}]+)\}(.*?)\\end\{algorithm\}",
        physical, re.S))
    model_ast = ast.parse((BASE / "physical_model.py").read_text())
    functions = {node.name for node in model_ast.body
        if isinstance(node, ast.FunctionDef)}
    declared_model_links = next(ast.literal_eval(node.value)
        for node in model_ast.body if isinstance(node, ast.Assign)
        and any(isinstance(target, ast.Name) and target.id == "PHYSICAL_MODEL_RULES"
                for target in node.targets))
    assert declared_model_links == {
        entry["rule"]: entry["model_marker"] for entry in physical_entries
        if entry["model_marker"] != "-"}, "model rule/function mapping drift"
    general_entries = {entry["rule"]: entry for entry in entries}
    for entry in physical_entries:
        assert entry["scope"], entry
        assert entry["algorithm"] in algorithms, entry
        assert entry["rule"] in algorithms[entry["algorithm"]], entry
        assert entry["model_marker"] == "-" or entry["model_marker"] in functions, entry
        general = general_entries[entry["rule"]]
        assert general["kind"] == "implementation", entry
        assert (general["path"], general["marker"]) == (entry["path"], entry["marker"]), entry
    return len(labels), len(physical_labels)


def check_source_identity():
    fingerprints = (BASE / "sources.sha256").read_text().splitlines()
    for line in fingerprints:
        expected, path = line.split("  ", 1)
        actual = hashlib.sha256((ROOT / path).read_bytes()).hexdigest()
        assert actual == expected, f"source drift: {path}; re-audit before refreshing snapshot"
    return len(fingerprints)


def main():
    rules, physical_rules = check_rule_inventory()
    fingerprints = check_source_identity()
    print(f"Checked {rules} named rule links and {fingerprints} source fingerprints.")
    print(f"Includes {physical_rules} Access refinement rules with source/algorithm/model links.")
    print("This checks coverage and source identity; it does not check proofs or semantics.")


if __name__ == "__main__":
    main()
