#!/usr/bin/env python3
"""Check family dependencies, actual source removal and isolated macro expansion.

This inventory describes build ownership only. It is not compiler dispatch.
Production isolation, module removal, macro resolution and semantic consumers
are separate gates. Package counts and test-only dependencies are not proofs
of compiler architecture.
"""
from __future__ import annotations

import argparse
import copy
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib

ROOT = Path(__file__).resolve().parents[1]
FOUNDATIONS = {"cq-ir": set(), "cq-storage": set()}
FAMILIES = {name: {"cq-ir"} for name in (
    "cq-plan", "cq-index", "cq-realize", "cq-native", "cq-execution", "cq-pipeline")}
ALLOWED = FOUNDATIONS | FAMILIES | {
    "cq-compose": {"cq-ir", *FAMILIES},
    "mini-linq": {"cq-ir", "cq-storage", "cq-compose", *FAMILIES},
}
FRONTEND = {
    "canonical": ("cq-plan", "canonical"), "prefix": ("cq-plan", "prefix"), "graph": ("cq-plan", "graph"),
    "input-indexes": ("cq-index", "input"), "result-indexes": ("cq-index", "result"),
    "hash-inputs": ("cq-realize", "hash-inputs"), "hash-prefix": ("cq-realize", "hash-prefix"),
    "hash-materialized": ("cq-realize", "hash-materialized"), "relations": ("cq-realize", "relations"),
    "scan-rows": ("cq-native", "candidate-scan"), "prune-rows": ("cq-native", "candidate-prune"),
    "exact-keys": ("cq-native", "keys-exact"), "zone-keys": ("cq-native", "keys-zone"),
    "hash-witnesses": ("cq-native", "witness-hash"), "nested-witnesses": ("cq-native", "witness-nested"),
    "hash-pairs": ("cq-native", "pairs-hash"), "nested-pairs": ("cq-native", "pairs-nested"),
    "sort-merge-pairs": ("cq-native", "pairs-merge"),
    "hash-rows": ("cq-native", "rows-hash"), "sorted-rows": ("cq-native", "rows-sorted"),
    "block-rows": ("cq-native", "rows-block"),
    "tuple-hash-rows": ("cq-native", "rows-tuple-hash"),
    "tuple-sorted-rows": ("cq-native", "rows-tuple-sorted"),
    "pull": ("cq-execution", "pull"), "push-set": ("cq-execution", "push"),
    "iterator-pipeline": ("cq-pipeline", "iterator"), "collect-pipeline": ("cq-pipeline", "collect"),
    "stream-pipeline": ("cq-pipeline", "stream"),
}
# Prefixes name complete implementation directories, or individual source files.
# An implementation can access its own descendants and these declared helpers.
IMPLEMENTATIONS = {
    "cq-plan": {
        "canonical": ["src/canonical", "tests/canonical.rs"],
        "prefix": ["src/prefix", "src/dependencies.rs", "src/model.rs", "src/proof.rs", "tests/prefix.rs"],
        "graph": ["src/graph", "src/dependencies.rs", "src/model.rs", "src/proof.rs", "tests/graph.rs"],
    },
    "cq-index": {name: [f"src/{name}", f"tests/{name}.rs"] for name in ("input", "result")},
    "cq-realize": {
        "hash-inputs": ["src/hash_inputs", "src/check_support"],
        "hash-prefix": ["src/hash_prefix", "src/check_support", "src/rust_bindings.rs"],
        "hash-materialized": ["src/hash_materialized", "src/check_support", "src/rust_bindings.rs"],
        "relations": ["src/relations", "src/rust_bindings.rs"],
    },
    "cq-native": {
        "candidate-scan": ["src/candidates/scan.rs"], "candidate-prune": ["src/candidates/prune.rs"],
        "keys-exact": ["src/keys/exact.rs"], "keys-zone": ["src/keys/zone.rs"],
        "witness-hash": ["src/witnesses/hash.rs"], "witness-nested": ["src/witnesses/nested.rs"],
        "pairs-hash": ["src/pairs/hash.rs"], "pairs-nested": ["src/pairs/nested.rs"],
        "pairs-merge": ["src/pairs/merge.rs"],
        "rows-hash": ["src/rows/hash.rs", "src/rows/native.rs", "src/rows/schema.rs", "src/rows/native_tests.rs", "tests/schemas.rs"],
        "rows-sorted": ["src/rows/sorted.rs", "src/rows/native.rs", "src/rows/schema.rs", "src/rows/native_tests.rs"],
        "rows-block": ["src/rows/block.rs", "src/rows/schema.rs", "tests/schemas.rs"],
        "rows-tuple-hash": ["src/rows/tuple_hash.rs", "src/rows/native.rs", "src/rows/schema.rs", "tests/schemas.rs"],
        "rows-tuple-sorted": ["src/rows/tuple_sorted.rs", "src/rows/native.rs", "src/rows/schema.rs"],
    },
    "cq-execution": {
        "pull": ["src/pull", "tests/boundary.rs"],
        "push": ["src/push.rs", "tests/push.rs"],
    },
    "cq-pipeline": {name: [f"src/{name}.rs", f"src/{name}", "src/operators.rs", "src/observations.rs", "tests/boundary.rs"]
                    for name in ("iterator", "collect", "stream")},
}
MODULE_WIRING = {
    "cq-native": [f"src/{group}/mod.rs" for group in ("candidates", "keys", "witnesses", "pairs", "rows")],
}


def toml(value):
    if isinstance(value, bool): return str(value).lower()
    if isinstance(value, str): return json.dumps(value)
    if isinstance(value, list): return "[" + ", ".join(map(toml, value)) + "]"
    if isinstance(value, dict): return "{ " + ", ".join(f"{k} = {toml(v)}" for k, v in value.items()) + " }"
    return str(value)


def metadata():
    return json.loads(subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--format-version=1", "--locked"], cwd=ROOT, text=True))


def check(data, announce=True):
    packages = {p["name"]: p for p in data["packages"]}
    for name, allowed in ALLOWED.items():
        if name not in packages: raise ValueError(f"declared compiler component disappeared: {name}")
        for dep in packages[name]["dependencies"]:
            target = dep["name"]
            if dep["kind"] == "dev": continue
            if "path" not in dep and target not in packages: continue
            expected = Path(packages[target]["manifest_path"]).parent if target in packages else None
            if target not in allowed or "path" not in dep or Path(dep["path"]).resolve() != expected:
                raise ValueError(f"forbidden {dep['kind'] or 'normal'} dependency: {name} -> {target}")
        if name in FOUNDATIONS | FAMILIES:
            for target in packages[name]["targets"]:
                if set(target["kind"]) & {"test", "example", "bench"}: continue
                if not Path(target["src_path"]).resolve().is_relative_to(ROOT / "crates" / name):
                    raise ValueError(f"target source escapes its family: {name}: {target['src_path']}")
    for name, implementations in IMPLEMENTATIONS.items():
        features = packages[name]["features"]
        if features.get("default") != []: raise ValueError(f"family has implicit implementations: {name}")
        for feature in implementations:
            if feature not in features: raise ValueError(f"missing implementation feature: {name}/{feature}")
            if local_features(features, [feature]) & set(implementations) != {feature}:
                raise ValueError(f"implementation feature couples a sibling: {name}/{feature}")
    frontend = packages["mini-linq"]
    for dep in frontend["dependencies"]:
        if dep["kind"] is None and dep["name"] in set(FAMILIES) | {"cq-compose"}:
            if not dep["optional"] or (dep["name"] in IMPLEMENTATIONS and dep["uses_default_features"]):
                raise ValueError(f"frontend unconditionally includes implementations: {dep['name']}")
    for feature, (name, implementation) in FRONTEND.items():
        expected = [f"dep:{name}"] + ([f"{name}/{implementation}"] if implementation else [])
        if frontend["features"].get(feature) != expected:
            raise ValueError(f"frontend feature acquired hidden dependencies: {feature}")
    if announce: print(f"PASS: production dependency boundaries and {len(FRONTEND)} scoped frontend feature contracts; test dependencies may compose implementations", flush=True)
    return packages


def negative_dependency_checks(data):
    # Metadata normalizes package aliases, optional and target-specific deps.
    for kind, remote in [(None, False), ("build", False), (None, True), ("build", True)]:
        altered = copy.deepcopy(data)
        package = next(p for p in altered["packages"] if p["name"] == "cq-index")
        dep = {"name": "cq-realize", "kind": kind, "optional": True, "rename": "innocent_alias",
               "target": "cfg(unix)", "uses_default_features": False}
        if not remote: dep["path"] = str(ROOT / "crates/cq-realize")
        package["dependencies"].append(dep)
        try: check(altered, announce=False)
        except ValueError: pass
        else: raise ValueError("dependency guard accepted a forbidden renamed/optional/target dependency")
    altered = copy.deepcopy(data)
    package = next(p for p in altered["packages"] if p["name"] == "cq-index")
    package["dependencies"].append({"name": "cq-realize", "kind": "dev", "path": str(ROOT / "crates/cq-realize")})
    check(altered, announce=False)
    print("PASS: four production dependency mutations rejected; cross-implementation test dependency accepted", flush=True)


def local_features(features, selected):
    active = set()
    pending = list(selected)
    while pending:
        feature = pending.pop()
        if feature in active: continue
        if feature not in features: raise ValueError(f"unknown feature: {feature}")
        active.add(feature)
        pending.extend(entry for entry in features[feature] if not entry.startswith("dep:") and "/" not in entry)
    return active


def audit_implementation_text(name, feature, path, text):
    alternatives = set(IMPLEMENTATIONS[name]) - {feature}
    mentioned_features = set(re.findall(r'feature\s*=\s*"([^"]+)"', text))
    if mentioned_features & alternatives:
        raise ValueError(f"implementation behavior depends on a sibling feature: {name}/{path}")
    for alternative in alternatives:
        primary = Path(IMPLEMENTATIONS[name][alternative][0]).with_suffix("")
        modules = primary.parts[1:]
        prefix = "::".join(modules)
        if prefix and re.search(r"(?:crate|" + re.escape(name.replace("-", "_")) + r")::" + re.escape(prefix) + r"\b", text):
                raise ValueError(f"implementation imports sibling: {name}/{path} -> {alternative}")


def audit_imports():
    checked = 0
    for name, variants in IMPLEMENTATIONS.items():
        for feature, paths in variants.items():
            base = ROOT / "crates" / name
            primary = base / paths[0]
            owned = list(primary.rglob("*.rs")) if primary.is_dir() else [primary]
            if name == "cq-pipeline": owned += list((base / "src" / feature).rglob("*.rs"))
            for path in owned:
                audit_implementation_text(name, feature, path.relative_to(base), path.read_text())
                checked += 1
    for text in ('use crate::result;', '#[cfg(feature = "result")] fn route() {}'):
        try: audit_implementation_text("cq-index", "input", "mutation.rs", text)
        except ValueError: pass
        else: raise ValueError("ownership guard accepted sibling import/feature mutation")
    print(f"PASS: {checked} implementation files audited; sibling import and feature mutations rejected", flush=True)


def source_files(name, features=(), tests=True):
    base = ROOT / "crates" / name
    if name not in IMPLEMENTATIONS:
        return {p.relative_to(base).as_posix() for folder in ("src", "tests") if tests or folder == "src"
                for p in (base / folder).rglob("*.rs")}
    selected = {"src/lib.rs"}
    for feature in features:
        for spelling in IMPLEMENTATIONS[name][feature]:
            path = base / spelling
            if path.is_dir(): selected.update(p.relative_to(base).as_posix() for p in path.rglob("*.rs"))
            elif path.is_file(): selected.add(spelling)
            else: raise ValueError(f"declared source disappeared: {name}/{spelling}")
    # Keep module declarations as wiring; their unselected declarations are
    # removed separately. This is independent of package or feature spelling.
    selected.update(MODULE_WIRING.get(name, ()))
    for spelling in tuple(selected):
        for parent in (base / spelling).parents:
            if parent == base / "src" or not parent.is_relative_to(base): break
            module = parent / "mod.rs"
            if module.is_file(): selected.add(module.relative_to(base).as_posix())
    if not tests: selected = {path for path in selected if path.startswith("src/")}
    return selected


def fold_cfg(condition, features):
    condition = re.sub(r'feature\s*=\s*"([^"]+)"', lambda m: "all()" if m[1] in features else "any()", condition)
    tokens = re.findall(r'[A-Za-z_]+|[(),]', condition)
    cursor = 0
    def parse():
        nonlocal cursor
        name = tokens[cursor]; cursor += 1
        if name == "test": return True
        if name not in ("all", "any", "not"): raise ValueError(f"unreviewed cfg predicate: {condition}")
        assert tokens[cursor] == "("; cursor += 1
        values = []
        while tokens[cursor] != ")":
            values.append(parse())
            if tokens[cursor] == ",": cursor += 1
        cursor += 1
        return all(values) if name == "all" else any(values) if name == "any" else not values[0]
    return parse(), condition


def selected_wiring(text, features):
    """Only regenerate cfg/module wiring; never alter implementation statements."""
    features = set(features)
    declaration = re.compile(r'#\[cfg\(([^]]*)\)\]\s*((?:pub(?:\([^)]*\))?\s+)?(?:mod\s+\w+;|use\s+[^;]+;))', re.S)
    def selected(match):
        enabled, _ = fold_cfg(match[1], features)
        if not enabled: return ""
        return ('#[cfg(test)]\n' if re.search(r'\btest\b', match[1]) else '') + match[2]
    text = declaration.sub(selected, text)
    # Shared helpers may mention another feature in cfg, but their actual
    # definitions remain byte-for-byte intact. The export exposes selected features only.
    return re.sub(r'#\[cfg\(([^]]*)\)\]', lambda m: '#[cfg(' + fold_cfg(m[1], features)[1] + ')]', text, flags=re.S)


def write_workspace(destination, selected):
    original = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]
    lines = ["[workspace]", 'resolver = "3"', "members = " + toml([f"crates/{p}" for p in sorted(selected)]), "", "[workspace.package]"]
    lines += [f"{k} = {toml(v)}" for k, v in original["package"].items()]
    lines += ["", "[workspace.dependencies]"]
    lines += [f"{k} = {toml(v)}" for k, v in original["dependencies"].items()
              if not isinstance(v, dict) or "path" not in v or k in selected]
    (destination / "Cargo.toml").write_text("\n".join(lines) + "\n")
    shutil.copy2(ROOT / "rust-toolchain.toml", destination / "rust-toolchain.toml")
    shutil.copytree(ROOT / "test-support", destination / "test-support")


def copy_selected(destination, name, features=()):
    base = ROOT / "crates" / name
    output = destination / "crates" / name
    output.mkdir(parents=True)
    shutil.copy2(base / "Cargo.toml", output / "Cargo.toml")
    paths = source_files(name, features)
    for path in paths:
        target = output / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(selected_wiring((base / path).read_text(), features) if name in IMPLEMENTATIONS else (base / path).read_text())
    # Cargo discovers explicit test targets even when unrelated features are off.
    # Remove only the metadata for physically excluded test files.
    manifest = (output / "Cargo.toml").read_text()
    manifest = re.sub(r'\n\[\[test\]\]\n(.*?)(?=\n\[|\Z)', lambda m:
        m[0] if set(tomllib.loads(m[0])["test"][0].get("required-features", [])) <= set(features) else "", manifest, flags=re.S)
    (output / "Cargo.toml").write_text(manifest)
    actual = {p.relative_to(output).as_posix() for p in output.rglob("*.rs")}
    if actual != paths: raise ValueError(f"source-removal inventory mismatch for {name}")
    return paths


def isolate(packages, only=()):
    logdir = ROOT / "target/component-logs"; logdir.mkdir(parents=True, exist_ok=True)
    jobs = [(name, None) for name in FOUNDATIONS]
    jobs += [(name, feature) for name, variants in IMPLEMENTATIONS.items() for feature in variants]
    for name, feature in jobs:
        label = name + ("-" + feature if feature else "")
        if only and label not in only and name not in only: continue
        with tempfile.TemporaryDirectory(prefix=f"mini-linq-{label}-") as temporary:
            destination = Path(temporary)
            selected = {name} | ALLOWED[name]
            write_workspace(destination, selected)
            for package in selected: copy_selected(destination, package, [feature] if package == name and feature else [])
            env = os.environ.copy(); env["CARGO_TARGET_DIR"] = str(destination / "target"); env["CARGO_TERM_COLOR"] = "never"
            command = ["cargo", "test", "-p", name, "--all-targets", "--offline"] + (["--features", feature] if feature else [])
            with (logdir / f"{label}.log").open("w") as log:
                result = subprocess.run(command, cwd=destination, env=env, stdout=log, stderr=subprocess.STDOUT)
            if result.returncode: raise ValueError(f"source-removal test failed: {label}; see {logdir / (label + '.log')}")
            print(f"PASS: {label} builds/tests with alternative implementation files absent", flush=True)


CQ = 'struct Q; relation R(a:i32); answer(x) :- R(x);'
RA = CQ + ' relational { r = rename R {a -> x}; p = project r keep {x}; output p as answer(x); }'
ACCESS = 'answer(x) => ((x,)) :- R(x) => for (x,) in ([(1i32,), (2,)]);'
PIPELINE = 'iter0 = scan (x,) in ([(1i32,), (1,), (2,)]) yield (x,); iter1 = project iter0 as (x,) yield ((x,)); iter2 = distinct iter1; return iter2;'
RESOLUTION_PROBES = {
    "canonical", "prefix", "graph", "input-indexes", "result-indexes", "pull",
    "hash-inputs", "hash-prefix", "hash-materialized", "relations", "prune-rows",
}

# Concrete consumer compositions are test data, not compiler policy. Each runs
# its generated query against an independent oracle, with no fake continuation.
CONSUMERS = {
    "full-defaults": (None, "mini_linq", None),
    "default-route": (["defaults"], "mini_linq", None),
    "graph-default": (["defaults"], "graph_query", None),
    "eager-optimized": (["defaults"], "optimized_query", ("result_indexes", "hash_materialized")),
    "canonical-input": (["canonical", "input-indexes", "hash-inputs", "pull", "iterator-pipeline"],
                        "canonical_query", ("input_indexes", "hash_inputs")),
    "canonical-result-lazy": (["canonical", "result-indexes", "hash-prefix", "pull", "iterator-pipeline"],
                            "canonical_query", ("result_indexes", "hash_prefix")),
    "prefix-result-lazy": (["prefix", "result-indexes", "hash-prefix", "pull", "iterator-pipeline"],
                          "prefix_query", ("result_indexes", "hash_prefix")),
    "graph-result-materialized": (["graph", "result-indexes", "hash-materialized", "pull", "collect-pipeline"],
                                 "graph_query", ("result_indexes", "hash_materialized")),
    "materialized-callable": (["graph", "result-indexes", "hash-materialized", "pull", "collect-pipeline"],
                              "graph_query", ("result_indexes", "hash_materialized_query")),
}
NATIVE_HOSTS = {
    "relation_program": (False, False), "relation_query": (True, False),
    "observed_relation_program": (False, True), "observed_relation_query": (True, True),
}
for host in NATIVE_HOSTS:
    CONSUMERS[host.replace("_", "-")] = (
        ["relations", "hash-rows", "hash-pairs", "scan-rows", "pull", "stream-pipeline"], host, None)


def consumer_source(label, entry, route):
    """Real planner/index/realization/execution composition over 256 databases."""
    query = 'struct Q; relation R(a:i32,b:i32); relation S(a:i32,b:i32); answer(x,z) :- R(x,y), S(y,z);'
    native = entry in NATIVE_HOSTS
    callable_, observed = NATIVE_HOSTS.get(entry, (bool(route and route[1] == "hash_materialized_query"), False))
    if native:
        query = query.replace("i32", "i64") + '''
relational { r = rename R {a -> x,b -> y}; s = rename S {a -> y,b -> z};
j = natural_join r with s; p = project j keep {x,z}; output p as answer(x,z); }'''
        observer = "storage cq_storage;" if observed else ""
        query = f'''{observer} access raw::pull_stream; materialize raw::hash_rows;
inputs {{ R => raw::hash_rows; S => raw::hash_rows; }}
scan raw::scan_rows; keys unused_keys; join raw::hash_pairs;
{query}'''
        wiring = 'use raw as mini_linq;'
        prepare = '''let left: HashSet<_> = left.iter().map(|&(a,b)| [i64::from(a),i64::from(b)]).collect();
let right: HashSet<_> = right.iter().map(|&(a,b)| [i64::from(a),i64::from(b)]).collect();'''
        arguments = '&left, &right'
        normalize = ('.0' if observed else '') + '.into_iter().map(|row| (row.0 as i32,row.1 as i32)).collect::<HashSet<_>>()'
    else:
        if route:
            index, backend = route
            endpoints = 'pull, iterator_pipeline' if backend in ('hash_inputs', 'hash_prefix') else 'pull_set, collect_pipeline'
            planners = 'graph_query, prefix_query, ' if entry == 'optimized_query' else ''
            wiring = f'mod wiring {{ pub use raw::{{{planners}{index} as choose_indexes, {backend} as choose_realization, {endpoints}}}; }} use wiring as mini_linq;'
        else:
            wiring = 'use raw as mini_linq;'
        prepare = ''
        arguments = 'left.iter().copied(), right.iter().copied()'
        normalize = '.into_iter().collect::<HashSet<_>>()'
    invocation = f'raw::{entry}! {{ {query} }}'
    declaration = '' if callable_ else invocation
    execute = f'let query = {invocation}; query({arguments})' if callable_ else f'Q::run({arguments})'
    return f'''use std::collections::HashSet;
{wiring}
{declaration}
fn execute(left: &[(i32,i32)], right: &[(i32,i32)]) -> HashSet<(i32,i32)> {{
{prepare}
let result = {{ {execute} }};
result{normalize}
}}
fn main() {{
let universe = [(0,0),(0,1),(1,0),(1,1)];
for lm in 0..16 {{ for rm in 0..16 {{
let rows = |mask: usize| universe.iter().enumerate().filter_map(|(i,row)| (mask & (1 << i) != 0).then_some(*row)).collect::<Vec<_>>();
let left = rows(lm); let right = rows(rm);
let expected: HashSet<_> = left.iter().flat_map(|&(x,y)| right.iter().filter_map(move |&(key,z)| (y == key).then_some((x,z)))).collect();
assert_eq!(execute(&left,&right),expected,"{{lm}} join {{rm}}");
let duplicated: Vec<_> = left.iter().chain(&left).copied().collect();
assert_eq!(execute(&duplicated,&right),expected);
}} }}
}}'''


def check_consumers(only=()):
    env = os.environ.copy()
    env["CARGO_TARGET_DIR"] = str(ROOT / "target/frontend-consumers")
    env["CARGO_TERM_COLOR"] = "never"
    logdir = ROOT / "target/component-logs"
    logdir.mkdir(parents=True, exist_ok=True)
    for label, (features, entry, route) in CONSUMERS.items():
        if only and label not in only and not set(features or ()) & set(only): continue
        with tempfile.TemporaryDirectory(prefix="mini-linq-consumer-") as temporary:
            destination = Path(temporary)
            (destination / "src").mkdir()
            (destination / "src/main.rs").write_text(consumer_source(label, entry, route))
            feature_text = '' if features is None else ', default-features = false, features = ' + toml(features)
            runtime = NATIVE_HOSTS.get(entry, (False, False))[1]
            (destination / "Cargo.toml").write_text(
                '[package]\nname = "semantic-consumer"\nversion = "0.0.0"\nedition = "2024"\n[workspace]\n[dependencies]\n'
                'raw = { package = "mini-linq", path = ' + toml(str(ROOT / "crates/mini-linq")) + feature_text + ' }\n'
                + ('cq-storage = { path = ' + toml(str(ROOT / "crates/cq-storage")) + ' }\n' if runtime else ''))
            logpath = logdir / f"consumer-{label}.log"
            with logpath.open("w") as log:
                result = subprocess.run(["cargo", "run", "--manifest-path", str(destination / "Cargo.toml"), "--offline"],
                                        cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT)
            if result.returncode: raise ValueError(f"semantic consumer failed: {label}; see {logpath}")
            print(f"PASS: semantic consumer {label}; 256 databases plus duplicate-input checks", flush=True)


def frontend_source(feature):
    """Atomic probes establish resolution or native behavior, not whole-query semantics."""
    if feature in ("canonical", "prefix", "graph", "input-indexes", "result-indexes"):
        macro, source, next_name, marker = {
            "canonical": ("canonical_query", CQ, "choose_indexes", "relational"),
            "prefix": ("prefix_query", CQ, "choose_indexes", "relational"),
            "graph": ("graph_query", CQ, "choose_indexes", "relational"),
            "input-indexes": ("input_indexes", RA, "choose_realization", "indexes"),
            "result-indexes": ("result_indexes", RA, "choose_realization", "indexes"),
        }[feature]
        return f'''#[macro_export] macro_rules! accept {{ ($($t:tt)*) => {{ const SEEN: &str = stringify!($($t)*); }}; }}
mod wiring {{ pub use crate::accept as {next_name}; }}
use wiring as mini_linq;
raw::{macro}! {{ {source} }}
fn main() {{ assert!(SEEN.contains("{marker}")); }}'''
    if feature == "pull":
        return f'''#[macro_export] macro_rules! endpoint {{ ($($t:tt)*) => {{ (stringify!($($t)*), 42) }}; }}
mod wiring {{ pub use crate::endpoint as iterator_pipeline; }} use wiring as mini_linq;
fn main() {{ let (text, marker) = raw::pull! {{ {ACCESS} }}; assert!(text.contains("distinct")); assert_eq!(marker, 42); }}'''
    if feature == "push-set":
        return f'''fn main() {{
let rows = raw::push_set! {{ {ACCESS} }};
assert_eq!(rows, ::std::collections::HashSet::from([(1i32,), (2,)]));
}}'''
    if feature.endswith("-pipeline"):
        macro = feature.replace('-', '_')
        result = 'result.len()' if feature == 'collect-pipeline' else 'result.count()'
        expected = 3 if feature == 'stream-pipeline' else 2
        return f'fn main() {{ let result = raw::{macro}! {{ {PIPELINE} }}; assert_eq!({result}, {expected}); }}'
    if feature in ("hash-inputs", "hash-prefix", "hash-materialized"):
        macro = feature.replace('-', '_')
        endpoint = '::std::collections::HashSet::<(i32,)>::new()' if feature == 'hash-materialized' else '::std::iter::empty::<(i32,)>()'
        continuation = 'pull_set' if feature == 'hash-materialized' else 'pull'
        return f'''#[macro_export] macro_rules! chosen_access {{ ($($t:tt)*) => {{ {endpoint} }}; }}
mod wiring {{ pub use crate::chosen_access as {continuation}; }} use wiring as mini_linq;
raw::{macro}! {{ {RA} indexes {{}} }}
fn main() {{ let _ = Q::run([(1i32,)]); }}'''
    if feature == "relations":
        return '''macro_rules! rows {
(type ($($ty:ty,)*)) => { ::std::collections::HashSet<[i64;1]> };
(input ($v:expr)) => { $v }; (row ($v:expr)) => { $v };
(field ($v:expr) ($column:literal)) => { $v[$column] };
(len ($v:expr)) => { $v.len() }; (rows ($v:expr)) => { $v.iter() }; (scan ($v:expr)) => { $v.iter() };
(materialize $owner:ident $value:ident ($v:expr)) => { let $owner = $v; let $value = &$owner; };
}
macro_rules! chosen_access { ($($t:tt)*) => { ::std::collections::HashSet::<[i64;1]>::new() }; }
raw::relation_program! { access chosen_access; materialize rows; scan unused_scan; keys unused_keys; join unused_join;
inputs { R => rows; } struct Q; relation R(a:i64); answer(x) :- R(x);
relational { r = rename R {a -> x}; p = project r keep {x}; output p as answer(x); } }
fn main() { let rows = std::collections::HashSet::from([[1i64]]); let _ = Q::run(&rows); }'''
    if feature == "scan-rows":
        return '''macro_rules! rows { (rows ($v:expr)) => { $v.into_iter() }; }
fn main() { let result = raw::scan_rows! { work (compile_error!("unused observer")); candidates rows { [1,2] } (compile_error!("unused predicate")) };
assert_eq!(result.sum::<i32>(),3); }'''
    if feature == "prune-rows":
        return '''macro_rules! chosen_access { ($($t:tt)*) => { [7i32].into_iter() }; }
fn main() { let result = raw::prune_rows! { iterator chosen_access; candidates unused_provider { () } (|_: &()| true) };
assert_eq!(result.sum::<i32>(),7); }'''
    if feature in ("exact-keys", "zone-keys"):
        macro = feature.replace('-', '_'); storage = 'storage cq_storage;' if feature == 'zone-keys' else ''
        return f'''macro_rules! chosen_access {{ ($($t:tt)*) => {{ [[1i64],[3]].into_iter() }}; }}
fn main() {{ let keys = raw::{macro}! {{ {storage} iterator chosen_access; keys ::std::collections::BTreeSet;
build {{ out(x) => ([x]) :- Rows(x) => for (x,) in (unused); }} }};
assert!(raw::{macro}! {{ storage cq_storage; contains(&keys)([1i64]) }});
assert!(!raw::{macro}! {{ storage cq_storage; contains(&keys)([2i64]) }}); }}'''
    if feature in ("hash-witnesses", "nested-witnesses"):
        macro = feature.replace('-', '_')
        return f'''macro_rules! chosen_access {{ ($($t:tt)*) => {{ [(1i32,10i32),(2,20)].into_iter() }}; }}
fn main() {{ let resource = raw::{macro}! {{ iterator chosen_access;
build {{ out(x) => (x) :- Rows(x) => for (x,) in (unused); }} }};
let found: Vec<_> = raw::{macro}! {{ probe(&resource)(1i32) }}.collect(); assert_eq!(found,vec![10]); }}'''
    if feature in ("hash-pairs", "nested-pairs", "sort-merge-pairs"):
        macro = feature.replace('-', '_')
        return f'''fn main() {{
let left = [(1i32,10i32), (1,10), (2,20)];
let right = [(1i32,30i32), (1,40), (3,50)];
let mut rows = Vec::new();
raw::{macro}! {{ pairs l in (left.iter()) by (l.0), r in (right.iter()) by (r.0) {{ rows.push((l.1,r.1)); }} }};
rows.sort(); assert_eq!(rows, vec![(10,30),(10,30),(10,40),(10,40)]);
}}'''
    if feature in ("hash-rows", "sorted-rows"):
        macro = feature.replace('-', '_')
        return f'''fn main() {{ let input = [[1i64],[1],[2]];
raw::{macro}! {{ materialize owner value (input) }}
let _: &raw::{macro}! {{ type (i64,) }} = value;
assert_eq!(raw::{macro}! {{ rows(value) }}.count(),2); }}'''
    if feature in ("tuple-hash-rows", "tuple-sorted-rows"):
        macro = feature.replace('-', '_')
        return f'''fn main() {{
raw::{macro}! {{ produce owner value via emit {{
    emit((String::from("a"), 1i32)); emit((String::from("a"), 1i32)); emit((String::from("b"), 2i32));
}} }}
let _: &raw::{macro}! {{ type (String, i32,) }} = value;
assert_eq!(raw::{macro}! {{ rows(value) }}.count(), 2);
}}'''
    if feature == "block-rows":
        return '''fn main() { let input = std::collections::HashSet::from([[1i64],[2]]);
raw::block_rows! { storage cq_storage; packing (::std::num::NonZeroUsize::new(1).unwrap()); materialize owner value (input) }
let _: &raw::block_rows! { storage cq_storage; type (i64,) } = &owner;
assert_eq!(raw::block_rows! { storage cq_storage; rows(&value) }.count(),2); }'''
    raise ValueError(f"missing real invocation fixture: {feature}")


def all_feature_checks(only=()):
    # These families contain completed implementations only. Do not enable the
    # frontend/course HW4 student extension when checking simultaneous inclusion.
    logdir = ROOT / "target/component-logs"; logdir.mkdir(parents=True, exist_ok=True)
    for name in IMPLEMENTATIONS:
        if only and name not in only and not any(label.startswith(name + "-") for label in only): continue
        with (logdir / f"all-features-{name}.log").open("w") as log:
            result = subprocess.run(["cargo", "test", "-p", name, "--all-features", "--all-targets", "--offline"],
                                    cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
        if result.returncode: raise ValueError(f"simultaneous implementation features failed: {name}")
        print(f"PASS: all completed {name} features together", flush=True)


def check_frontends(packages, only=()):
    env = os.environ.copy(); env["CARGO_TARGET_DIR"] = str(ROOT / "target/frontend-isolation"); env["CARGO_TERM_COLOR"] = "never"
    logdir = ROOT / "target/component-logs"; logdir.mkdir(parents=True, exist_ok=True)
    for feature, (name, implementation) in FRONTEND.items():
        if only and feature not in only and name not in only: continue
        with tempfile.TemporaryDirectory(prefix="mini-linq-frontend-") as temporary:
            destination = Path(temporary); (destination / "src").mkdir()
            (destination / "src/main.rs").write_text(frontend_source(feature))
            runtime = feature in ("exact-keys", "zone-keys", "block-rows")
            (destination / "Cargo.toml").write_text(
                '[package]\nname = "frontend-probe"\nversion = "0.0.0"\nedition = "2024"\n[workspace]\n[dependencies]\n'
                'raw = { package = "mini-linq", path = ' + toml(str(ROOT / "crates/mini-linq"))
                + ', default-features = false, features = [' + toml(feature) + '] }\n'
                + ('cq-storage = { path = ' + toml(str(ROOT / "crates/cq-storage")) + ' }\n' if runtime else ''))
            args = ["--manifest-path", str(destination / "Cargo.toml"), "--offline"]
            resolved = json.loads(subprocess.check_output(["cargo", "metadata", "--format-version=1", *args], cwd=ROOT, text=True, stderr=subprocess.PIPE))
            by_id = {p["id"]: p["name"] for p in resolved["packages"]}; nodes = {n["id"]: n for n in resolved["resolve"]["nodes"]}
            start = next(i for i, n in by_id.items() if n == "mini-linq"); seen = set(); pending = [start]
            while pending:
                current = pending.pop()
                if current in seen: continue
                seen.add(current)
                for dep in nodes[current]["deps"]:
                    if any(k["kind"] != "dev" for k in dep["dep_kinds"]): pending.append(dep["pkg"])
            actual = {by_id[i] for i in seen} & set(ALLOWED)
            expected = {"cq-ir", "mini-linq", name}
            if actual != expected: raise ValueError(f"frontend {feature} includes {actual}, expected {expected}")
            family_node = nodes[next(i for i, n in by_id.items() if n == name)]
            active = set(family_node["features"])
            expected_features = {implementation} if implementation else set()
            if active != expected_features: raise ValueError(f"frontend {feature} enables {name} features {active}, expected {expected_features}")
            with (logdir / f"frontend-{feature}.log").open("w") as log:
                result = subprocess.run(["cargo", "run", *args], cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT)
            if result.returncode: raise ValueError(f"actual frontend {feature} failed; see {logdir / ('frontend-' + feature + '.log')}")
            kind = "resolution probe" if feature in RESOLUTION_PROBES else "native-operation probe"
            print(f"PASS: {kind} {feature}; renamed dependency and isolated {name}/{implementation}", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--isolate", action="store_true", help="build/test copied implementation sources with sibling files absent, then invoke isolated macros")
    parser.add_argument("--frontends", action="store_true", help="invoke each isolated macro with replacement continuations")
    parser.add_argument("--consumers", action="store_true", help="execute complete public routes against independent relational oracles")
    parser.add_argument("--only", action="append", default=[], help="limit expensive probes by family, implementation label, or frontend feature")
    args = parser.parse_args(); data = metadata(); packages = check(data); negative_dependency_checks(data); audit_imports()
    if args.isolate:
        isolate(packages, args.only)
        all_feature_checks(args.only)
    if args.isolate or args.frontends: check_frontends(packages, args.only)
    if args.consumers: check_consumers(args.only)


if __name__ == "__main__":
    main()
