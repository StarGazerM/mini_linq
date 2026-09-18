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
    "cq-query-lowering", "cq-index-policy", "cq-relation-lowering", "cq-native-lowering", "cq-access-lowering", "cq-pipeline-lowering")}
ALLOWED = FOUNDATIONS | FAMILIES | {
    "cq-compose": {"cq-ir", *FAMILIES},
    "mini-linq": {"cq-ir", "cq-storage", "cq-compose", *FAMILIES},
}
FRONTEND = {
    "written-order": ("cq-query-lowering", "written-order"), "clause-prefix": ("cq-query-lowering", "clause-prefix"), "hypergraph": ("cq-query-lowering", "hypergraph"),
    "base-index-demands": ("cq-index-policy", "base-demands"), "result-index-demands": ("cq-index-policy", "result-demands"),
    "no-index-demands": ("cq-index-policy", "no-demands"),
    "base-hash": ("cq-relation-lowering", "base-hash"), "prefix-hash": ("cq-relation-lowering", "prefix-hash"),
    "materialized-hash": ("cq-relation-lowering", "materialized-hash"), "configured": ("cq-relation-lowering", "configured"),
    "borrowed-nested": ("cq-relation-lowering", "borrowed-nested"),
    "scan-candidates": ("cq-native-lowering", "candidate-scan"), "prune-block-candidates": ("cq-native-lowering", "candidate-prune"),
    "exact-keys": ("cq-native-lowering", "keys-exact"), "summarized-exact-keys": ("cq-native-lowering", "keys-zone"),
    "hash-witnesses": ("cq-native-lowering", "witness-hash"), "nested-witnesses": ("cq-native-lowering", "witness-nested"),
    "hash-pairs": ("cq-native-lowering", "pairs-hash"), "nested-pairs": ("cq-native-lowering", "pairs-nested"),
    "sort-merge-pairs": ("cq-native-lowering", "pairs-merge"),
    "array-hash-rows": ("cq-native-lowering", "rows-hash"), "array-sorted-rows": ("cq-native-lowering", "rows-sorted"),
    "array-block-rows": ("cq-native-lowering", "rows-block"),
    "tuple-hash-rows": ("cq-native-lowering", "rows-tuple-hash"),
    "tuple-sorted-rows": ("cq-native-lowering", "rows-tuple-sorted"),
    "replay-resource": ("cq-native-lowering", "rows-replay"),
    "access-to-pipeline": ("cq-access-lowering", "to-pipeline"), "access-to-set-loops": ("cq-access-lowering", "to-set-loops"),
    "pipeline-distinct-iter": ("cq-pipeline-lowering", "distinct-iter"), "pipeline-set": ("cq-pipeline-lowering", "collect-set"),
    "pipeline-support-iter": ("cq-pipeline-lowering", "support-iter"), "pipeline-exists": ("cq-pipeline-lowering", "exists"),
}
# Explicit invocation fixtures: feature names, implementation modules and public
# macro names are separate contracts, not spellings derived from one another.
FRONTEND_MACROS = {
    "written-order": "cq_to_ra_written_order", "clause-prefix": "cq_to_ra_clause_prefix", "hypergraph": "cq_to_ra_hypergraph",
    "base-index-demands": "base_index_demands", "result-index-demands": "result_index_demands", "no-index-demands": "no_index_demands",
    "base-hash": "lower_base_hash", "prefix-hash": "lower_prefix_hash", "materialized-hash": "materialized_hash_program",
    "configured": "configured_program", "borrowed-nested": "lower_borrowed_nested",
    "scan-candidates": "scan_candidates", "prune-block-candidates": "prune_block_candidates",
    "exact-keys": "exact_keys", "summarized-exact-keys": "summarized_exact_keys",
    "hash-witnesses": "hash_witnesses", "nested-witnesses": "nested_witnesses",
    "hash-pairs": "hash_pairs", "nested-pairs": "nested_pairs", "sort-merge-pairs": "sort_merge_pairs",
    "array-hash-rows": "array_hash_rows", "array-sorted-rows": "array_sorted_rows", "array-block-rows": "array_block_rows",
    "tuple-hash-rows": "tuple_hash_rows", "tuple-sorted-rows": "tuple_sorted_rows", "replay-resource": "replay_resource",
    "access-to-pipeline": "access_to_distinct_iter", "access-to-set-loops": "access_to_set_loops",
    "pipeline-distinct-iter": "pipeline_to_distinct_iter", "pipeline-set": "pipeline_to_set",
    "pipeline-support-iter": "pipeline_to_support_iter", "pipeline-exists": "pipeline_to_exists",
}
# Prefixes name complete implementation directories, or individual source files.
# An implementation can access its own descendants and these declared helpers.
IMPLEMENTATIONS = {
    "cq-query-lowering": {
        "written-order": ["src/written_order", "tests/written_order.rs"],
        "clause-prefix": ["src/clause_prefix", "src/dependencies.rs", "src/model.rs", "src/proof.rs", "tests/clause_prefix.rs"],
        "hypergraph": ["src/hypergraph", "src/dependencies.rs", "src/model.rs", "src/proof.rs", "tests/hypergraph.rs"],
    },
    "cq-index-policy": {
        "base-demands": ["src/base_demands", "tests/base_demands.rs"],
        "result-demands": ["src/result_demands", "tests/result_demands.rs"],
        "no-demands": ["src/no_demands.rs"],
    },
    "cq-relation-lowering": {
        "base-hash": ["src/base_hash", "src/check_support"],
        "prefix-hash": ["src/prefix_hash", "src/check_support", "src/rust_bindings.rs"],
        "materialized-hash": ["src/materialized_hash", "src/check_support", "src/rust_bindings.rs"],
        "configured": ["src/configured", "src/rust_bindings.rs"],
        "borrowed-nested": ["src/borrowed_nested", "src/rust_bindings.rs"],
    },
    "cq-native-lowering": {
        "candidate-scan": ["src/candidates/scan.rs"], "candidate-prune": ["src/candidates/prune.rs"],
        "keys-exact": ["src/keys/exact.rs"], "keys-zone": ["src/keys/zone.rs"],
        "witness-hash": ["src/witnesses/hash.rs"], "witness-nested": ["src/witnesses/nested.rs"],
        "pairs-hash": ["src/pairs/hash.rs"], "pairs-nested": ["src/pairs/nested.rs"],
        "pairs-merge": ["src/pairs/merge.rs"],
        "rows-hash": ["src/rows/hash.rs", "src/rows/set_ops.rs", "src/rows/schema.rs", "src/rows/set_ops_tests.rs", "tests/schemas.rs"],
        "rows-sorted": ["src/rows/sorted.rs", "src/rows/set_ops.rs", "src/rows/schema.rs", "src/rows/set_ops_tests.rs"],
        "rows-block": ["src/rows/block.rs", "src/rows/schema.rs", "tests/schemas.rs"],
        "rows-tuple-hash": ["src/rows/tuple_hash.rs", "src/rows/set_ops.rs", "src/rows/schema.rs", "tests/schemas.rs"],
        "rows-tuple-sorted": ["src/rows/tuple_sorted.rs", "src/rows/set_ops.rs", "src/rows/schema.rs"],
        "rows-replay": ["src/rows/replay.rs"],
    },
    "cq-access-lowering": {
        "to-pipeline": ["src/to_pipeline", "tests/boundary.rs"],
        "to-set-loops": ["src/to_set_loops.rs", "tests/to_set_loops.rs"],
    },
    "cq-pipeline-lowering": {
        **{feature: [f"src/{module}.rs", f"src/{module}", "src/operators.rs", "src/observations.rs", "tests/boundary.rs"]
           for feature, module in (("distinct-iter", "distinct_iter"), ("collect-set", "collect_set"), ("support-iter", "support_iter"))},
        "exists": ["src/exists.rs", "src/operators.rs", "tests/exists.rs"],
    },
}
MODULE_WIRING = {
    "cq-native-lowering": [f"src/{group}/mod.rs" for group in ("candidates", "keys", "witnesses", "pairs", "rows")],
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
    if set(FRONTEND_MACROS) != set(FRONTEND):
        raise ValueError("frontend invocation fixture inventory differs from feature contracts")
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
        package = next(p for p in altered["packages"] if p["name"] == "cq-index-policy")
        dep = {"name": "cq-relation-lowering", "kind": kind, "optional": True, "rename": "innocent_alias",
               "target": "cfg(unix)", "uses_default_features": False}
        if not remote: dep["path"] = str(ROOT / "crates/cq-relation-lowering")
        package["dependencies"].append(dep)
        try: check(altered, announce=False)
        except ValueError: pass
        else: raise ValueError("dependency guard accepted a forbidden renamed/optional/target dependency")
    altered = copy.deepcopy(data)
    package = next(p for p in altered["packages"] if p["name"] == "cq-index-policy")
    package["dependencies"].append({"name": "cq-relation-lowering", "kind": "dev", "path": str(ROOT / "crates/cq-relation-lowering")})
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
            if name == "cq-pipeline-lowering": owned += list(primary.with_suffix("").rglob("*.rs"))
            for path in owned:
                audit_implementation_text(name, feature, path.relative_to(base), path.read_text())
                checked += 1
    for text in ('use crate::result_demands;', '#[cfg(feature = "result-demands")] fn route() {}'):
        try: audit_implementation_text("cq-index-policy", "base-demands", "mutation.rs", text)
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
    "written-order", "clause-prefix", "hypergraph", "base-index-demands", "result-index-demands", "no-index-demands", "access-to-pipeline",
    "base-hash", "prefix-hash", "materialized-hash", "configured", "borrowed-nested", "prune-block-candidates",
}

# Concrete consumer compositions are test data, not compiler policy. Each runs
# its generated query against an independent oracle, with no fake continuation.
CONSUMERS = {
    "full-defaults": (None, "mini_linq", None),
    "default-route": (["defaults"], "mini_linq", None),
    "hypergraph-default": (["defaults"], "cq_to_ra_hypergraph", None),
    "graph-or-prefix-materialized": (["defaults"], "choose_graph_or_prefix", ("result_index_demands", "materialized_hash_program")),
    "written-order-base-demands": (["written-order", "base-index-demands", "base-hash", "access-to-pipeline", "pipeline-distinct-iter"],
                        "cq_to_ra_written_order", ("base_index_demands", "lower_base_hash")),
    "written-order-result-demands": (["written-order", "result-index-demands", "prefix-hash", "access-to-pipeline", "pipeline-distinct-iter"],
                            "cq_to_ra_written_order", ("result_index_demands", "lower_prefix_hash")),
    "clause-prefix-result-demands": (["clause-prefix", "result-index-demands", "prefix-hash", "access-to-pipeline", "pipeline-distinct-iter"],
                          "cq_to_ra_clause_prefix", ("result_index_demands", "lower_prefix_hash")),
    "hypergraph-materialized": (["hypergraph", "result-index-demands", "materialized-hash", "access-to-pipeline", "pipeline-set"],
                                 "cq_to_ra_hypergraph", ("result_index_demands", "materialized_hash_program")),
    "materialized-callable": (["hypergraph", "result-index-demands", "materialized-hash", "access-to-pipeline", "pipeline-set"],
                              "cq_to_ra_hypergraph", ("result_index_demands", "materialized_hash_callable")),
}
NATIVE_HOSTS = {
    "configured_program": (False, False), "configured_callable": (True, False),
    "observed_configured_program": (False, True), "observed_configured_callable": (True, True),
}
for host in NATIVE_HOSTS:
    CONSUMERS[host.replace("_", "-")] = (
        ["configured", "array-hash-rows", "hash-pairs", "scan-candidates", "access-to-pipeline", "pipeline-support-iter"], host, None)

BORROWED_FEATURES = ["borrowed-nested", "access-to-pipeline", "pipeline-distinct-iter", "pipeline-support-iter",
                     "pipeline-exists", "replay-resource", "tuple-hash-rows", "tuple-sorted-rows"]
CONSUMERS["borrowed-nested"] = (BORROWED_FEATURES, "lower_borrowed_nested", None)
CONSUMERS["hypergraph-borrowed-nested"] = (["hypergraph", "no-index-demands", *BORROWED_FEATURES], "lower_borrowed_nested", "cq_to_ra_hypergraph")


def borrowed_consumer_source(planner):
    """Execute the complete IX/Access/Pipeline route directly and after planning."""
    # A valid relation may share a name with a stage API; their aliases must
    # remain in separate namespaces.
    first_input = "choose_index_policy" if planner else "R"
    query = f'struct Q; relation {first_input}(a:Value,b:Value); relation S(a:Value,b:Value); answer(x,z) :- {first_input}(x,y), S(y,z);'
    provider_aliases = f"pub mod inputs {{ pub use raw::{{tuple_hash_rows as {first_input}, tuple_sorted_rows as S}}; }}"
    stages = '''pub use raw::{lower_borrowed_nested as choose_relation_lowering, access_to_distinct_iter, access_to_support_iter, access_to_exists,
pipeline_to_distinct_iter, pipeline_to_support_iter, pipeline_to_exists, replay_resource};
pub use cq_storage as storage;'''
    if planner:
        declaration = f'''mod wiring {{
    pub use raw::{{{planner} as query, no_index_demands as choose_index_policy}};
    {stages}
    {provider_aliases}
}}
use wiring as mini_linq;
mini_linq::query! {{ {query} }}'''
    else:
        declaration = f'''mod wiring {{ {stages} {provider_aliases} }}
use wiring as mini_linq;
raw::lower_borrowed_nested! {{
{query}
relational {{
    l = rename R {{a -> x,b -> y}}; r = rename S {{a -> y,b -> z}};
    live = semijoin l with r; joined = natural_join live with r;
    p = project joined keep {{x,z}}; output p as answer(x,z);
}}
indexes {{}}
}}'''
    execute = 'let rows: Vec<_> = Q::iter(&left, &right).map(|(x,z)| (x.0,z.0)).collect();'
    return f'''use std::collections::{{BTreeSet, HashSet}};
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Value(i32);
{declaration}
fn execute(left: &[(i32,i32)], right: &[(i32,i32)]) -> HashSet<(i32,i32)> {{
let left: HashSet<_> = left.iter().map(|&(a,b)| (Value(a),Value(b))).collect();
let right: BTreeSet<_> = right.iter().map(|&(a,b)| (Value(a),Value(b))).collect();
{execute}
let result: HashSet<_> = rows.iter().copied().collect();
assert_eq!(rows.len(), result.len(), "the public output must already be distinct");
result
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


def consumer_source(label, entry, route):
    """Real planner/index/realization/execution composition over 256 databases."""
    if entry == "lower_borrowed_nested":
        return borrowed_consumer_source(route)
    query = 'struct Q; relation R(a:i32,b:i32); relation S(a:i32,b:i32); answer(x,z) :- R(x,y), S(y,z);'
    native = entry in NATIVE_HOSTS
    callable_, observed = NATIVE_HOSTS.get(entry, (bool(route and route[1] == "materialized_hash_callable"), False))
    if native:
        query = query.replace("i32", "i64") + '''
relational { r = rename R {a -> x,b -> y}; s = rename S {a -> y,b -> z};
j = natural_join r with s; p = project j keep {x,z}; output p as answer(x,z); }'''
        observer = "counter_storage cq_storage;" if observed else ""
        query = f'''{observer} access raw::access_to_support_iter; result_rows raw::array_hash_rows;
inputs {{ R => raw::array_hash_rows; S => raw::array_hash_rows; }}
candidates raw::scan_candidates; keys unused_keys; pairs raw::hash_pairs;
{query}'''
        wiring = 'use raw as mini_linq;'
        prepare = '''let left: HashSet<_> = left.iter().map(|&(a,b)| [i64::from(a),i64::from(b)]).collect();
let right: HashSet<_> = right.iter().map(|&(a,b)| [i64::from(a),i64::from(b)]).collect();'''
        arguments = '&left, &right'
        normalize = ('.0' if observed else '') + '.into_iter().map(|row| (row.0 as i32,row.1 as i32)).collect::<HashSet<_>>()'
    else:
        if route:
            index, backend = route
            endpoints = 'access_to_distinct_iter, pipeline_to_distinct_iter' if backend in ('lower_base_hash', 'lower_prefix_hash') else 'access_to_set, pipeline_to_set'
            planners = 'cq_to_ra_hypergraph, cq_to_ra_clause_prefix, ' if entry == 'choose_graph_or_prefix' else ''
            wiring = f'mod wiring {{ pub use raw::{{{planners}{index} as choose_index_policy, {backend} as choose_relation_lowering, {endpoints}}}; }} use wiring as mini_linq;'
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
            runtime = NATIVE_HOSTS.get(entry, (False, False))[1] or entry == "lower_borrowed_nested"
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
            if label == "borrowed-nested":
                check_borrowed_rejections(destination, env, logdir)


def borrowed_runtime_source(scan="($v).iter()", *, bind_provider=True):
    """A real Access/Pipeline route for ownership and unresolved-provider checks."""
    provider_alias = "pub(crate) use rows as R;" if bind_provider else ""
    return f'''macro_rules! rows {{
        (type ($($ty:ty,)*)) => {{ Vec<($($ty,)*)> }};
        (scan ($v:expr)) => {{ {scan} }};
        (field ($v:expr) ($column:tt)) => {{ ($v).$column }};
        }}
        mod wiring {{
            pub use raw::{{access_to_distinct_iter, access_to_support_iter, access_to_exists, pipeline_to_distinct_iter,
                           pipeline_to_support_iter, pipeline_to_exists, replay_resource}};
            pub use cq_storage as storage;
            pub mod inputs {{ {provider_alias} }}
        }}
        use wiring as mini_linq;
        raw::lower_borrowed_nested! {{ {RA} indexes {{}} }}
        fn main() {{ let rows = vec![(1i32,), (1,), (2,)];
            let result: Vec<_> = Q::iter(&rows).map(|row| *row.0).collect();
            assert_eq!(result, vec![1, 2]); }}'''


def borrowed_resolution_source():
    """Stop at caller-selected Access endpoints; never simulate query semantics."""
    return f'''use std::sync::atomic::{{AtomicUsize, Ordering}};
static SUPPORT: AtomicUsize = AtomicUsize::new(0);
static BUILDS: AtomicUsize = AtomicUsize::new(0);
static EXISTENCE_REGIONS: AtomicUsize = AtomicUsize::new(0);
static OUTPUT: i32 = 42;
macro_rules! rows {{ (type ($($ty:ty,)*)) => {{ Vec<($($ty,)*)> }}; }}
macro_rules! support {{ ($($ir:tt)*) => {{{{
    let text = stringify!($($ir)*);
    assert!(text.contains("=>") && text.contains("for"), "expected Access, got {{text}}");
    SUPPORT.fetch_add(1, Ordering::SeqCst);
    if text.contains("access_to_exists") {{ EXISTENCE_REGIONS.fetch_add(1, Ordering::SeqCst); }}
    ::std::iter::empty::<(&i32,)>()
}}}}; }}
macro_rules! replay {{ (build {{ $($ir:tt)* }}) => {{{{
    let text = stringify!($($ir)*);
    assert!(text.contains("=>") && text.contains("for"), "expected Build of Access, got {{text}}");
    BUILDS.fetch_add(1, Ordering::SeqCst);
    0usize
}}}}; }}
macro_rules! output {{ ($($ir:tt)*) => {{{{
    let text = stringify!($($ir)*);
    assert!(text.contains("=>") && text.contains("for"), "expected Access, got {{text}}");
    ::std::iter::once((&OUTPUT,))
}}}}; }}
mod wiring {{
    pub(crate) use support as access_to_support_iter;
    pub(crate) use output as access_to_distinct_iter;
    pub(crate) use replay as replay_resource;
    pub mod inputs {{ pub(crate) use rows as R; }}
}}
use wiring as mini_linq;
raw::lower_borrowed_nested! {{ {RA} indexes {{}} }}
raw::lower_borrowed_nested! {{
    struct Shared; relation R(a:i32); answer(x) :- R(x);
    relational {{
        r = rename R {{a -> x}};
        s = semijoin r with r;
        output s as answer(x);
    }}
    indexes {{}}
}}
fn main() {{
    let rows = vec![(1i32,)];
    assert_eq!(Q::iter(&rows).next(), Some((&42,)));
    assert_eq!(SUPPORT.load(Ordering::SeqCst), 2);
    assert_eq!(Shared::iter(&rows).next(), Some((&42,)));
    assert_eq!(BUILDS.load(Ordering::SeqCst), 1);
    assert_eq!(SUPPORT.load(Ordering::SeqCst), 3);
    assert_eq!(EXISTENCE_REGIONS.load(Ordering::SeqCst), 1);
}}'''


def check_borrowed_rejections(destination, env, logdir):
    """Check the fully expanded borrowed route, never a sentinel's fake output."""
    base = borrowed_runtime_source()
    cases = {
        "temporary": (borrowed_runtime_source("($v).iter().copied()"), r"error\[E(?:0515|0597|0716)\]"),
        "missing-provider": (borrowed_runtime_source(bind_provider=False), r"could not find `R` in `inputs`"),
        "nonempty-indexes": (base.replace("indexes {}", "indexes { R[0]; }"), r"require an empty index overlay"),
        "selection": (base.replace("p = project r keep {x};", "p = select r if (x > 0);"), r"does not admit Select"),
        "extension": (base.replace("p = project r keep {x};", "e = extend r with { y: i32 } value ((x + 1,)); p = project e keep {x};"), r"does not admit Extend"),
        "bare-ra": (base.replace(" indexes {}", ""), r"expected `indexes`"),
        "inputs-envelope": (base.replace("raw::lower_borrowed_nested! {", "raw::lower_borrowed_nested! { inputs { R => rows; }", 1), r"expected `struct`"),
    }
    # Establish that all negative variants start from an executable fixture.
    cases = {"accepted": (base, None), **cases}
    for label, (source, expected) in cases.items():
        (destination / "src/main.rs").write_text(source)
        logpath = logdir / f"borrowed-route-{label}.log"
        command = ["cargo", "run" if expected is None else "check", "--manifest-path",
                   str(destination / "Cargo.toml"), "--offline"]
        with logpath.open("w") as log:
            result = subprocess.run(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT)
        if expected is None:
            if result.returncode: raise ValueError(f"borrowed rejection fixture does not execute: {logpath}")
        elif not result.returncode or not re.search(expected, logpath.read_text()):
            raise ValueError(f"borrowed route did not produce the expected {label} rejection: {logpath}")
        print(f"PASS: complete borrowed route {label}", flush=True)


def frontend_source(feature):
    """Atomic probes establish resolution or native behavior, not whole-query semantics."""
    macro = FRONTEND_MACROS[feature]
    if feature == "borrowed-nested":
        return borrowed_resolution_source()
    if feature in ("written-order", "clause-prefix", "hypergraph", "base-index-demands", "result-index-demands", "no-index-demands"):
        macro, source, next_name, marker = {
            "written-order": ("cq_to_ra_written_order", CQ, "choose_index_policy", "relational"),
            "clause-prefix": ("cq_to_ra_clause_prefix", CQ, "choose_index_policy", "relational"),
            "hypergraph": ("cq_to_ra_hypergraph", CQ, "choose_index_policy", "relational"),
            "base-index-demands": ("base_index_demands", RA, "choose_relation_lowering", "indexes"),
            "result-index-demands": ("result_index_demands", RA, "choose_relation_lowering", "indexes"),
            "no-index-demands": ("no_index_demands", RA, "choose_relation_lowering", "indexes"),
        }[feature]
        exact = 'assert!(SEEN.chars().filter(|c| !c.is_whitespace()).collect::<String>().ends_with("indexes{}"));' if feature == "no-index-demands" else ''
        return f'''#[macro_export] macro_rules! accept {{ ($($t:tt)*) => {{ const SEEN: &str = stringify!($($t)*); }}; }}
mod wiring {{ pub use crate::accept as {next_name}; }}
use wiring as mini_linq;
raw::{macro}! {{ {source} }}
fn main() {{ assert!(SEEN.contains("{marker}")); {exact} }}'''
    if feature == "access-to-pipeline":
        return f'''#[macro_export] macro_rules! output {{ ($($t:tt)*) => {{ (stringify!($($t)*), 42) }}; }}
#[macro_export] macro_rules! support {{ ($($t:tt)*) => {{ (stringify!($($t)*), 43) }}; }}
#[macro_export] macro_rules! exists {{ ($($t:tt)*) => {{ (stringify!($($t)*), 44) }}; }}
mod wiring {{ pub use crate::{{output as pipeline_to_distinct_iter, support as pipeline_to_support_iter, exists as pipeline_to_exists}}; }}
use wiring as mini_linq;
fn main() {{
    let (text, marker) = raw::access_to_distinct_iter! {{ {ACCESS} }};
    assert!(text.contains("scan") && text.contains("distinct")); assert_eq!(marker, 42);
    let (text, marker) = raw::access_to_support_iter! {{ {ACCESS} }};
    assert!(text.contains("scan") && text.contains("return")); assert_eq!(marker, 43);
    let (text, marker) = raw::access_to_exists! {{ {ACCESS} }};
    assert!(text.contains("scan") && text.contains("return")); assert_eq!(marker, 44);
}}'''
    if feature == "access-to-set-loops":
        return f'''fn main() {{
let rows = raw::access_to_set_loops! {{ {ACCESS} }};
assert_eq!(rows, ::std::collections::HashSet::from([(1i32,), (2,)]));
}}'''
    if feature == "pipeline-exists":
        return '''fn main() {
struct Payload;
let visited = ::std::cell::Cell::new(0);
let counter = &visited;
let exists = raw::pipeline_to_exists! {
    a = scan x in ((0..3).map(move |_| { counter.set(counter.get() + 1); Payload })) yield (x,);
    b = project a as (x,) yield ((x,));
    c = distinct b;
    return c;
};
assert!(exists); assert_eq!(visited.get(), 1);
assert!(!raw::pipeline_to_exists! {
    a = scan x in (::std::iter::empty::<Payload>()) yield (x,);
    b = distinct a;
    return b;
});
}'''
    if FRONTEND[feature][0] == "cq-pipeline-lowering":
        result = 'result.len()' if feature == 'pipeline-set' else 'result.count()'
        expected = 3 if feature == 'pipeline-support-iter' else 2
        return f'fn main() {{ let result = raw::{macro}! {{ {PIPELINE} }}; assert_eq!({result}, {expected}); }}'
    if feature in ("base-hash", "prefix-hash", "materialized-hash"):
        endpoint = '::std::collections::HashSet::<(i32,)>::new()' if feature == 'materialized-hash' else '::std::iter::empty::<(i32,)>()'
        continuation = 'access_to_set' if feature == 'materialized-hash' else 'access_to_distinct_iter'
        return f'''#[macro_export] macro_rules! chosen_access {{ ($($t:tt)*) => {{ {endpoint} }}; }}
mod wiring {{ pub use crate::chosen_access as {continuation}; }} use wiring as mini_linq;
raw::{macro}! {{ {RA} indexes {{}} }}
fn main() {{ let _ = Q::run([(1i32,)]); }}'''
    if feature == "configured":
        return '''macro_rules! rows {
(type ($($ty:ty,)*)) => { ::std::collections::HashSet<[i64;1]> };
(input ($v:expr)) => { $v }; (row ($v:expr)) => { $v };
(field ($v:expr) ($column:literal)) => { $v[$column] };
(len ($v:expr)) => { $v.len() }; (rows ($v:expr)) => { $v.iter() }; (scan ($v:expr)) => { $v.iter() };
(materialize $owner:ident $value:ident ($v:expr)) => { let $owner = $v; let $value = &$owner; };
}
macro_rules! chosen_access { ($($t:tt)*) => { ::std::collections::HashSet::<[i64;1]>::new() }; }
raw::configured_program! { access chosen_access; result_rows rows; candidates unused_scan; keys unused_keys; pairs unused_join;
inputs { R => rows; } struct Q; relation R(a:i64); answer(x) :- R(x);
relational { r = rename R {a -> x}; p = project r keep {x}; output p as answer(x); } }
fn main() { let rows = std::collections::HashSet::from([[1i64]]); let _ = Q::run(&rows); }'''
    if feature == "scan-candidates":
        return '''macro_rules! rows { (rows ($v:expr)) => { $v.into_iter() }; }
fn main() { let result = raw::scan_candidates! { work (compile_error!("unused observer")); candidates rows { [1,2] } (compile_error!("unused predicate")) };
assert_eq!(result.sum::<i32>(),3); }'''
    if feature == "prune-block-candidates":
        return '''macro_rules! chosen_access { ($($t:tt)*) => { [7i32].into_iter() }; }
fn main() { let result = raw::prune_block_candidates! { access_iter chosen_access; candidates unused_provider { () } (|_: &()| true) };
assert_eq!(result.sum::<i32>(),7); }'''
    if feature in ("exact-keys", "summarized-exact-keys"):
        storage = 'storage cq_storage;' if feature == 'summarized-exact-keys' else ''
        return f'''macro_rules! chosen_access {{ ($($t:tt)*) => {{ [[1i64],[3]].into_iter() }}; }}
fn main() {{ let keys = raw::{macro}! {{ {storage} access_iter chosen_access; keys ::std::collections::BTreeSet;
build {{ out(x) => ([x]) :- Rows(x) => for (x,) in (unused); }} }};
assert!(raw::{macro}! {{ storage cq_storage; contains(&keys)([1i64]) }});
assert!(!raw::{macro}! {{ storage cq_storage; contains(&keys)([2i64]) }}); }}'''
    if feature in ("hash-witnesses", "nested-witnesses"):
        return f'''macro_rules! chosen_access {{ ($($t:tt)*) => {{ [(1i32,10i32),(2,20)].into_iter() }}; }}
fn main() {{ let resource = raw::{macro}! {{ access_iter chosen_access;
build {{ out(x) => (x) :- Rows(x) => for (x,) in (unused); }} }};
let found: Vec<_> = raw::{macro}! {{ probe(&resource)(1i32) }}.collect(); assert_eq!(found,vec![10]); }}'''
    if feature in ("hash-pairs", "nested-pairs", "sort-merge-pairs"):
        return f'''fn main() {{
let left = [(1i32,10i32), (1,10), (2,20)];
let right = [(1i32,30i32), (1,40), (3,50)];
let mut rows = Vec::new();
raw::{macro}! {{ pairs l in (left.iter()) by (l.0), r in (right.iter()) by (r.0) {{ rows.push((l.1,r.1)); }} }};
rows.sort(); assert_eq!(rows, vec![(10,30),(10,30),(10,40),(10,40)]);
}}'''
    if feature in ("array-hash-rows", "array-sorted-rows"):
        return f'''fn main() {{ let input = [[1i64],[1],[2]];
raw::{macro}! {{ materialize owner value (input) }}
let _: &raw::{macro}! {{ type (i64,) }} = value;
assert_eq!(raw::{macro}! {{ rows(value) }}.count(),2); }}'''
    if feature in ("tuple-hash-rows", "tuple-sorted-rows"):
        return f'''fn main() {{
raw::{macro}! {{ produce owner value via emit {{
    emit((String::from("a"), 1i32)); emit((String::from("a"), 1i32)); emit((String::from("b"), 2i32));
}} }}
let _: &raw::{macro}! {{ type (String, i32,) }} = value;
assert_eq!(raw::{macro}! {{ rows(value) }}.count(), 2);
}}'''
    if feature == "array-block-rows":
        return '''fn main() { let input = std::collections::HashSet::from([[1i64],[2]]);
raw::array_block_rows! { storage cq_storage; packing (::std::num::NonZeroUsize::new(1).unwrap()); materialize owner value (input) }
let _: &raw::array_block_rows! { storage cq_storage; type (i64,) } = &owner;
assert_eq!(raw::array_block_rows! { storage cq_storage; rows(&value) }.count(),2); }'''
    if feature == "replay-resource":
        return f'''use std::sync::atomic::{{AtomicUsize, Ordering}};
static VISITS: AtomicUsize = AtomicUsize::new(0);
macro_rules! support {{ ($($ir:tt)*) => {{{{
    let text = stringify!($($ir)*);
    assert!(text.contains("=>") && text.contains("for"));
    [1i32, 2].into_iter().inspect(|_| {{ VISITS.fetch_add(1, Ordering::SeqCst); }})
}}}}; }}
mod wiring {{ pub use cq_storage as storage; pub(crate) use support as access_to_support_iter; }}
use wiring as mini_linq;
fn main() {{
    let rows = raw::replay_resource! {{ build {{ {ACCESS} }} }};
    let mut first = raw::replay_resource! {{ scan (rows) }};
    let mut second = raw::replay_resource! {{ scan (rows) }};
    assert_eq!(VISITS.load(Ordering::SeqCst), 0);
    assert_eq!(first.next(), Some(1));
    assert_eq!(second.next(), Some(1));
    assert_eq!(VISITS.load(Ordering::SeqCst), 1);
    assert_eq!(second.next(), Some(2));
    assert_eq!(first.next(), Some(2));
    assert_eq!(VISITS.load(Ordering::SeqCst), 2);
    assert_eq!(first.next(), None); assert_eq!(second.next(), None);
}}'''
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
            runtime = feature in ("exact-keys", "summarized-exact-keys", "array-block-rows", "replay-resource")
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
            kind = ("resolution probe" if feature in RESOLUTION_PROBES else
                    "Pipeline endpoint probe" if name == "cq-pipeline-lowering" else "native-operation probe")
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
