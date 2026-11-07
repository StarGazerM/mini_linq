#!/usr/bin/env python3
"""Check specification inventory and source identity, never mathematical proofs."""
from pathlib import Path
import csv
import re
import argparse
import subprocess

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
    # Contract 7 has one opaque, pure-total Rust category. Removed recognizers
    # must not quietly return through an inventory-only edit.
    retired = {
        "CQ-Arith", "CQ-GuardRecognize", "CQ-GuardSplit",
        "WF-VO-Arithmetic", "BE-Partial",
    }
    required = {"CQ-Where", "CQ-Closed", "WF-VO-Rust", "BE-RustRepresentation"}
    assert not retired.intersection(labels), "retired Rust-recognition rules"
    assert required.issubset(labels), "missing contract-7 rules: " + str(required - set(labels))
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


STATUSES = {"exact", "weaker", "stronger", "unimplemented", "unverified", "no-rule"}


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


def check_design_anchors():
    """Keep design ownership comments and their registry in one-to-one correspondence."""
    with (ROOT / "doc/DESIGN-ANCHORS.tsv").open() as manifest:
        reader = csv.DictReader(manifest, delimiter="\t")
        assert reader.fieldnames == ["anchor", "contract", "path", "marker", "summary"], (
            "invalid design anchor registry header"
        )
        entries = list(reader)
    anchors = [entry["anchor"] for entry in entries]
    assert len(anchors) == len(set(anchors)), "duplicate design anchor ids"
    registered = {entry["anchor"]: entry for entry in entries}
    marker = re.compile(r"\bDESIGN ([a-z0-9][a-z0-9-]*)\b")
    comments = {
        ".rs": re.compile(r"^\s*(?://[/!]?|/\*[*!]?|\*)\s*"),
        ".py": re.compile(r"^\s*#\s*"),
        ".tex": re.compile(r"^\s*%\s*"),
    }

    def locations(path):
        prefix = comments.get(path.suffix)
        assert prefix is not None, f"unsupported design anchor source: {path}"
        for number, line in enumerate(path.read_text().splitlines(), 1):
            comment = prefix.match(line)
            if comment:
                text = line[comment.end():]
                match = marker.match(text)
                if match:
                    yield match.group(1), number, text.removesuffix("*/").rstrip()

    for entry in entries:
        assert all(entry.values()) and None not in entry, ("invalid design anchor row", entry)
        assert re.fullmatch(r"[a-z0-9][a-z0-9-]*", entry["anchor"]), entry
        assert entry["marker"] == f"DESIGN {entry['anchor']}", entry
        source = ROOT / entry["path"]
        assert source.is_file(), f"missing design anchor source: {entry['path']}"
        matches = [text for anchor, _, text in locations(source) if anchor == entry["anchor"]]
        assert matches == [entry["marker"]], (
            "design marker must occur exactly once on its own comment line", entry
        )

    tracked = subprocess.check_output(["git", "ls-files", "-z"], cwd=ROOT).decode().split("\0")
    found = {}
    for name in tracked:
        path = Path(name)
        if path.suffix not in comments or {"target", ".git"}.intersection(path.parts):
            continue
        # Tracked deletions have no working-tree comments; registered files were checked above.
        if not (ROOT / path).is_file():
            continue
        for anchor, number, _ in locations(ROOT / path):
            assert anchor in registered, f"unregistered design anchor {anchor}: {name}:{number}"
            assert registered[anchor]["path"] == name, (
                f"design anchor {anchor} in unregistered file: {name}:{number}"
            )
            assert anchor not in found, f"duplicate design anchor {anchor}: {name}:{number}"
            found[anchor] = (name, number)
    assert set(found) == set(registered), (
        "design anchors must be in tracked source files", set(registered) - set(found)
    )
    return len(entries)


def check_decode_sites():
    """Check comment/registry identity, not completeness of semantic decode discovery."""
    with (ROOT / "doc/DECODE-SITES.tsv").open() as manifest:
        reader = csv.DictReader(manifest, delimiter="\t")
        assert reader.fieldnames == ["path", "marker", "class", "reason"], (
            "invalid decode site registry header"
        )
        entries = list(reader)
    classes = ("R", "P", "V", "I", "?")
    registered = {}
    for entry in entries:
        assert all(entry.values()) and None not in entry, ("invalid decode site row", entry)
        assert entry["class"] in classes, entry
        assert entry["marker"] == f"// DECODE {entry['class']}: {entry['reason']}", entry
        if entry["class"] == "V":
            assert re.search(r"\bP[456]\b", entry["reason"]), ("violation lacks plan step", entry)
        path = Path(entry["path"])
        assert not path.is_absolute() and ".." not in path.parts, entry
        assert (ROOT / path).is_file(), ("missing decode source", entry)
        key = (entry["path"], entry["marker"])
        assert key not in registered, ("duplicate decode registry entry", entry)
        registered[key] = entry

    scopes = (
        "experiments/gpu", "runtime/cq-load", "runtime/cq-storage",
        "passes/cq-print", "passes/cq-sequence-pull", "features/datalog",
    )
    sources = {ROOT / entry["path"] for entry in entries}
    for scope in scopes:
        sources.update(path for path in (ROOT / scope).rglob("*.rs")
                       if not {"target", ".git"}.intersection(path.relative_to(ROOT).parts))
    found = {}
    for source in sorted(sources):
        name = source.relative_to(ROOT).as_posix()
        for number, line in enumerate(source.read_text().splitlines(), 1):
            marker = line.strip()
            if not marker.startswith("// DECODE "):
                continue
            assert re.fullmatch(r"// DECODE [RPVI?]: .+", marker), (
                f"malformed decode comment: {name}:{number}"
            )
            key = (name, marker)
            assert key in registered, f"unregistered decode comment: {name}:{number}: {marker}"
            assert key not in found, f"duplicate decode comment: {name}:{number}: {marker}"
            found[key] = number
    assert set(found) == set(registered), (
        "registry rows without decode comments", set(registered) - set(found)
    )
    return {kind: sum(entry["class"] == kind for entry in entries) for kind in classes}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.parse_args()
    rules = check_rule_inventory()
    print(f"Documentation inventory: {len(rules)} rules.")
    rows = check_conformance(rules)
    print(f"Conformance records: {rows} rows name existing rules, files and statuses (doc/formal/CONFORMANCE.md).")
    anchors = check_design_anchors()
    print(f"Design anchors: {anchors} registered comments identify existing tracked sources.")
    decode_counts = check_decode_sites()
    print("Decode sites: " + ", ".join(f"{kind}={count}" for kind, count in decode_counts.items())
          + f" ({sum(decode_counts.values())} registered comments).")
    print("Rule locations identify current sources. This does not establish compiler conformance.")


if __name__ == "__main__":
    main()
