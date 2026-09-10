#!/usr/bin/env python3
"""Check specification inventory and source identity, never mathematical proofs."""
from pathlib import Path
import csv
import hashlib
import re
import ast

BASE = Path(__file__).resolve().parent
ROOT = BASE.parent.parent


def main():
    occurrences = [
        (rule, path.name)
        for path in sorted(BASE.glob("*.tex"))
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
        assert entry["marker"] in source.read_text(), entry
    # Inventory of executable rule variants, independent of LaTeX rendering.
    rule_source = (ROOT / "crates/cq-compiler/src/cq_to_relational/rules.rs").read_text()
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
    # Proposed physical rules are inventoried separately from production code.
    physical = (BASE / "ya_physical.tex").read_text()
    proposed = set(re.findall(r"\\Rule\{([^}]+)\}", physical))
    with (BASE / "physical_rules.tsv").open() as manifest:
        physical_entries = list(csv.DictReader(manifest, delimiter="\t"))
    physical_names = [entry["rule"] for entry in physical_entries]
    assert len(physical_names) == len(set(physical_names))
    assert set(physical_names) == proposed, "physical rule mapping mismatch"
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
    for entry in physical_entries:
        assert entry["planned_symbol"] and entry["scope"], entry
        assert entry["algorithm"] in algorithms, entry
        # Ensure expands explicitly into Choose/Build/Reuse; other rule calls
        # and runtime cases must occur by full name in their algorithm block.
        marker = {"PA-Choose": "PA-Choose/Build/Reuse",
                  "PA-Build": "PA-Choose/Build/Reuse",
                  "PA-Reuse": "PA-Choose/Build/Reuse"}.get(entry["rule"], entry["rule"])
        assert marker in algorithms[entry["algorithm"]], entry
        assert entry["model_marker"] == "-" or entry["model_marker"] in functions, entry
    assert all(entry["kind"] == "abstract" for entry in entries
               if entry["rule"] in proposed), "design must not claim production implementation"
    fingerprints = (BASE / "sources.sha256").read_text().splitlines()
    for line in fingerprints:
        expected, path = line.split("  ", 1)
        actual = hashlib.sha256((ROOT / path).read_bytes()).hexdigest()
        assert actual == expected, f"source drift: {path}; re-audit before refreshing snapshot"
    print(f"Checked {len(labels)} named rule links and {len(fingerprints)} source fingerprints.")
    print(f"Includes {len(proposed)} proposed physical rules with algorithm/model links.")
    print("This checks coverage and source identity; it does not check proofs or semantics.")


if __name__ == "__main__":
    main()
