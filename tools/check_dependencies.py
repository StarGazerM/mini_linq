#!/usr/bin/env python3
"""Check active compiler dependencies, source isolation and real macro boundaries.

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
    "cq-query-lowering", "cq-index-policy", "cq-stream-lowering",
    "cq-stream-realization", "cq-access-lowering", "cq-pipeline-lowering")}
PROVIDERS = {"cq-state-providers": {"cq-ir"}}
TEST_DEPENDENCIES = {
    "cq-stream-lowering": {"cq-index-policy"},
    "cq-stream-realization": {"cq-index-policy", "cq-stream-lowering"},
}
ALLOWED = FOUNDATIONS | FAMILIES | PROVIDERS | {"mini-linq": {"cq-ir", *FAMILIES}}
FRONTEND = {
    "written-order": ("cq-query-lowering", "written-order"),
    "clause-prefix": ("cq-query-lowering", "clause-prefix"),
    "hypergraph": ("cq-query-lowering", "hypergraph"),
    "index-demands": ("cq-index-policy", None),
    "stream-lowering": ("cq-stream-lowering", None),
    "stream-realization": ("cq-stream-realization", None),
    "access-to-pipeline": ("cq-access-lowering", "to-pipeline"),
    "access-to-set-loops": ("cq-access-lowering", "to-set-loops"),
    "pipeline-distinct-iter": ("cq-pipeline-lowering", "distinct-iter"),
    "pipeline-set": ("cq-pipeline-lowering", "collect-set"),
    "pipeline-support-iter": ("cq-pipeline-lowering", "support-iter"),
    "pipeline-exists": ("cq-pipeline-lowering", "exists"),
}
FRONTEND_MACROS = {
    "written-order": "cq_to_ra_written_order", "clause-prefix": "cq_to_ra_clause_prefix",
    "hypergraph": "cq_to_ra_hypergraph", "index-demands": "index_demands",
    "stream-lowering": "ix_to_stream", "stream-realization": "stream_to_access",
    "access-to-pipeline": "access_to_distinct_iter", "access-to-set-loops": "access_to_set_loops",
    "pipeline-distinct-iter": "pipeline_to_distinct_iter", "pipeline-set": "pipeline_to_set",
    "pipeline-support-iter": "pipeline_to_support_iter", "pipeline-exists": "pipeline_to_exists",
}
# This fixed build inventory is not provider selection in the query compiler.
# Primary directories must not import or condition behavior on a sibling feature.
IMPLEMENTATIONS = {
    "cq-query-lowering": {
        "written-order": ["src/written_order", "tests/written_order.rs"],
        "clause-prefix": ["src/clause_prefix", "src/dependencies.rs", "src/model.rs", "src/proof.rs", "tests/clause_prefix.rs"],
        "hypergraph": ["src/hypergraph", "src/dependencies.rs", "src/model.rs", "src/proof.rs", "tests/hypergraph.rs"],
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
MODULE_WIRING = {}
CONSUMERS = {
    "state_providers": ["pipeline-distinct-iter", "pipeline-set", "pipeline-support-iter", "pipeline-exists", "access-to-set-loops"],
    "stream_query": ["written-order", "hypergraph", "index-demands", "stream-lowering", "stream-realization",
                     "access-to-pipeline", "access-to-set-loops", "pipeline-distinct-iter", "pipeline-set", "pipeline-support-iter"],
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
        if name != "mini-linq":
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
    if set(frontend["features"]) != set(FRONTEND) | {"default", "logical-selection"}:
        raise ValueError("frontend feature inventory differs from the active boundary contracts")
    if frontend["features"]["logical-selection"] != ["written-order", "clause-prefix", "hypergraph"]:
        raise ValueError("logical-selection acquired dependencies outside logical planning")
    for dep in frontend["dependencies"]:
        if dep["kind"] is None and dep["name"] in set(FAMILIES):
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
        package = next(p for p in altered["packages"] if p["name"] == "cq-stream-realization")
        dep = {"name": "cq-state-providers", "kind": kind, "optional": True, "rename": "innocent_alias",
               "target": "cfg(unix)", "uses_default_features": False}
        if not remote: dep["path"] = str(ROOT / "crates/cq-state-providers")
        package["dependencies"].append(dep)
        try: check(altered, announce=False)
        except ValueError: pass
        else: raise ValueError("dependency guard accepted a forbidden renamed/optional/target dependency")
    altered = copy.deepcopy(data)
    package = next(p for p in altered["packages"] if p["name"] == "cq-stream-realization")
    package["dependencies"].append({"name": "cq-state-providers", "kind": "dev", "path": str(ROOT / "crates/cq-state-providers")})
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
    for text in ('use crate::hypergraph;', '#[cfg(feature = "hypergraph")] fn route() {}'):
        try: audit_implementation_text("cq-query-lowering", "written-order", "mutation.rs", text)
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
    jobs = [(name, None) for name in FOUNDATIONS | PROVIDERS | {name: dependencies for name, dependencies in FAMILIES.items() if name not in IMPLEMENTATIONS}]
    jobs += [(name, feature) for name, variants in IMPLEMENTATIONS.items() for feature in variants]
    for name, feature in jobs:
        label = name + ("-" + feature if feature else "")
        if only and label not in only and name not in only: continue
        with tempfile.TemporaryDirectory(prefix=f"mini-linq-{label}-") as temporary:
            destination = Path(temporary)
            selected = {name} | ALLOWED[name] | TEST_DEPENDENCIES.get(name, set())
            write_workspace(destination, selected)
            for package in selected: copy_selected(destination, package, [feature] if package == name and feature else [])
            env = os.environ.copy(); env["CARGO_TARGET_DIR"] = str(ROOT / "target/component-isolation"); env["CARGO_TERM_COLOR"] = "never"
            command = ["cargo", "test", "-p", name, "--all-targets", "--offline"] + (["--features", feature] if feature else [])
            with (logdir / f"{label}.log").open("w") as log:
                result = subprocess.run(command, cwd=destination, env=env, stdout=log, stderr=subprocess.STDOUT)
            if result.returncode: raise ValueError(f"source-removal test failed: {label}; see {logdir / (label + '.log')}")
            print(f"PASS: {label} builds/tests with alternative implementation files absent", flush=True)


CQ = 'struct Q; relation R(a:i32); answer(x) :- R(x);'
RA = CQ + ' relational { r = rename R {a -> x}; output r as answer(x); }'
IX = RA + ' indexes {} uses {}'
POLICED_IX = IX + ' policy { R provides { iter }; }'
STREAM = IX + ' stream { open R provides { iter }; r = alias R; return r; }'
ACCESS = 'answer(x) => ((x,)) :- R(x) => bind (x,) <- ([(1i32,), (2,)]);'
PIPELINE = 'iter0 = scan (x,) in ([(1i32,), (1,), (2,)]) yield (x,); iter1 = project iter0 as (x,) yield ((x,)); iter2 = distinct iter1; return iter2;'


STATE_BINDINGS = r"""
macro_rules! distinct_state {
    (operator $id:ident; new) => { ::std::collections::HashSet::new() };
    (operator $id:ident; insert($state:expr, $row:expr)) => { $state.insert(Clone::clone($row)) };
}
macro_rules! set_sink {
    (operator $id:ident; new) => { ::std::collections::HashSet::new() };
    (operator $id:ident; insert($state:expr, $row:expr)) => {{ $state.insert($row); }};
    (operator $id:ident; finish($state:expr)) => { $state };
}
#[allow(unused_imports)]
pub(crate) use {distinct_state, set_sink};
#[allow(unused_imports)]
mod wiring { pub(crate) use super::{distinct_state, set_sink}; }
use wiring as mini_linq;
"""

def frontend_source(feature):
    """Valid, small calls: boundary captures are not end-to-end acceptance."""
    implementation = FRONTEND_MACROS[feature]
    if feature in ("written-order", "clause-prefix", "hypergraph", "index-demands", "stream-lowering"):
        source, continuation, expected = {
            "written-order": (CQ, "choose_index_policy", ["relational", "rename"]),
            "clause-prefix": (CQ, "choose_index_policy", ["relational", "rename"]),
            "hypergraph": (CQ, "choose_index_policy", ["relational", "rename"]),
            "index-demands": (RA, "stream_policy", ["indexes", "uses"]),
            "stream-lowering": (POLICED_IX, "stream_to_access", ["stream", "alias", "return"]),
        }[feature]
        checks = " ".join(f'assert!(CAPTURE.contains({json.dumps(word)}));' for word in expected)
        return f'''
macro_rules! capture {{ ($($tokens:tt)*) => {{ const CAPTURE: &str = stringify!($($tokens)*); }}; }}
use capture as {continuation};
mod wiring {{ pub(crate) use super::{continuation}; }}
use wiring as mini_linq;
raw::{implementation}! {{ {source} }}
fn main() {{ {checks} }}
'''
    if feature == "stream-realization":
        return f'''
macro_rules! capture {{ ($($tokens:tt)*) => {{ stringify!($($tokens)*) }}; }}
use capture as access;
mod wiring {{ pub(crate) use super::access; }}
use wiring as mini_linq;
fn main() {{
    let region = raw::{implementation}! {{ {STREAM} }};
    assert!(region.contains("bind"));
    assert!(region.contains("iter"));
}}
'''
    if feature == "access-to-pipeline":
        return f'''
macro_rules! capture {{ ($($tokens:tt)*) => {{ stringify!($($tokens)*) }}; }}
use capture as pipeline_to_distinct_iter;
mod wiring {{ pub(crate) use super::pipeline_to_distinct_iter; }}
use wiring as mini_linq;
fn main() {{
    let pipeline = raw::{implementation}! {{ {ACCESS} }};
    assert!(pipeline.contains("scan"));
    assert!(pipeline.contains("distinct"));
    assert!(pipeline.contains("return"));
}}
'''
    if feature == "access-to-set-loops":
        return STATE_BINDINGS + f'fn main() {{ assert_eq!(raw::{implementation}! {{ {ACCESS} }}, std::collections::HashSet::from([(1,), (2,)])); }}'
    if feature == "pipeline-exists":
        return f'fn main() {{ assert!(raw::{implementation}! {{ {PIPELINE} }}); }}'
    if feature == "pipeline-set":
        return STATE_BINDINGS + f'fn main() {{ assert_eq!(raw::{implementation}! {{ {PIPELINE} }}, std::collections::HashSet::from([(1,), (2,)])); }}'
    expected = 'vec![(1,), (1,), (2,)]' if feature == "pipeline-support-iter" else 'vec![(1,), (2,)]'
    return ('' if feature == 'pipeline-support-iter' else STATE_BINDINGS) + f'fn main() {{ assert_eq!(raw::{implementation}! {{ {PIPELINE} }}.collect::<Vec<_>>(), {expected}); }}'


def all_feature_checks(only=()):
    # Check combined implementation features separately from source removal.
    logdir = ROOT / "target/component-logs"; logdir.mkdir(parents=True, exist_ok=True)
    for name in IMPLEMENTATIONS:
        if only and name not in only and not any(label.startswith(name + "-") for label in only): continue
        with (logdir / f"all-features-{name}.log").open("w") as log:
            result = subprocess.run(["cargo", "test", "-p", name, "--all-features", "--all-targets", "--offline", "--locked"],
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
            (destination / "Cargo.toml").write_text(
                '[package]\nname = "frontend-probe"\nversion = "0.0.0"\nedition = "2024"\n[workspace]\n[dependencies]\n'
                'raw = { package = "mini-linq", path = ' + toml(str(ROOT / "crates/mini-linq"))
                + ', default-features = false, features = [' + toml(feature) + '] }\n'
                )
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
            kind = ("boundary capture" if feature in {"written-order", "clause-prefix", "hypergraph", "index-demands", "stream-lowering", "stream-realization", "access-to-pipeline"} else "execution smoke")
            component = name + (f"/{implementation}" if implementation else "")
            print(f"PASS: {kind} {feature}; renamed dependency and isolated {component}", flush=True)


def check_consumers(only=()):
    logdir = ROOT / "target/component-logs"
    logdir.mkdir(parents=True, exist_ok=True)
    for target, features in CONSUMERS.items():
        if only and target not in only and "mini-linq" not in only:
            continue
        path = logdir / f"consumer-{target}.log"
        command = ["cargo", "test", "-p", "mini-linq", "--no-default-features", "--features", ",".join(features), "--test", target, "--locked", "--offline"]
        with path.open("w") as log:
            result = subprocess.run(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
        if result.returncode:
            raise ValueError(f"integration consumer failed: {target}; see {path}")
        print(f"PASS: actual integration tests {target}; see {path}", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--isolate", action="store_true", help="test copied components with unselected implementation sources absent, then invoke isolated macros")
    parser.add_argument("--frontends", action="store_true", help="invoke each leaf-feature macro with boundary captures or small executable input")
    parser.add_argument("--consumers", action="store_true", help="run current IX/Stream integration test targets; separate from boundary smoke")
    parser.add_argument("--only", action="append", default=[], help="limit probes by component, implementation label, frontend feature, or integration target")
    args = parser.parse_args()
    labels = set(ALLOWED) | set(FRONTEND) | set(CONSUMERS) | {f"{name}-{feature}" for name, variants in IMPLEMENTATIONS.items() for feature in variants}
    if set(args.only) - labels:
        parser.error("unknown --only label(s): " + ", ".join(sorted(set(args.only) - labels)))
    data = metadata()
    packages = check(data)
    negative_dependency_checks(data)
    audit_imports()
    if args.isolate:
        isolate(packages, args.only)
        all_feature_checks(args.only)
    if args.isolate or args.frontends:
        check_frontends(packages, args.only)
    if args.consumers:
        check_consumers(args.only)


if __name__ == "__main__":
    main()
