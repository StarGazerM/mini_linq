#!/usr/bin/env python3
"""Gate refactors on expanded Rust identity, not timing tolerance.

    python3 tools/expanded_gate.py --baseline HEAD --targets all
    python3 tools/expanded_gate.py --baseline 4944b917 --targets tpch library

Targets are the authoritative workloads/src/tpch.rs catalog and the 17 migrated
library kernels below. Named targets and the groups all/tpch/library are accepted.
Both trees use cargo rustc --offline -- -Zunpretty=expanded. The default
--profile audited observes release cfg for TPC-H and dev cfg for library kernels,
matching the audit; --profile dev/release overrides this explicitly. Unpretty
stops before query machine-code generation, including in release mode.
Dependencies/proc macros must still be built (optimized for fast expansion).
The temporary detached worktree is always removed.

Normalization: replace repository/harness roots by <TREE>/<HARNESS>; rustfmt
(edition 2024); remove retained macro_rules definitions, not executable items;
rename lexical generated identifiers __cq_<digits>_<digits> in first-use order;
rustfmt again. Literals/comments are never rewritten as identifiers. Library
modules are extracted from the fully expanded cq-workloads library. Nothing
normalizes expressions, providers, clones, keys, comparisons or operation order.

Optional --classes JSON has {"version":1,"rules":[{"target":"retail_h01",
"class":"a","hunk_sha256":"...","reason":"...","source":"path:line"}]}.
Targets may be shell globs. Hunk hashes cover exact removed/added lines (without
line numbers or context). Only reviewed exact hunks match; class labels are NOT
blanket permission. Reports include the hash needed for review. No rule is
created automatically. a-f are the contract-7 refactor classes; g-l identify
intended interning, closed domains, full-key membership, router annotations,
fused-provider partition exclusion, and BorrowHost keyset publication.
Additional named classes require a definitions entry with kind=refactor/intended
and a description. Built-in class kinds cannot be overridden. Every rule still
requires its exact hash, reason and source citation. Unexplained hunks always fail.

Each key hashes normalized expansion, linked runtime sources (cq-storage,
cq-load, cq-sequence-tasks, and reachable GPU runtime packages), rustc -vV, expansion
flags/environment, resolved dependency versions/features, and data/oracle
manifests. Runtime sources include all crate files except target/.git/__pycache__.
TPC-H manifests/oracles are shared execution inputs from the working tree's
benchmark cache; library inputs/goldens/SQL and host support are target-specific. Missing inputs
are recorded, never silently treated as verified. --data-manifest adds inputs.

Default execution is expansion ONLY. --verify-command is an opt-in, shell-free
command template with {target}, {tree}, {key}, {expansion}, {baseline_tree},
and {baseline_key}; it must print one JSON
object: {"exact":true,"counters":{"passed":true,"values":{...}},
"scope":"default", "evidence":["path/to/result.json", ...]}. It is run only
when no verified result for this key exists. Evidence files are hashed and copied
into the cache. --verified-results imports the same outcomes from existing runs:
{"version":1,"results":[{"target":"...","key":"...","outcome":{...}}]}.
Imports are explicit attestations, not inferred from timings or an expansion.
Exact=true, counters.passed=true, nonempty counter values/scope and real evidence
are required.

Verified results live in target/expand-gate/cache/<key>.json. Identical targets
with a valid cached result skip verification compilation/execution. Expansion
is itself content-addressed by source/build inputs; an unchanged snapshot needs
no compiler invocation. Identical code without a verified cache needs no run.
Runtime-changed or classified-differs targets require exactness/counter evidence.
JSON/table output is in target/expand-gate/report (override --output). Exit 0:
passed; 1: unexplained hunks; 2: error, failed or missing required verification.
Mid-expansion input changes and compiler errors are reported per target as
error/failed; the remaining targets still finish and appear in the table.
--classify-report PATH only reapplies classes to a captured JSON report; it does
not expand, compile, run verification, or refresh source/runtime hashes. Its
table labels each snapshot target identical/refactor-only/intended/unexplained,
separately from runtime-changed status and the execution-evidence requirement.
Runtime-only changes with no emitted hunks remain execution-required.
"""
from __future__ import annotations

import argparse
import contextlib
import difflib
import fcntl
import fnmatch
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
TOOL_DIGEST = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
LIBRARY = (
    "aggregate_stats aggregate_two_keys correlated_exists correlated_parameter "
    "correlated_scalar recursion_closure surface_boolean_guard surface_compare "
    "surface_computed_head surface_disconnected surface_let_chain surface_let_sum "
    "surface_self_join surface_wildcards union_project union_two typed_line_filter"
).split()
TPCH_DEPS = (
    "mini-linq cq-load cq-storage cq-planners cq-rewrites cq-join-filters "
    "cq-ext-aggregate cq-ext-correlated cq-sequence-tasks cq-sequence-tasks-macros"
).split()
FLAGS = ["rustc", "--offline", "--", "-Zunpretty=expanded"]
EXCLUDED = {"target", ".git", "__pycache__"}
RAW_STRING = re.compile(r'(?:br|cr|r)(#*)"')
STRING = re.compile(r'(?:b|c)?"')
CHAR = re.compile(r"b?'(?:\\(?:u\{[0-9a-fA-F_]+\}|x[0-9a-fA-F]{2}|.)|[^'\\])'")
IDENT = re.compile(r"(?:r#)?[^\W\d]\w*", re.UNICODE)
CLASS_KINDS = {**dict.fromkeys("abcdef", "refactor"), **dict.fromkeys("ghijkl", "intended")}


def sha(data):
    return hashlib.sha256(data).hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def save(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def run(command, cwd, env=None):
    result = subprocess.run(command, cwd=cwd, env=env, capture_output=True)
    if result.returncode:
        raise RuntimeError(f"{shlex.join(command)}\n{result.stderr.decode(errors='replace')}")
    return result.stdout


def tokens(text):
    """Small lossless Rust lexical scanner; strings/comments remain opaque."""
    i = 0
    while i < len(text):
        start = i
        if text[i].isspace():
            i += 1
            continue
        if text.startswith("//", i):
            end = text.find("\n", i)
            i = len(text) if end < 0 else end
            yield "opaque", start, i, text[start:i]
            continue
        if text.startswith("/*", i):
            depth = 1
            i += 2
            while depth and i < len(text):
                if text.startswith("/*", i):
                    depth += 1
                    i += 2
                elif text.startswith("*/", i):
                    depth -= 1
                    i += 2
                else:
                    i += 1
            if depth:
                raise ValueError("unterminated Rust comment")
            yield "opaque", start, i, text[start:i]
            continue
        raw = RAW_STRING.match(text, i)
        if raw:
            close = '"' + raw.group(1)
            end = text.find(close, raw.end())
            if end < 0:
                raise ValueError("unterminated Rust raw string")
            i = end + len(close)
            yield "opaque", start, i, text[start:i]
            continue
        string = STRING.match(text, i)
        char = CHAR.match(text, i)
        if string:
            i = string.end()
            while i < len(text):
                if text[i] == "\\":
                    i += 2
                elif text[i] == '"':
                    i += 1
                    break
                else:
                    i += 1
            else:
                raise ValueError("unterminated Rust string")
            yield "opaque", start, i, text[start:i]
            continue
        if char:
            i = char.end()
            yield "opaque", start, i, text[start:i]
            continue
        ident = IDENT.match(text, i)
        if ident:
            i = ident.end()
            yield "ident", start, i, text[start:i]
        else:
            i += 1
            yield "punct", start, i, text[start:i]


def group_end(items, start):
    pairs = {"{": "}", "(": ")", "[": "]"}
    stack = []
    for n in range(start, len(items)):
        kind, _, _, value = items[n]
        if kind != "punct":
            continue
        if value in pairs:
            stack.append(pairs[value])
        elif value in pairs.values():
            if not stack or stack.pop() != value:
                raise ValueError("unbalanced Rust token group")
            if not stack:
                return n
    raise ValueError("unterminated Rust token group")


def replace_ranges(text, edits):
    pieces = []
    previous = 0
    for start, end, value in edits:
        pieces.extend((text[previous:start], value))
        previous = end
    pieces.append(text[previous:])
    return "".join(pieces)


def normalize(text, tree, harness, rustfmt):
    text = text.replace(str(harness), "<HARNESS>").replace(str(tree), "<TREE>")
    def fmt(value):
        p = subprocess.run([rustfmt, "--edition", "2024", "--emit", "stdout"],
                           input=value.encode(), capture_output=True, cwd=tree)
        if p.returncode:
            raise RuntimeError(p.stderr.decode(errors="replace"))
        return p.stdout.decode()
    text = fmt(text)
    items = list(tokens(text))
    cuts = []
    n = 0
    while n < len(items):
        if (items[n][0] == "ident" and items[n][3] == "macro_rules"
                and n + 3 < len(items) and items[n + 1][3] == "!"
                and items[n + 2][0] == "ident" and items[n + 3][3] in "{(["):
            end = group_end(items, n + 3)
            if end + 1 < len(items) and items[end + 1][3] == ";":
                end += 1
            cuts.append((items[n][1], items[end][2]))
            n = end + 1
        else:
            n += 1
    text = replace_ranges(text, ((start, end, "") for start, end in cuts))
    replacements = []
    names = {}
    for kind, start, end, value in tokens(text):
        if kind == "ident" and re.fullmatch(r"__cq_\d+_\d+", value):
            names.setdefault(value, f"__cq_generated_{len(names)}")
            replacements.append((start, end, names[value]))
    text = replace_ranges(text, replacements)
    return fmt(text)


def extract_module(text, name):
    items = list(tokens(text))
    matches = []
    for n in range(len(items) - 2):
        if items[n][3] == "mod" and items[n + 1][3] == name and items[n + 2][3] == "{":
            start = n - 1 if n and items[n - 1][3] == "pub" else n
            end = group_end(items, n + 2)
            matches.append(text[items[start][1]:items[end][2]] + "\n")
    if len(matches) != 1:
        raise ValueError(f"expected one expanded module {name}, found {len(matches)}")
    return matches[0]


def catalog(tree):
    text = (tree / "features/workloads/src/tpch.rs").read_text()
    pairs = re.findall(r'name:\s*"([^"]+)"\s*,\s*rust_path:\s*"([^"]+)"', text)
    lexemes = [value for kind, _, _, value in tokens(text)
               if kind != "opaque" or not value.startswith(("//", "/*"))]
    cases = sum(value == "Case" and lexemes[i + 1] == "{"
                and (i == 0 or lexemes[i - 1] not in {"struct", "impl"})
                for i, value in enumerate(lexemes[:-1]))
    if not pairs or cases != len(pairs) or len(dict(pairs)) != len(pairs):
        raise ValueError("unrecognized or duplicate TPC-H catalog entries")
    return {name: "features/" + path for name, path in pairs}


def manifest_digest(files):
    return sha(canonical(sorted(files)))


def crate_digest(path):
    files = []
    for directory, dirs, names in os.walk(path):
        dirs[:] = sorted(d for d in dirs if d not in EXCLUDED)
        for name in sorted(names):
            file = Path(directory) / name
            files.append((file.relative_to(path).as_posix(), sha(file.read_bytes())))
    return manifest_digest(files)


def runtime_sources(metadata):
    packages = {p["id"]: p for p in metadata["packages"]}
    graph = {n["id"]: n for n in metadata["resolve"]["nodes"]}
    root = metadata["resolve"]["root"]
    if root is None:
        root = next(p["id"] for p in packages.values() if p["name"] == "cq-workloads")
    seen, pending = set(), [root]
    while pending:
        item = pending.pop()
        if item in seen or any("proc-macro" in t["kind"] for t in packages[item].get("targets", [])):
            continue
        seen.add(item)
        node = graph[item]
        if "deps" in node:
            pending.extend(dep["pkg"] for dep in node["deps"]
                           if any(kind["kind"] is None for kind in dep["dep_kinds"]))
        else:
            pending.extend(node["dependencies"])
    runtimes = {}
    dependencies = []
    for identifier in sorted(seen):
        p = packages[identifier]
        name = p["name"]
        path = Path(p["manifest_path"]).parent
        dependencies.append([name, p["version"], p["source"], sorted(graph[identifier]["features"])])
        if name in {"cq-storage", "cq-load", "cq-sequence-tasks"} or (name.startswith("cq-gpu") and "runtime" in name) or "gpu" in path.parts:
            runtimes[name] = crate_digest(path)
    return dict(sorted(runtimes.items())), sorted(dependencies, key=lambda x: canonical(x))


def data_digest(root, target, extra, tree=None):
    files = set()
    bench = root / "features/workloads/target/bench"
    for directory in (bench / "data").glob("tpch-*"):
        files.update((directory / target).glob("*.manifest"))
        if (directory / target).exists():
            files.add(directory / "dbgen.manifest")
    files.update((bench / "runs").glob(f"sf-*/{target}/*/oracle.json"))
    for group in ("data", "golden", "sql"):
        directory = root / "features/workloads" / group
        for file in directory.rglob("*"):
            if file.is_file() and (target in file.parts or file.stem == target):
                files.add(file)
    files.update(extra)
    entries = []
    if target in LIBRARY:
        source_tree = tree or root
        for file in sorted((source_tree / "features/workloads/src").glob("*.rs")):
            entries.append(("host-support/" + file.relative_to(source_tree).as_posix(),
                            sha(file.read_bytes())))
    for file in sorted(files):
        label = file.relative_to(root).as_posix() if file.is_relative_to(root) else str(file)
        entries.append((label, sha(file.read_bytes()) if file.exists() else "missing"))
    if not entries:
        entries.append((target, "no-data-oracle-manifests"))
    return manifest_digest(entries), entries


def hunks(before, after, target, rules):
    diff = list(difflib.unified_diff(before.splitlines(True), after.splitlines(True),
                                   fromfile="baseline", tofile="working"))
    blocks = []
    for line in diff[2:]:
        if line.startswith("@@"):
            blocks.append([line])
        elif blocks:
            blocks[-1].append(line)
    result = []
    for block in blocks:
        changed = "".join(line for line in block[1:] if line.startswith(("+", "-")))
        digest = sha(changed.encode())
        matched = [rule for rule in rules if fnmatch.fnmatchcase(target, rule["target"])
                   and digest == rule["hunk_sha256"]]
        result.append({"header": block[0].strip(), "sha256": digest,
                       "classes": sorted({r["class"] for r in matched}),
                       "rules": matched, "text": "".join(block),
                       "unexplained": not matched})
    return "".join(diff), result


def read_rules(path):
    if path is None:
        return []
    value = json.loads(path.read_text())
    if not isinstance(value, dict) or value.get("version") != 1 or not isinstance(value.get("rules"), list):
        raise ValueError("classes file requires version=1 and rules array")
    kinds = dict(CLASS_KINDS)
    definitions = value.get("definitions", {})
    if not isinstance(definitions, dict):
        raise ValueError("class definitions must be an object")
    for name, definition in definitions.items():
        if (not isinstance(name, str) or not isinstance(definition, dict)
                or definition.get("kind") not in {"refactor", "intended"}
                or not isinstance(definition.get("description"), str) or not definition["description"]):
            raise ValueError(f"invalid class definition: {name}")
        if name in kinds and kinds[name] != definition["kind"]:
            raise ValueError(f"cannot redefine built-in class kind: {name}")
        kinds[name] = definition["kind"]
    rules = []
    for rule in value["rules"]:
        if (not isinstance(rule, dict) or not isinstance(rule.get("class"), str)
                or rule["class"] not in kinds
                or not isinstance(rule.get("hunk_sha256"), str)
                or not re.fullmatch(r"[0-9a-f]{64}", rule["hunk_sha256"])
                or any(not isinstance(rule.get(k), str) or not rule[k]
                       for k in ("target", "reason", "source"))):
            raise ValueError(f"invalid class rule: {rule!r}")
        rules.append({**rule, "kind": kinds[rule["class"]]})
    return rules


def cache_key(inputs):
    return sha(canonical(inputs))


def verified_record(cache, key):
    path = cache / (key + ".json")
    if not path.exists():
        return None
    value = json.loads(path.read_text())
    if value.get("key") != key or value.get("version") != 1:
        return None
    outcome = value.get("outcome", {})
    if (outcome.get("exact") is not True or not isinstance(outcome.get("counters"), dict)
            or outcome["counters"].get("passed") is not True
            or not isinstance(outcome["counters"].get("values"), dict) or not outcome["counters"]["values"]):
        return None
    evidence = value.get("evidence", [])
    if not evidence or not outcome.get("scope"):
        return None
    for item in evidence:
        file = cache / item["file"]
        if not file.is_file() or sha(file.read_bytes()) != item["sha256"]:
            return None
    return value


def record_verified(cache, target, key, outcome, evidence_root):
    if (outcome.get("exact") is not True or not isinstance(outcome.get("counters"), dict)
            or outcome["counters"].get("passed") is not True
            or not isinstance(outcome["counters"].get("values"), dict) or not outcome["counters"]["values"]
            or not isinstance(outcome.get("scope"), str) or not outcome["scope"]
            or not isinstance(outcome.get("evidence"), list) or not outcome["evidence"]):
        raise ValueError("verification needs exact=true, counters.passed=true with values, scope and evidence files")
    evidence = []
    for item in outcome["evidence"]:
        source = evidence_root / item
        content = source.read_bytes()
        digest = sha(content)
        relative = "evidence/" + digest
        destination = cache / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        if not destination.exists():
            destination.write_bytes(content)
        evidence.append({"file": relative, "sha256": digest, "source": str(source)})
    value = {"version": 1, "target": target, "key": key, "outcome": outcome, "evidence": evidence}
    save(cache / (key + ".json"), value)
    return value


@contextlib.contextmanager
def baseline_tree(root, revision):
    # Outside the repository: Cargo must not inherit working-tree ancestor
    # .cargo/config files when expanding an older checkout.
    with tempfile.TemporaryDirectory(prefix="mini-linq-expand-gate-") as temporary:
        tree = Path(temporary) / "worktree"
        try:
            run(["git", "-c", "core.hooksPath=/dev/null", "worktree", "add", "--detach", str(tree), revision], root)
            yield tree
        finally:
            if (tree / ".git").exists():
                run(["git", "-c", "core.hooksPath=/dev/null", "worktree", "remove", "--force", str(tree)], root)


def metadata(manifest, env):
    return json.loads(run(["cargo", "metadata", "--offline", "--format-version=1",
                           "--manifest-path", str(manifest)], manifest.parent, env))


def harness(tree, directory, target, source, deps):
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "src").mkdir(exist_ok=True)
    manifest = '[package]\nname="expand-gate-' + target.replace("_", "-") + '"\nversion="0.0.0"\nedition="2024"\n[workspace]\n[profile.dev.build-override]\nopt-level=3\n[features]\ncounters=["cq-storage/counters"]\nprofile=["cq-storage/operator-timing"]\n[dependencies]\n'
    for name in TPCH_DEPS:
        dependency = deps[name]
        manifest += f'{name} = {{ path = {json.dumps(dependency["path"])}'
        if name == "cq-load":
            manifest += ', features = ["parquet"]'
        manifest += ' }\n'
    manifest += 'serde_json="1"\nrayon="1"\n'
    (directory / "Cargo.toml").write_text(manifest)
    lock = tree / "features/workloads/Cargo.lock"
    shutil.copyfile(lock, directory / "Cargo.lock")
    workload = tree / "features/workloads"
    (directory / "src/main.rs").write_text(
        '#![recursion_limit="512"]\n#[allow(unused_imports,unused_macros,dead_code)]\n'
        f'#[path={json.dumps(str(workload / "src/tpch_runtime.rs"))}] mod tpch_runtime;\n'
        f'#[path={json.dumps(str(tree / source))}] mod query;\n'
        f'#[path={json.dumps(str(workload / "xtask/src/tpch/comparison.rs"))}] mod comparison;\n'
        f'include!({json.dumps(str(workload / "xtask/src/tpch/worker.rs"))});\n')
    return directory / "Cargo.toml"


def source_snapshot(tree, output, target, paths):
    names = run(["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"], tree).decode().split("\0")
    # Each catalog worker includes exactly its authored source. The library
    # does not include these standalone files. Shared helpers remain hashed.
    unrelated = set(paths.values()) - {paths.get(target)}
    files = {}
    for name in sorted(set(names) - {""}):
        file = tree / name
        if name in unrelated or file.is_relative_to(output) or not file.is_file():
            continue
        files[name] = sha(file.read_bytes())
    return files


def expand_side(root, tree, state, output, label, selected, paths, extra, profile):
    env = os.environ.copy()
    for key in list(env):
        if key.startswith("CARGO_PKG_") or key in {"CARGO_MANIFEST_DIR", "CARGO_MANIFEST_PATH"}:
            env.pop(key)
    env["RUSTC_BOOTSTRAP"] = "1"
    env.setdefault("CARGO_PROFILE_DEV_BUILD_OVERRIDE_OPT_LEVEL", "3")
    env.setdefault("CARGO_PROFILE_RELEASE_BUILD_OVERRIDE_OPT_LEVEL", "3")
    env["CARGO_TARGET_DIR"] = str(state / "build" / label)
    toolchain = run([env.get("RUSTC", "rustc"), "-vV"], tree, env).decode()
    rustfmt = "rustfmt"
    workspace = tree / "features/workloads/Cargo.toml"
    meta = metadata(workspace, env)
    package = next(p for p in meta["packages"] if p["name"] == "cq-workloads")
    deps = {d["name"]: d for d in package["dependencies"]}
    config_files = {parent / ".cargo" / name for parent in (tree, *tree.parents)
                    for name in ("config", "config.toml")}
    config_files.update(Path(env.get("CARGO_HOME", str(Path.home() / ".cargo"))) / name
                        for name in ("config", "config.toml"))
    flags = {
        "cargo": FLAGS, "target_kind": None,
        "workspace_manifest": sha(workspace.read_bytes()),
        "cargo_config": {str(p).replace(str(tree), "<TREE>"): sha(p.read_bytes())
                         for p in sorted(config_files) if p.is_file()},
        "environment": {k: v for k, v in sorted(env.items()) if k != "CARGO_TARGET_DIR" and (
            k in {"RUSTC", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER"}
            or k.startswith(("CARGO_PROFILE_", "CARGO_TARGET_", "CARGO_BUILD_")))},
    }
    library_cache = {}

    def expand_target(target):
        dest = output / target
        dest.mkdir(parents=True, exist_ok=True)
        for suffix in ("rs", "raw.rs", "stderr", "key.json", "error.json"):
            (dest / f"{label}.{suffix}").unlink(missing_ok=True)
        if label == "baseline":
            (dest / "expanded.diff").unlink(missing_ok=True)
        directory = state / "harness" / label / target
        selected_profile = ("dev" if target in LIBRARY else "release") if profile == "audited" else profile
        profile_args = ["--release"] if selected_profile == "release" else []
        if target in paths:
            manifest = harness(tree, directory, target, paths[target], deps)
            graph = metadata(manifest, env)
            command = ["cargo", "rustc", *profile_args, "--offline", "--manifest-path", str(manifest), "--", "-Zunpretty=expanded"]
        elif target in LIBRARY:
            manifest = workspace
            graph = metadata(workspace, env)
            command = ["cargo", "rustc", *profile_args, "--offline", "--manifest-path", str(manifest), "-p", "cq-workloads", "--lib", "--", "-Zunpretty=expanded"]
        else:
            (dest / (label + ".rs")).write_text("")
            return {"text": "", "inputs": {"missing_target": target}, "key": cache_key({"missing_target": target})}
        source_files = source_snapshot(tree, output, target, paths)
        sources = manifest_digest(source_files.items())
        runtimes, dependencies = runtime_sources(graph)
        target_flags = dict(flags)
        target_flags["profile"] = selected_profile
        target_flags["cargo"] = FLAGS[:1] + profile_args + FLAGS[1:]
        target_flags["target_kind"] = "library" if target in LIBRARY else "worker"
        target_flags["workspace_manifest"] = sha(workspace.read_bytes())
        target_flags["manifest"] = sha(manifest.read_text().replace(str(tree), "<TREE>").encode())
        expansion_key = sha(canonical({"sources": sources, "target": target,
                                       "rustc": toolchain, "flags": target_flags,
                                       "dependencies": dependencies, "normalizer": TOOL_DIGEST}))
        expansion_cache = state / "expansions" / (expansion_key + ".json")
        cached = json.loads(expansion_cache.read_text()) if expansion_cache.exists() else None
        if cached and sha(cached["text"].encode()) == cached["sha256"]:
            text, raw = cached["text"], cached["raw"]
            (dest / (label + ".stderr")).write_text("Reused content-addressed expansion; no compiler invocation.\n")
        else:
            if target in LIBRARY and library_cache.get("sources") == sources:
                raw = library_cache["raw"]
            else:
                process = subprocess.run(command, cwd=tree, env=env, capture_output=True)
                (dest / (label + ".stderr")).write_bytes(process.stderr)
                if process.returncode:
                    raise RuntimeError(f"expansion failed for {label}/{target}: see {dest / (label + '.stderr')}")
                raw = process.stdout.decode()
                if target in LIBRARY:
                    library_cache.update(sources=sources, raw=raw)
            kernel = extract_module(raw, target) if target in LIBRARY else raw
            text = normalize(kernel, tree, directory, rustfmt)
            raw = raw.replace(str(directory), "<HARNESS>").replace(str(tree), "<TREE>")
        (dest / (label + ".raw.rs")).write_text(raw)
        (dest / (label + ".rs")).write_text(text)
        after_files = source_snapshot(tree, output, target, paths)
        changed = sorted(name for name in source_files.keys() | after_files.keys()
                         if source_files.get(name) != after_files.get(name))
        after_runtimes, _ = runtime_sources(graph)
        if changed or after_runtimes != runtimes:
            raise RuntimeError(f"inputs changed during {label}/{target} expansion: {changed or 'runtime sources'}")
        if not cached:
            save(expansion_cache, {"text": text, "raw": raw, "sha256": sha(text.encode())})
        data_hash, data_files = data_digest(root, target, extra, tree)
        inputs = {"normalization_version": 1, "expansion": sha(text.encode()),
                  "runtimes": runtimes, "rustc": toolchain, "flags": target_flags,
                  "dependencies": dependencies, "data_oracle": data_hash}
        result = {"text": text, "inputs": inputs, "key": cache_key(inputs), "data_files": data_files}
        save(dest / (label + ".key.json"), {k: v for k, v in result.items() if k != "text"})
        print(f"expanded {label}/{target}", flush=True)
        return result

    results = {}
    for target in selected:
        try:
            results[target] = expand_target(target)
        except (OSError, ValueError, RuntimeError, KeyError, subprocess.SubprocessError) as error:
            results[target] = {"error": str(error)}
            save(output / target / (label + ".error.json"), results[target])
            print(f"error {label}/{target}: {error}", file=sys.stderr, flush=True)
    return results


def write_report(report, output):
    rows = report["targets"]
    for row in rows:
        row["classes"] = sorted({c for hunk in row["hunks"] for c in hunk["classes"]})
        if row["status"] == "error" or row["unexplained"]:
            classification = "unexplained"
        elif row["status"] == "identical" and not row.get("identity_inputs_changed"):
            classification = "identical"
        elif any(rule.get("kind", CLASS_KINDS.get(rule["class"])) == "intended"
                 for hunk in row["hunks"] for rule in hunk["rules"]):
            classification = "intended"
        else:
            classification = "refactor-only"
        row["classification"] = classification
        row["execution_evidence_pending"] = row["verification"] == "required"
    save(output / "report.json", report)
    lines = ["target | status | classification | classes | hunks | unexplained | verification",
             "--- | --- | --- | --- | --- | --- | ---"]
    lines.extend(f"{r['target']} | {r['status']} | {r['classification']} | {','.join(r['classes']) or '-'} | {len(r['hunks'])} | {r['unexplained']} | {r['verification']}" for r in rows)
    table = "\n".join(lines) + "\n"
    (output / "table.txt").write_text(table)
    print(table)
    if any(r["unexplained"] for r in rows):
        return 1
    return 2 if any(r["verification"] in {"required", "failed"} for r in rows) else 0


def reclassify_report(path, rules, output):
    report = json.loads(path.read_text())
    if report.get("version") != 1 or not isinstance(report.get("targets"), list):
        raise ValueError("unrecognized captured gate report")
    for row in report["targets"]:
        for hunk in row["hunks"]:
            lines = hunk["text"].splitlines(True)
            changed = "".join(line for line in lines[1:] if line.startswith(("+", "-")))
            if sha(changed.encode()) != hunk["sha256"]:
                raise ValueError(f"captured hunk hash mismatch: {row['target']}")
            matched = [rule for rule in rules if fnmatch.fnmatchcase(row["target"], rule["target"])
                       and rule["hunk_sha256"] == hunk["sha256"]]
            hunk.update(rules=matched, classes=sorted({r["class"] for r in matched}),
                        unexplained=not matched)
        row["unexplained"] = sum(h["unexplained"] for h in row["hunks"])
        row["passed"] = not row["unexplained"] and row["verification"] in {"cached", "not-required", "passed"}
    report["snapshot_only"] = True
    return write_report(report, output)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--baseline", default="HEAD")
    parser.add_argument("--targets", nargs="+", default=["all"])
    parser.add_argument("--classes", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--data-manifest", type=Path, action="append", default=[])
    parser.add_argument("--verified-results", type=Path)
    parser.add_argument("--verify-command")
    parser.add_argument("--profile", choices=["audited", "dev", "release"], default="audited")
    parser.add_argument("--classify-report", type=Path)
    args = parser.parse_args(argv)
    state = ROOT / "target/expand-gate"
    state.mkdir(parents=True, exist_ok=True)
    output = args.output.resolve() if args.output else ROOT / "target/expand-gate/report"
    cache = state / "cache"
    rules = read_rules(args.classes)
    if args.classify_report:
        if args.verify_command or args.verified_results or args.data_manifest:
            raise ValueError("classification-only mode cannot execute/import verification or change inputs")
        output = args.output.resolve() if args.output else args.classify_report.resolve().parent
        return reclassify_report(args.classify_report, rules, output)
    imports = {}
    if args.verified_results:
        imported = json.loads(args.verified_results.read_text())
        if imported.get("version") != 1 or not isinstance(imported.get("results"), list):
            raise ValueError("verified-results requires version=1 and results array")
        imports = {(r["target"], r["key"]): r["outcome"] for r in imported["results"]}
    revision = run(["git", "rev-parse", "--verify", args.baseline + "^{commit}"], ROOT).decode().strip()
    with (state / "gate.lock").open("w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        with baseline_tree(ROOT, revision) as baseline:
            current_paths, old_paths = catalog(ROOT), catalog(baseline)
            tpch = set(current_paths) | set(old_paths)
            selected = set()
            for item in args.targets:
                if item in {"all", "tpch"}:
                    selected.update(tpch)
                if item in {"all", "library"}:
                    selected.update(LIBRARY)
                if item not in {"all", "tpch", "library"}:
                    if item not in tpch and item not in LIBRARY:
                        raise ValueError(f"unknown target: {item}")
                    selected.add(item)
            selected = sorted(selected)
            extra = [p.resolve() for p in args.data_manifest]
            before = expand_side(ROOT, baseline, state, output, "baseline", selected, old_paths, extra, args.profile)
            after = expand_side(ROOT, ROOT, state, output, "working", selected, current_paths, extra, args.profile)
            rows = []
            for target in selected:
                a, b = before[target], after[target]
                if "error" in a or "error" in b:
                    rows.append({"target": target, "status": "error", "hunks": [],
                                 "unexplained": 0, "verification": "failed", "passed": False,
                                 "verification_compile_run_skipped": True,
                                 "errors": {side: value["error"] for side, value in
                                            (("baseline", a), ("working", b)) if "error" in value}})
                    continue
                same = a["text"] == b["text"]
                status = "identical" if same and a["inputs"].get("runtimes") == b["inputs"].get("runtimes") else "runtime-changed" if same else "differs"
                diff, changes = hunks(a["text"], b["text"], target, rules)
                (output / target / "expanded.diff").write_text(diff)
                key = b["key"]
                if (target, key) in imports:
                    record_verified(cache, target, key, imports[target, key], args.verified_results.resolve().parent)
                verified = verified_record(cache, key)
                required = status != "identical" or a["key"] != b["key"]
                verification = "cached" if verified else "required" if required else "not-required"
                unexplained = sum(h["unexplained"] for h in changes)
                if not verified and args.verify_command and not unexplained:
                    values = {"target": target, "tree": str(ROOT), "key": key,
                              "baseline_tree": str(baseline), "baseline_key": a["key"],
                              "expansion": str(output / target / "working.rs")}
                    command = [part.format_map(values) for part in shlex.split(args.verify_command)]
                    outcome = json.loads(run(command, ROOT))
                    verified = record_verified(cache, target, key, outcome, ROOT)
                    verification = "passed"
                row = {"target": target, "status": status, "baseline_key": a["key"], "key": key,
                       "hunks": changes, "unexplained": unexplained, "verification": verification,
                       "passed": not unexplained and verification in {"cached", "not-required", "passed"},
                       "runtime_hash_changes": sorted(
                           name for name in set(a["inputs"].get("runtimes", {})) | set(b["inputs"].get("runtimes", {}))
                           if a["inputs"].get("runtimes", {}).get(name) != b["inputs"].get("runtimes", {}).get(name)),
                       "verification_compile_run_skipped": verification in {"cached", "not-required"},
                       "verified_scope": verified["outcome"]["scope"] if verified else None,
                       "identity_inputs_changed": same and a["key"] != b["key"]}
                rows.append(row)
            report = {"version": 1, "baseline": revision, "targets": rows}
            return write_report(report, output)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, RuntimeError, KeyError, subprocess.SubprocessError) as error:
        print(f"expanded gate: {error}", file=sys.stderr)
        raise SystemExit(2)
