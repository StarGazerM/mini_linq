#!/usr/bin/env python3
"""Check active compiler dependencies, source isolation and real macro boundaries.

This inventory describes build ownership only. It is not compiler dispatch.
Production isolation, macro resolution and semantic consumers are separate
gates. Package counts and test-only dependencies are not proofs of compiler
architecture.
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


def inventory(folder):
    return {p.parent.name: p.parent.relative_to(ROOT) for p in (ROOT / folder).glob("*/Cargo.toml")}


LANGUAGES = inventory("languages")
FAMILIES = inventory("passes")
RUNTIME = inventory("runtime")
FACADES = inventory("facade")
PATHS = LANGUAGES | FAMILIES | RUNTIME | FACADES
# The crate declares the languages its pass transforms, not an arbitrary allowlist.
EDGES = {}
for name, path in FAMILIES.items():
    spec = tomllib.loads((ROOT / path / "Cargo.toml").read_text())["package"]["metadata"]["cq"]
    EDGES[name] = spec
ALLOWED = set(PATHS)

# Each frontend feature enables exactly the passes its macro runs.
FRONTEND = {
    "written-order": ("cq-order", "cq-elaborate"),
    "hypergraph": ("cq-order", "cq-reduce", "cq-elaborate"),
    "index-demands": ("cq-index-policy",),
    "physical-lowering": ("cq-physical-lowering",),
    "structure-selection": ("cq-structure-selection",),
    "physical-realization": ("cq-physical-realization",),
}
FRONTEND_MACROS = {
    "written-order": "cq_to_ra", "hypergraph": "cq_to_ra_hypergraph", "index-demands": "index_demands",
    "physical-lowering": "lower_physical", "structure-selection": "select_structures",
    "physical-realization": "structures_to_rust",
}
CONSUMERS = {
    "wcoj": ["written-order", "hypergraph", "index-demands", "physical-lowering", "structure-selection",
             "physical-realization"],
    "physical_query": ["written-order", "hypergraph", "index-demands", "physical-lowering", "structure-selection",
                       "physical-realization"],
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


def production(package):
    return [d for d in package["dependencies"] if d["kind"] != "dev"]


def check(data, announce=True):
    packages = {p["name"]: p for p in data["packages"]}
    for name, path in PATHS.items():
        package = packages[name]
        internal = {d["name"] for d in production(package) if "path" in d or d["name"] in packages}
        if name in LANGUAGES:
            base = {"cq-lang-vo": "cq-lang-cq", "cq-lang-ix": "cq-lang-ra", "cq-lang-structures": "cq-lang-physical"}.get(name)
            allowed = ({"cq-syntax", base} - {None, name})
            if package["features"]:
                raise ValueError(f"language features are forbidden: {name}")
        elif name in FAMILIES:
            edge = EDGES[name]
            required = {edge["source"], edge["target"]} - {"rust"}
            if not required <= set(LANGUAGES): raise ValueError(f"pass endpoints are not languages: {name}")
            allowed = required | {"cq-syntax"} | ({"cq-print"} if name == "cq-physical-realization" else set())
            if internal & (set(LANGUAGES) - {"cq-syntax"}) != required:
                raise ValueError(f"pass must depend on exactly its source and target languages: {name}: {internal}")
            if package["features"]:
                raise ValueError(f"pass features are forbidden: {name}")
        elif name in RUNTIME:
            allowed = {"cq-syntax", "cq-protocol"} - {name}
        else:
            allowed = set(packages)
        for dep in package["dependencies"]:
            target = dep["name"]
            if "path" not in dep and target not in packages: continue
            expected = Path(packages[target]["manifest_path"]).parent if target in packages else None
            if target not in allowed or "path" not in dep or Path(dep["path"]).resolve() != expected:
                raise ValueError(f"forbidden {dep['kind'] or 'normal'} dependency: {name} -> {target}")
        for target in package["targets"]:
            if set(target["kind"]) & {"test", "example", "bench"}: continue
            if not Path(target["src_path"]).resolve().is_relative_to(ROOT / path):
                raise ValueError(f"source escapes its crate: {name}: {target['src_path']}")
    frontend = packages["mini-linq"]
    for feature, names in FRONTEND.items():
        if frontend["features"].get(feature) != [f"dep:{name}" for name in names]:
            raise ValueError(f"frontend feature acquired hidden dependencies: {feature}")
    for dep in production(frontend):
        if dep["name"] in FAMILIES and not dep["optional"]:
            raise ValueError(f"frontend unconditionally includes a pass: {dep['name']}")
    if announce: print("PASS: folder-derived language, pass, runtime and facade boundaries", flush=True)
    return packages


def negative_dependency_checks(data):
    mutations = [("cq-physical-realization", "cq-storage", None, False),
                 ("cq-physical-realization", "cq-state-providers", "build", False),
                 ("cq-physical-lowering", "cq-lang-ra", None, False),
                 ("cq-lang-ix", "cq-storage", None, False),
                 ("cq-lang-ix", "cq-lang-physical", None, False),
                 ("cq-physical-lowering", "cq-lang-vo", "dev", False),
                 ("cq-physical-realization", "cq-storage", None, True)]
    for owner, target, kind, remote in mutations:
        altered = copy.deepcopy(data)
        package = next(p for p in altered["packages"] if p["name"] == owner)
        dep = {"name": target, "kind": kind, "optional": True, "rename": "innocent_alias", "target": "cfg(unix)"}
        if not remote: dep["path"] = str(ROOT / PATHS[target])
        package["dependencies"].append(dep)
        try: check(altered, announce=False)
        except ValueError: pass
        else: raise ValueError(f"guard accepted forbidden edge: {owner} -> {target}")
    for owner in ("cq-lang-ra", "cq-physical-lowering"):
        altered = copy.deepcopy(data)
        next(p for p in altered["packages"] if p["name"] == owner)["features"]["hidden"] = []
        try: check(altered, announce=False)
        except ValueError: pass
        else: raise ValueError(f"guard accepted language/pass feature: {owner}")
    print(f"PASS: {len(mutations)} forbidden dependency and 2 feature mutations rejected", flush=True)


def closure(packages, name):
    selected, pending = set(), [name]
    while pending:
        current = pending.pop()
        if current in selected: continue
        selected.add(current)
        pending.extend(d["name"] for d in production(packages[current]) if d["name"] in PATHS)
    return selected


def write_workspace(destination, selected):
    original = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]
    lines = ["[workspace]", 'resolver = "3"', "members = " + toml([str(PATHS[p]) for p in sorted(selected)]), "", "[workspace.package]"]
    lines += [f"{k} = {toml(v)}" for k, v in original["package"].items()]
    lines += ["", "[workspace.dependencies]"]
    lines += [f"{k} = {toml(v)}" for k, v in original["dependencies"].items()
              if not isinstance(v, dict) or "path" not in v or k in selected]
    (destination / "Cargo.toml").write_text("\n".join(lines) + "\n")
    shutil.copy2(ROOT / "rust-toolchain.toml", destination / "rust-toolchain.toml")


def copy_component(destination, name, selected):
    base = ROOT / PATHS[name]
    output = destination / PATHS[name]
    shutil.copytree(base / "src", output / "src")
    if (base / "tests").exists(): shutil.copytree(base / "tests", output / "tests")
    manifest = tomllib.loads((base / "Cargo.toml").read_text())
    tests = manifest.pop("test", [])
    manifest["package"].pop("metadata", None)
    for kind in ("dependencies", "dev-dependencies", "build-dependencies"):
        for dep in manifest.get(kind, {}):
            if dep in PATHS and dep not in selected:
                raise ValueError(f"isolated {name} test adds an undeclared language or pass: {dep}")
    lines = []
    for section, values in manifest.items():
        lines.append(f"[{section}]")
        lines.extend(f"{k} = {toml(v)}" for k, v in values.items())
        lines.append("")
    for test in tests:
        lines += ["[[test]]"] + [f"{k} = {toml(v)}" for k, v in test.items()]
    (output / "Cargo.toml").write_text("\n".join(lines) + "\n")


def isolate(packages, only=()):
    logdir = ROOT / "target/component-logs"; logdir.mkdir(parents=True, exist_ok=True)
    for name in LANGUAGES | RUNTIME | FAMILIES:
        if only and name not in only: continue
        selected = closure(packages, name) | {"cq-syntax"}
        with tempfile.TemporaryDirectory(prefix=f"mini-linq-{name}-") as temporary:
            destination = Path(temporary)
            write_workspace(destination, selected)
            for package in selected: copy_component(destination, package, selected)
            shutil.copytree(ROOT / "test-support", destination / "test-support")
            shutil.copytree(ROOT / "teaching", destination / "teaching")
            # Runtime source is a generated-code execution fixture, never a compiler dependency.
            shutil.copytree(ROOT / "runtime/cq-storage/src", destination / "runtime/cq-storage/src", dirs_exist_ok=True)
            env = os.environ.copy(); env["CARGO_TARGET_DIR"] = str(ROOT / "target/component-isolation"); env["CARGO_TERM_COLOR"] = "never"
            command = ["cargo", "test", "-p", name, "--no-default-features", "--offline"]
            with (logdir / f"{name}.log").open("w") as log:
                result = subprocess.run(command, cwd=destination, env=env, stdout=log, stderr=subprocess.STDOUT)
            if result.returncode: raise ValueError(f"isolation failed: {name}; see {logdir / (name + '.log')}")
            if name in FAMILIES | LANGUAGES:
                if not re.search(r"test result: ok\. [1-9][0-9]* passed", (logdir / f"{name}.log").read_text()):
                    raise ValueError(f"isolation executed no tests: {name}")
            detail = "with only: " + ", ".join(sorted(selected - {name}))
            print(f"PASS: {name} {detail}", flush=True)


CQ = 'struct Q; relation R(a:i32); answer(x) :- R(x);'
RA = 'relational { relation R(a:i32); r = rename R {a -> x}; output r as answer(x); }'
IX = RA + ' indexes {}'
POLICED_IX = IX + ' policy { R canon; }'
PHYSICAL = 'physical { relation R(a:i32); r = alias rename R {a -> x}; return r as answer(x); }'
POLICED_PHYSICAL = PHYSICAL + ' policy { R canon; }'
STRUCTURES = PHYSICAL + ' structures { R canon; }'


STATE_BINDINGS = r"""
macro_rules! distinct_state {
    (operator $id:ident; new) => { ::std::collections::HashSet::new() };
    (operator $id:ident; insert($state:expr, $row:expr)) => { $state.insert(Clone::clone($row)) };
}
#[allow(unused_imports)]
pub(crate) use distinct_state;
#[allow(unused_imports)]
mod wiring { pub(crate) use super::distinct_state; }
use wiring as mini_linq;
"""

def frontend_source(feature):
    """Valid, small calls: boundary captures are not end-to-end acceptance."""
    implementation = FRONTEND_MACROS[feature]
    if feature in ("written-order", "hypergraph", "index-demands", "physical-lowering", "structure-selection"):
        source, continuation, expected = {
            "written-order": (CQ, "choose_index_policy", ["relational", "rename"]),
            "hypergraph": (CQ, "choose_index_policy", ["relational", "rename"]),
            "index-demands": (RA, "physical_policy", ["indexes"]),
            "physical-lowering": (POLICED_IX, "select_structures", ["physical", "alias", "policy"]),
            "structure-selection": (POLICED_PHYSICAL, "structures_to_rust", ["structures", "R"]),
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
    assert feature == "physical-realization", feature
    return STATE_BINDINGS + f'''
fn main() {{
    let R: &::std::collections::BTreeSet<(i32,)> = &[(1,), (2,)].into_iter().collect();
    let rows: Vec<_> = raw::{implementation}! {{ {STRUCTURES} policy {{ R canon; }} }}.collect();
    assert_eq!(rows.len(), 2);
}}
'''
def check_frontends(packages, only=()):
    env = os.environ.copy(); env["CARGO_TARGET_DIR"] = str(ROOT / "target/frontend-isolation"); env["CARGO_TERM_COLOR"] = "never"
    logdir = ROOT / "target/component-logs"; logdir.mkdir(parents=True, exist_ok=True)
    for feature, names in FRONTEND.items():
        if only and feature not in only and not set(names) & set(only): continue
        with tempfile.TemporaryDirectory(prefix="mini-linq-frontend-") as temporary:
            destination = Path(temporary); (destination / "src").mkdir()
            (destination / "src/main.rs").write_text(frontend_source(feature))
            (destination / "Cargo.toml").write_text(
                '[package]\nname = "frontend-probe"\nversion = "0.0.0"\nedition = "2024"\n[workspace]\n[dependencies]\n'
                'raw = { package = "mini-linq", path = ' + toml(str(ROOT / "facade/mini-linq"))
                + ', default-features = false, features = [' + toml(feature) + '] }\n'
                + ('cq-storage = { path = ' + toml(str(ROOT / "runtime/cq-storage")) + ' }\n' if feature == "physical-realization" else '')
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
            actual = {by_id[i] for i in seen} & set(FAMILIES)
            expected = set(names) | ({"cq-print"} if "cq-physical-realization" in names else set())
            if actual != expected: raise ValueError(f"frontend {feature} includes {actual}, expected {expected}")
            for name in names:
                active = set(nodes[next(i for i, n in by_id.items() if n == name)]["features"])
                if active: raise ValueError(f"frontend {feature} enables {name} features {active}")
            with (logdir / f"frontend-{feature}.log").open("w") as log:
                result = subprocess.run(["cargo", "run", *args], cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT)
            if result.returncode: raise ValueError(f"actual frontend {feature} failed; see {logdir / ('frontend-' + feature + '.log')}")
            kind = ("boundary capture" if feature in {"written-order", "hypergraph", "index-demands", "physical-lowering", "structure-selection"} else "execution smoke")
            print(f"PASS: {kind} {feature}; renamed dependency and isolated {' + '.join(names)}", flush=True)


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
    parser.add_argument("--isolate", action="store_true", help="test copied components with only their declared dependencies present, then invoke isolated macros")
    parser.add_argument("--frontends", action="store_true", help="invoke each leaf-feature macro with boundary captures or small executable input")
    parser.add_argument("--consumers", action="store_true", help="run current IX/Physical integration test targets; separate from boundary smoke")
    parser.add_argument("--only", action="append", default=[], help="limit probes by component, frontend feature, or integration target")
    args = parser.parse_args()
    labels = set(ALLOWED) | set(FRONTEND) | set(CONSUMERS)
    if set(args.only) - labels:
        parser.error("unknown --only label(s): " + ", ".join(sorted(set(args.only) - labels)))
    data = metadata()
    packages = check(data)
    negative_dependency_checks(data)
    if args.isolate:
        isolate(packages, args.only)
    if args.isolate or args.frontends:
        check_frontends(packages, args.only)
    if args.consumers:
        check_consumers(args.only)


if __name__ == "__main__":
    main()
