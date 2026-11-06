#!/usr/bin/env python3
"""Check specification inventory and source identity, never mathematical proofs."""
from pathlib import Path
import argparse
import collections
import csv
import re
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


# ---------------------------------------------------------------------------
# Rule coverage (CONFORMANCE.md, "Rule coverage gate").

COVERAGE_SCOPES = ("passes", "languages")
# Output IR of each pass crate: the language crate whose declared types are
# its output definitions. `None` marks printers, whose output is Rust tokens.
OUTPUT_IR = {
    "cq-order": "cq-lang-vo",
    "cq-reduce": "cq-lang-vo",
    "cq-relational-lowering": "cq-lang-ra",
    "cq-ra-rewrites": "cq-lang-ra",
    "cq-index-policy": "cq-lang-ix",
    "cq-physical-lowering": "cq-lang-physical",
    "cq-structure-selection": "cq-lang-structures",
    "cq-represent": "cq-lang-structures",
    "cq-staging": "cq-lang-structures",
    "cq-sequence-lowering": "cq-lang-sequence",
    "cq-sequence-plan": "cq-lang-sequence",
    "cq-print": None,
    "cq-physical-realization": None,
    "cq-sequence-pull": None,
}
ALLOWLIST = BASE / "coverage-allowlist.tsv"
MARKER = re.compile(r"Formal rule ([A-Za-z0-9][A-Za-z0-9_-]*)")
RUST_TOKEN = re.compile(r'''
 (?P<line>//[^\n]*)
|(?P<block>/\*)
|(?P<raw>b?r(?P<hashes>\#*)")
|(?P<string>b?"(?:\\.|[^"\\])*")
|(?P<char>b?'(?:\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F]+\}|.)|[^\\'\n])')
|(?P<lifetime>'[A-Za-z_]\w*)
|(?P<ident>[A-Za-z_]\w*)
|(?P<number>\d[\w.]*)
|(?P<punct>::|->|=>|==|!=|<=|>=|&&|\|\||[{}()\[\];,<>=!?#:.&|*+\-/%^@$~])
|(?P<space>\s+)
''', re.X | re.S)
QUALIFIERS = {"pub", "async", "const", "unsafe", "extern", "default"}
PARSE_MACROS = {"parse_quote", "parse_quote_spanned"}
PRINT_MACROS = {"quote", "quote_spanned"}
OUTPUT_COLLECTIONS = {"definitions", "entries", "steps"}
CLOSERS = {"{": "}", "(": ")", "[": "]"}


def rust_tokens(text):
    """Split Rust source into code tokens (kind, text, line) and comments (index, text, line).

    A comment's index is the number of code tokens before it.
    """
    code, comments, i, line = [], [], 0, 1
    while i < len(text):
        match = RUST_TOKEN.match(text, i)
        if not match:
            i += 1
            continue
        kind = match.lastgroup
        if kind == "block":
            depth, j = 1, i + 2
            while j < len(text) and depth:
                if text.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif text.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            token, kind = text[i:j], "comment"
        elif kind == "raw":
            hashes = match.group("hashes")
            j = text.index('"' + hashes, match.end()) + 1 + len(hashes)
            token, kind = text[i:j], "string"
        else:
            j, token = match.end(), match.group(0)
            kind = {"line": "comment", "char": "string", "lifetime": "other",
                    "number": "other"}.get(kind, kind)
        if kind == "comment":
            comments.append((len(code), token, line))
        elif kind != "space":
            code.append((kind, token, line))
        line += token.count("\n")
        i = j
    return code, comments


def closing(code, i):
    opener, closer, depth = code[i][1], CLOSERS[code[i][1]], 0
    for j in range(i, len(code)):
        if code[j][0] == "punct":
            if code[j][1] == opener:
                depth += 1
            elif code[j][1] == closer:
                depth -= 1
                if depth == 0:
                    return j
    raise AssertionError("unbalanced Rust delimiters")


def item_start(code, i):
    """First token of the item whose keyword is at i, and its attribute texts."""
    attributes = []
    while i > 0:
        previous = code[i - 1][1]
        if code[i - 1][0] == "ident" and previous in QUALIFIERS:
            i -= 1
        elif code[i - 1][0] == "string" and i > 1 and code[i - 2][1] == "extern":
            i -= 1
        elif previous in (")", "]"):
            opener, depth, j = {")": "(", "]": "["}[previous], 0, i - 1
            while j >= 0:
                if code[j][1] == previous:
                    depth += 1
                elif code[j][1] == opener:
                    depth -= 1
                    if depth == 0:
                        break
                j -= 1
            if previous == ")" and j > 0 and code[j - 1][1] == "pub":
                i = j - 1
            elif previous == "]" and j > 0 and code[j - 1][1] == "#":
                attributes.append(" ".join(token for _, token, _ in code[j + 1:i - 1]))
                i = j - 1
            else:
                break
        else:
            break
    return i, attributes


def rust_functions(code):
    """Every fn item with a body, outside test items and macro invocations."""
    functions = []

    def walk(low, high, path, depth):
        i = low
        while i < high:
            kind, token, _ = code[i]
            if (kind == "ident" and i + 2 < high and code[i + 1][1] == "!"
                    and code[i + 2][1] in CLOSERS):
                i = closing(code, i + 2) + 1
                continue
            if kind == "ident" and token == "macro_rules" and code[i + 1][1] == "!":
                i = closing(code, i + 3 if code[i + 2][0] == "ident" else i + 2) + 1
                continue
            if (kind != "ident" or token not in ("mod", "trait", "impl", "fn")
                    or (i > 0 and code[i - 1][1] in (".", "::"))):
                i += 1
                continue
            if token != "impl" and not (i + 1 < high and code[i + 1][0] == "ident"):
                i += 1
                continue
            if token == "impl" and not (i == low or code[i - 1][1] in (";", "}", "{", "]")
                                        or code[i - 1][1] in QUALIFIERS):
                i += 1
                continue
            j, nesting = i + 1, 0
            while j < high:
                if code[j][0] == "punct":
                    if code[j][1] in "([":
                        nesting += 1
                    elif code[j][1] in ")]":
                        nesting -= 1
                    elif nesting == 0 and code[j][1] in "{;":
                        break
                j += 1
            if j >= high or code[j][1] == ";":
                i = j + 1
                continue
            end = closing(code, j)
            start, attributes = item_start(code, i)
            if any(a == "test" or a == "cfg ( test )" for a in attributes):
                i = end + 1
                continue
            if token == "impl":
                header, k = code[i + 1:j], 0
                if header and header[0][1] == "<":
                    angle = 0
                    for k, (_, text, _) in enumerate(header):
                        angle += (text == "<") - (text == ">")
                        if angle == 0:
                            break
                    k += 1
                header = header[k:]
                words = [text for _, text, _ in header]
                if "for" in words:
                    header = header[words.index("for") + 1:]
                name, angle = "?", 0
                for kind_, text, _ in header:
                    if text == "<":
                        angle += 1
                    elif text == ">":
                        angle -= 1
                    elif text == "where" and angle == 0:
                        break
                    elif kind_ == "ident" and angle == 0 and text not in ("dyn", "mut"):
                        name = text
                walk(j + 1, end, path + [name], depth)
            elif token == "fn":
                name = code[i + 1][1]
                functions.append({"path": path + [name], "start": start, "keyword": i,
                                  "open": j, "close": end, "depth": depth})
                walk(j + 1, end, path + [name], depth + 1)
            else:
                walk(j + 1, end, path + [code[i + 1][1]], depth)
            i = end + 1

    walk(0, len(code), [], 0)
    return functions


def own_ranges(function, functions):
    """Body token ranges of a function, excluding nested fn items."""
    nested = sorted((g["start"], g["close"]) for g in functions
                    if g["open"] > function["open"] and g["close"] < function["close"])
    ranges, cursor = [], function["open"] + 1
    for start, close in nested:
        if start > cursor:
            ranges.append((cursor, start))
        cursor = max(cursor, close + 1)
    ranges.append((cursor, function["close"]))
    return ranges


def in_pattern(code, k, high):
    """The construction ending before k (plus an optional group) is a pattern."""
    if k < high and code[k][1] in "({":
        k = closing(code, k) + 1
    return k < high and code[k][1] in ("=>", "|", "if", "=")


def crate_sources(crate):
    return sorted((ROOT / crate / "src").rglob("*.rs"))


def declared_types(parsed):
    """pub struct names, pub tuple-struct names and pub enum names."""
    structs, tuples, enums = set(), set(), set()
    for code, _, _ in parsed:
        for i in range(1, len(code) - 2):
            if code[i][1] not in ("struct", "enum") or code[i + 1][0] != "ident":
                continue
            if "pub" not in (text for _, text, _ in code[item_start(code, i)[0]:i]):
                continue
            name, k = code[i + 1][1], i + 2
            if code[i][1] == "enum":
                enums.add(name)
                continue
            if code[k][1] == "<":
                angle = 0
                while True:
                    angle += (code[k][1] == "<") - (code[k][1] == ">")
                    if angle == 0:
                        break
                    k += 1
                k += 1
            (tuples if code[k][1] == "(" else structs).add(name)
    return structs, tuples, enums


def error_helper(code, function):
    """A body that is exactly one `Err(..)` expression: no program condition of its own."""
    i = function["open"] + 1
    return (code[i][1] == "Err" and code[i + 1][1] == "("
            and closing(code, i + 1) == function["close"] - 1)


def classify(code, function, functions, output, helpers):
    """(producer, checker) per CONFORMANCE.md's syntactic definition."""
    producer = checker = False
    for low, high in own_ranges(function, functions):
        i = low
        while i < high:
            kind, token, _ = code[i]
            if kind == "ident" and i + 2 < high and code[i + 1][1] == "!" and code[i + 2][1] in CLOSERS:
                end = closing(code, i + 2)
                if token in PARSE_MACROS or (token in PRINT_MACROS and output is None):
                    producer = True
                if token in PARSE_MACROS | PRINT_MACROS | {"matches"}:
                    i = end + 1
                    continue
                checker |= token == "bail"
                i += 3
                continue
            if kind == "ident":
                previous, following = code[i - 1][1], code[i + 1][1]
                if token == "Err" and following == "(" and not in_pattern(code, i + 1, high):
                    checker = True
                elif token == "Error" and following == "::" and code[i + 2][1] in ("new", "new_spanned"):
                    checker = True
                elif token in helpers and following == "(" and previous not in ("fn", "."):
                    checker = True
                elif (token in OUTPUT_COLLECTIONS and previous == "." and following == "."
                      and code[i + 2][1] == "push" and code[i + 3][1] == "("):
                    producer = True
                elif output and previous not in ("struct", "enum", "impl", "for", "trait",
                                                 "->", "type", "==", "!=", ":"):
                    structs, tuples, enums = output
                    if token in structs and following == "{":
                        producer |= not in_pattern(code, i + 1, high)
                    elif token in tuples and following == "(":
                        producer |= not in_pattern(code, i + 1, high)
                    elif (token in enums and following == "::" and code[i + 2][0] == "ident"
                          and code[i + 2][1][:1].isupper()):
                        producer |= (not in_pattern(code, i + 3, high)
                                     and code[i + 3][1] not in ("==", "!="))
            i += 1
    return producer, checker


def function_markers(code, comments, function, functions):
    """Marker names in the function's attached comments and own body, with lines."""
    nested = [(g["start"], g["close"]) for g in functions
              if g["open"] > function["open"] and g["close"] < function["close"]]
    before = code[function["start"] - 1][2] if function["start"] else 0
    found = []
    for index, text, line in comments:
        attached = (function["start"] <= index <= function["keyword"]
                    and not (index == function["start"] and line <= before))
        own = (function["open"] < index <= function["close"]
               and not any(start <= index <= close for start, close in nested))
        if attached or own:
            found += [(name, line) for name in MARKER.findall(text)]
    return found


def coverage_sites():
    """Decision sites and every Formal-rule comment in the coverage scopes."""
    crates = sorted(path.relative_to(ROOT).as_posix()
                    for scope in COVERAGE_SCOPES for path in (ROOT / scope).iterdir()
                    if (path / "src").is_dir())
    parsed = {}
    for crate in crates:
        for path in crate_sources(crate):
            code, comments = rust_tokens(path.read_text())
            parsed[path] = (code, comments, rust_functions(code))
    declared = {crate: declared_types([parsed[p] for p in crate_sources(crate)]) for crate in crates}
    sites, markers = [], []
    for crate in crates:
        scope, name = crate.split("/")
        if scope == "languages":
            output = declared[crate]
        else:
            assert name in OUTPUT_IR, f"coverage: no OUTPUT_IR entry for {crate}"
            target = OUTPUT_IR[name]
            output = None
            if target is not None:
                local = set().union(*declared[crate])
                output = tuple(names - local for names in declared["languages/" + target])
        helpers = {
            function["path"][-1]
            for path in crate_sources(crate)
            for function in parsed[path][2]
            if error_helper(parsed[path][0], function)
        }
        for path in crate_sources(crate):
            code, comments, functions = parsed[path]
            relative = path.relative_to(ROOT).as_posix()
            markers += [(relative, name, line) for _, text, line in comments
                        for name in MARKER.findall(text)]
            seen = collections.Counter()
            for function in functions:
                key = f"{relative}::{'::'.join(function['path'])}"
                seen[key] += 1
                if seen[key] > 1:
                    key += f"#{seen[key]}"
                if error_helper(code, function):
                    continue
                producer, checker = classify(code, function, functions, output, helpers)
                if producer or checker:
                    sites.append({
                        "key": key, "crate": crate, "file": relative,
                        "kind": "a" * producer + "b" * checker,
                        "markers": function_markers(code, comments, function, functions),
                    })
    return crates, sites, markers


def tex_rules():
    """Name -> (premise, conclusion) for every \\Rule of the published closure."""
    def group(text, i):
        while text[i].isspace():
            i += 1
        assert text[i] == "{", text[i:i + 40]
        depth = 0
        for j in range(i, len(text)):
            if text[j] in "{}" and text[j - 1] != "\\":
                depth += 1 if text[j] == "{" else -1
                if depth == 0:
                    return text[i + 1:j], j + 1
        raise AssertionError("unbalanced TeX group")

    rules, displays = {}, []
    for path in tex_sources():
        text = re.sub(r"(?m)(?<!\\)%.*$", "", path.read_text())
        for match in re.finditer(r"\\Rule(?=\{)", text):
            name, i = group(text, match.end())
            premise, i = group(text, i)
            conclusion, _ = group(text, i)
            rules[name] = (premise, conclusion)
        displays += re.findall(r"\\\[(.*?)\\\]", text, re.S)
        displays += re.findall(r"\$\$(.*?)\$\$", text, re.S)
        displays += re.findall(
            r"\\begin\{(?:align\*?|equation\*?|gather\*?|displaymath)\}(.*?)\\end", text, re.S)
    return rules, displays


PREDICATE = r"\\mathsf\{([A-Za-z][A-Za-z0-9]*)\}"
DEFINITION = re.compile(PREDICATE + r"(?:_\{[^{}]*\})?\s*(?:\([^=]*?\))?\s*"
                        r"(?:\\triangleq|:=|\\coloneqq|\\iff|\\Leftrightarrow|\\equiv|\\defeq|=)", re.S)
# One argument group, balanced to three nesting levels.
ARGUMENTS = r"\((?:[^()]|\((?:[^()]|\([^()]*\))*\))*\)"
BARE_PREDICATE = re.compile(r"^" + PREDICATE + r"(?:_\{[^{}]*\})?\s*(?:" + ARGUMENTS
                            + r")?\s*(?:\\Downarrow\s*\S+)?$", re.S)


def premise_conjuncts(premise):
    parts = re.split(r"\\\\|\\qquad|\\quad|\\and\b", premise)
    return [part.strip(" \n\t,") for part in parts if part.strip(" \n\t,")]


def prose_premises(rules, displays):
    """Rule name -> reasons its premise is not mathematical (CONFORMANCE.md)."""
    defined = set()
    for _, conclusion in rules.values():
        defined.update(re.findall(PREDICATE, conclusion))
    for display in displays:
        defined.update(match.group(1) for match in DEFINITION.finditer(display))
    verdicts = {}
    for name, (premise, _) in rules.items():
        conjuncts = premise_conjuncts(premise)
        reasons = [] if conjuncts else ["empty premise"]
        for conjunct in conjuncts:
            bare = BARE_PREDICATE.match(conjunct)
            if bare and bare.group(1) not in defined:
                reasons.append(f"undefined predicate {bare.group(1)}")
        verdicts[name] = reasons
    return verdicts


def read_allowlist():
    with ALLOWLIST.open() as manifest:
        reader = csv.DictReader(manifest, delimiter="\t")
        assert reader.fieldnames == ["kind", "site"], "invalid coverage allowlist header"
        entries = [(row["kind"], row["site"]) for row in reader]
    assert all(kind in ("uncovered", "misplaced") for kind, _ in entries), "coverage allowlist kind"
    # A file may hold several misplaced copies of one marker; sites are unique.
    uncovered = [site for kind, site in entries if kind == "uncovered"]
    assert len(uncovered) == len(set(uncovered)), "duplicate uncovered coverage allowlist rows"
    assert entries == sorted(entries), "coverage allowlist must be sorted"
    return entries


def coverage_report():
    """Classify every decision site; return (sites, uncovered rows, misplaced rows)."""
    with (BASE / "rules.tsv").open() as manifest:
        entries = list(csv.DictReader(manifest, delimiter="\t"))
    registry = collections.defaultdict(list)
    for entry in entries:
        for name in MARKER.findall(entry["marker"]):
            registry[name].append(entry)
    for entry in entries:
        registry.setdefault(entry["rule"], [entry])
    rules, displays = tex_rules()
    prose = prose_premises(rules, displays)
    crates, sites, markers = coverage_sites()
    misplaced = []
    for path, name, line in markers:
        assert name in registry, f"coverage: unregistered Formal rule {name}: {path}:{line}"
        if all(entry["path"] != path for entry in registry[name]):
            misplaced.append(f"{path}#{name}")
    misplaced_lines = {(path, name) for path, name, _ in markers
                       if f"{path}#{name}" in misplaced}
    for site in sites:
        reasons = []
        if len(site["markers"]) != 1:
            reasons.append(f"{len(site['markers'])} markers")
        for name, _ in site["markers"]:
            if (site["file"], name) in misplaced_lines:
                reasons.append(f"{name} registered in another file")
            for entry in registry[name]:
                reasons += [f"{entry['rule']}: {reason}" for reason in prose[entry["rule"]]]
        site["reasons"] = reasons
    return crates, sites, sorted(misplaced), prose


def check_coverage(verbose=False):
    crates, sites, misplaced, _ = coverage_report()
    current = sorted([("misplaced", site) for site in misplaced]
                     + [("uncovered", site["key"]) for site in sites if site["reasons"]])
    if verbose:
        for kind, site in current:
            print(f"{kind}\t{site}")
    allowed = read_allowlist()
    new = collections.Counter(current) - collections.Counter(allowed)
    stale = collections.Counter(allowed) - collections.Counter(current)
    reasons = {site["key"]: site["reasons"] for site in sites}
    assert not new, "coverage: sites not covered and not allowlisted:\n" + "\n".join(
        f"  {kind}\t{site}  ({'; '.join(reasons.get(site, ['misplaced marker']))})"
        for kind, site in sorted(new))
    assert not stale, ("coverage: allowlist rows now covered or gone (delete them):\n"
                       + "\n".join(f"  {kind}\t{site}" for kind, site in sorted(stale)))
    totals = {crate: [0, 0] for crate in crates}
    for site in sites:
        totals[site["crate"]][bool(site["reasons"])] += 1
    return sites, totals, len(misplaced)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--coverage", action="store_true",
                        help="run only the rule coverage gate")
    parser.add_argument("--list", action="store_true",
                        help="with --coverage, print the current uncovered and misplaced rows")
    arguments = parser.parse_args()
    if not arguments.coverage:
        rules = check_rule_inventory()
        print(f"Documentation inventory: {len(rules)} rules.")
        rows = check_conformance(rules)
        print(f"Conformance records: {rows} rows name existing rules, files and statuses (doc/formal/CONFORMANCE.md).")
        anchors = check_design_anchors()
        print(f"Design anchors: {anchors} registered comments identify existing tracked sources.")
        decode_counts = check_decode_sites()
        print("Decode sites: " + ", ".join(f"{kind}={count}" for kind, count in decode_counts.items())
              + f" ({sum(decode_counts.values())} registered comments).")
    sites, totals, misplaced = check_coverage(arguments.list)
    covered = sum(c for c, _ in totals.values())
    print(f"Rule coverage: {covered}/{len(sites)} decision sites covered; "
          f"{len(sites) - covered} uncovered and {misplaced} misplaced markers allowlisted "
          f"({ALLOWLIST.name}).")
    print("Rule locations identify current sources. This does not establish compiler conformance.")


if __name__ == "__main__":
    main()
