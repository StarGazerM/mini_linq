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
FOUNDATIONS = {"cq-ir": set(), "cq-storage": set()}
# One pass per crate; each depends on the IR only. Planning is the chain
# cq-order (NF → VO), cq-reduce (VO → VO), cq-elaborate (VO → RA).
FAMILIES = {name: {"cq-ir"} for name in (
    "cq-order", "cq-reduce", "cq-elaborate", "cq-index-policy", "cq-physical-lowering",
    "cq-structure-selection", "cq-physical-realization")}
PROVIDERS = {"cq-state-providers": {"cq-ir"}}
PLANNING = {"cq-order", "cq-reduce", "cq-elaborate"}
TEST_DEPENDENCIES = {
    "cq-reduce": {"cq-elaborate"},
    "cq-physical-lowering": PLANNING | {"cq-index-policy"},
    "cq-structure-selection": PLANNING | {"cq-index-policy", "cq-physical-lowering"},
    "cq-physical-realization": PLANNING | {"cq-index-policy", "cq-physical-lowering", "cq-structure-selection"},
}
ALLOWED = FOUNDATIONS | FAMILIES | PROVIDERS | {"mini-linq": {"cq-ir", *FAMILIES}}
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
    for name in FAMILIES:
        if packages[name]["features"].get("default") != []: raise ValueError(f"family has implicit features: {name}")
    frontend = packages["mini-linq"]
    if set(frontend["features"]) != set(FRONTEND) | {"default", "logical-selection"}:
        raise ValueError("frontend feature inventory differs from the active boundary contracts")
    if frontend["features"]["logical-selection"] != ["written-order", "hypergraph"]:
        raise ValueError("logical-selection acquired dependencies outside logical planning")
    for dep in frontend["dependencies"]:
        if dep["kind"] is None and dep["name"] in set(FAMILIES) and not dep["optional"]:
            raise ValueError(f"frontend unconditionally includes a pass: {dep['name']}")
    for feature, names in FRONTEND.items():
        if frontend["features"].get(feature) != [f"dep:{name}" for name in names]:
            raise ValueError(f"frontend feature acquired hidden dependencies: {feature}")
    if announce: print(f"PASS: production dependency boundaries and {len(FRONTEND)} scoped frontend feature contracts; test dependencies may compose passes", flush=True)
    return packages


def negative_dependency_checks(data):
    # Metadata normalizes package aliases, optional and target-specific deps.
    for kind, remote in [(None, False), ("build", False), (None, True), ("build", True)]:
        altered = copy.deepcopy(data)
        package = next(p for p in altered["packages"] if p["name"] == "cq-physical-realization")
        dep = {"name": "cq-state-providers", "kind": kind, "optional": True, "rename": "innocent_alias",
               "target": "cfg(unix)", "uses_default_features": False}
        if not remote: dep["path"] = str(ROOT / "crates/cq-state-providers")
        package["dependencies"].append(dep)
        try: check(altered, announce=False)
        except ValueError: pass
        else: raise ValueError("dependency guard accepted a forbidden renamed/optional/target dependency")
    altered = copy.deepcopy(data)
    package = next(p for p in altered["packages"] if p["name"] == "cq-physical-realization")
    package["dependencies"].append({"name": "cq-state-providers", "kind": "dev", "path": str(ROOT / "crates/cq-state-providers")})
    check(altered, announce=False)
    print("PASS: four production dependency mutations rejected; test dependency accepted", flush=True)


def source_files(name):
    base = ROOT / "crates" / name
    return {p.relative_to(base).as_posix() for folder in ("src", "tests") for p in (base / folder).rglob("*.rs")}


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


def copy_under_test(destination, name):
    """The crate under test is copied with its sources and test targets."""
    base = ROOT / "crates" / name
    output = destination / "crates" / name
    output.mkdir(parents=True)
    shutil.copy2(base / "Cargo.toml", output / "Cargo.toml")
    paths = source_files(name)
    for path in paths:
        target = output / path
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(base / path, target)
    return paths


def copy_whole(destination, name):
    """A dependency is copied as published: all sources, no test targets."""
    base = ROOT / "crates" / name
    output = destination / "crates" / name
    shutil.copytree(base / "src", output / "src")
    manifest = re.sub(r'\n\[\[test\]\]\n(.*?)(?=\n\[|\Z)', "", (base / "Cargo.toml").read_text(), flags=re.S)
    (output / "Cargo.toml").write_text(manifest)


def isolate(packages, only=()):
    """Each component builds and tests in a workspace holding only its declared dependencies."""
    logdir = ROOT / "target/component-logs"; logdir.mkdir(parents=True, exist_ok=True)
    for name in FOUNDATIONS | PROVIDERS | FAMILIES:
        if only and name not in only: continue
        with tempfile.TemporaryDirectory(prefix=f"mini-linq-{name}-") as temporary:
            destination = Path(temporary)
            selected = {name} | ALLOWED[name] | TEST_DEPENDENCIES.get(name, set())
            write_workspace(destination, selected)
            for package in selected:
                if package == name:
                    copy_under_test(destination, package)
                else:
                    copy_whole(destination, package)
            env = os.environ.copy(); env["CARGO_TARGET_DIR"] = str(ROOT / "target/component-isolation"); env["CARGO_TERM_COLOR"] = "never"
            command = ["cargo", "test", "-p", name, "--all-targets", "--offline"]
            with (logdir / f"{name}.log").open("w") as log:
                result = subprocess.run(command, cwd=destination, env=env, stdout=log, stderr=subprocess.STDOUT)
            if result.returncode: raise ValueError(f"isolation test failed: {name}; see {logdir / (name + '.log')}")
            print(f"PASS: {name} builds/tests with only its declared dependencies present", flush=True)


CQ = 'struct Q; relation R(a:i32); answer(x) :- R(x);'
RA = CQ + ' relational { r = rename R {a -> x}; output r as answer(x); }'
IX = RA + ' indexes {}'
POLICED_IX = IX + ' policy { R canon; }'
PHYSICAL = IX + ' physical { r = alias rename; return r; }'
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
                'raw = { package = "mini-linq", path = ' + toml(str(ROOT / "crates/mini-linq"))
                + ', default-features = false, features = [' + toml(feature) + '] }\n'
                + ('cq-storage = { path = ' + toml(str(ROOT / "crates/cq-storage")) + ' }\n' if feature == "physical-realization" else '')
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
            expected = {"cq-ir", "mini-linq", *names}
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
