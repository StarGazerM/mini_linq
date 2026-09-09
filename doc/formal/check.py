#!/usr/bin/env python3
"""Check specification inventory and source identity, never mathematical proofs."""
from pathlib import Path
import csv
import hashlib
import re

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
    fingerprints = (BASE / "sources.sha256").read_text().splitlines()
    for line in fingerprints:
        expected, path = line.split("  ", 1)
        actual = hashlib.sha256((ROOT / path).read_bytes()).hexdigest()
        assert actual == expected, f"source drift: {path}; re-audit before refreshing snapshot"
    print(f"Checked {len(labels)} named rule links and {len(fingerprints)} source fingerprints.")
    print("This checks coverage and source identity; it does not check proofs or semantics.")


if __name__ == "__main__":
    main()
