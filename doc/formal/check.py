#!/usr/bin/env python3
"""Check specification inventory and source identity, never mathematical proofs."""
from pathlib import Path
import csv
import re
import argparse

BASE = Path(__file__).resolve().parent
ROOT = BASE.parent.parent


def tex_sources():
    """Follow the literal input/include closure of the published root."""
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

    visit(BASE / "compiler.tex")
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
    return labels


STATUSES = {"exact", "weaker", "stronger", "unimplemented", "no-rule"}


def check_conformance(rules):
    """Conformance tables name real rules, real files and known statuses."""
    rules = set(rules)
    records = sorted((BASE / "conformance").glob("*.md")) + [BASE / "CONFORMANCE.md"]
    rows = 0
    for record in records:
        text = record.read_text()
        for line in text.splitlines():
            if not line.startswith("|") or line.startswith("|---") or line.startswith("| Rule") or line.startswith("|Rule"):
                continue
            cells = [cell.strip() for cell in line.strip("|").split("|")]
            rows += 1
            # Rule names cited as inline code or bare; '—' and 'No rule' rows are decisions.
            for name in re.findall(r"\b([A-Z][A-Za-z0-9]*(?:-[A-Z][A-Za-z0-9]*)+)\b", cells[0]):
                if name in rules:
                    continue
                if name.split("-")[0] in {"H", "C", "S", "O", "E", "B", "D", "R", "Y", "L", "A", "G", "T", "FV", "CQ", "WF", "CK", "RA", "IX", "PH", "PL", "BE", "ST", "STR", "EXT", "GPU", "DL", "P", "CTX", "IF", "SEL", "ORD", "CT", "Plan"}:
                    raise AssertionError(f"{record.name}: unknown rule {name!r} in row {cells[0][:60]!r}")
            for path in re.findall(r"`([A-Za-z0-9_./-]+\.(?:rs|tex|md|py|tsv))(?::[0-9,\-]+)?`", line):
                candidate = ROOT / path
                if not candidate.is_file() and not (BASE / path).is_file():
                    raise AssertionError(f"{record.name}: cited file missing: {path}")
            statuses = [s for s in STATUSES if re.search(rf"\*\*{re.escape(s)}\*\*|\b{re.escape(s)}\b", line)]
            if "Status" not in line and not statuses and len(cells) >= 6 and record.parent.name == "conformance":
                raise AssertionError(f"{record.name}: row without a known status: {cells[0][:60]!r}")
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.parse_args()
    rules = check_rule_inventory()
    print(f"Documentation inventory: {len(rules)} rules.")
    rows = check_conformance(rules)
    print(f"Conformance records: {rows} rows name existing rules, files and statuses (doc/formal/CONFORMANCE.md).")
    print("Rule locations identify current sources. This does not establish compiler conformance.")


if __name__ == "__main__":
    main()
