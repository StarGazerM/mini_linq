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
EXTENSIONS = inventory("extensions")
TESTKIT = inventory("testkit")
CORE = LANGUAGES | FAMILIES | RUNTIME | FACADES
PATHS = CORE | EXTENSIONS | TESTKIT
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


def mask_rust(source):
    return re.sub(r'//[^\n]*|/\*.*?\*/|r(#+)".*?"\1|"(?:\\.|[^"\\])*"',
                  lambda m: " " * len(m.group()), source, flags=re.S)


def recomputing_checks(source):
    """Inspect balanced check bodies after masking Rust comments and strings."""
    masked = mask_rust(source)
    for match in re.finditer(r"\bpub\s+fn\s+check\s*\(", masked):
        start = masked.index("{", match.end())
        depth, end = 1, start + 1
        while depth and end < len(masked):
            depth += (masked[end] == "{") - (masked[end] == "}")
            end += 1
        if re.search(r"\bcompile\s*\(", masked[start:end]):
            yield source.count("\n", 0, match.start()) + 1


def check_independent_relations():
    assert list(recomputing_checks('pub fn check() { if true { compile(s); } }')) == [1]
    assert not list(recomputing_checks('pub fn check() { let x = "compile("; /* compile(s) */ }\nfn f() { compile(s); }'))
    for path in sorted(list((ROOT / "passes").glob("*/src/**/*.rs")) + list((ROOT / "extensions").glob("*/src/**/*.rs"))):
        for line in recomputing_checks(path.read_text()):
            raise ValueError(f"{path.relative_to(ROOT)}:{line}: check must not call compile")
    print("PASS: pass and extension check bodies do not call compile", flush=True)


def forbidden_import(source, extension_names, surface_only=False):
    masked = mask_rust(source)
    names = "|".join(re.escape(name.replace("-", "_")) for name in extension_names)
    if names and re.search(rf"\b({names})\s*::|\b(?:use|extern\s+crate)\b[^;]*\b({names})\b", masked):
        return "core imports an extension"
    if surface_only:
        aliases = {"cq_lang_cq"} | set(re.findall(r"\bcq_lang_cq\s+as\s+(\w+)", masked))
        for alias in aliases:
            if re.search(rf"\b{alias}\s*::\s*(?:contract|normal|hypergraph)\b|\buse\s+{alias}\s*::\s*\{{[^}}]*\b(?:contract|normal|hypergraph)\b", masked):
                return "extensions may use CQ for surface parsing only"
    return None


def check_extension_sources():
    assert forbidden_import('use cq_ext_union::union;', EXTENSIONS)
    assert forbidden_import('use cq_lang_cq as cq; cq::normal::normalize(s);', (), True)
    assert not forbidden_import('// use cq_ext_union::union;\nconst S: &str = "cq::contract";', EXTENSIONS)
    for name, path in PATHS.items():
        for source in (ROOT / path / "src").rglob("*.rs"):
            violation = forbidden_import(source.read_text(), EXTENSIONS if name in CORE else (), name in EXTENSIONS)
            if violation:
                raise ValueError(f"{source.relative_to(ROOT)}: {violation}")
    print("PASS: core never imports extensions; extensions use only CQ surface parsing", flush=True)


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
        elif name in EXTENSIONS:
            allowed = {"cq-syntax", "cq-lang-cq", "cq-stage", "cq-storage", "cq-state-providers"} | (set(EXTENSIONS) - {name})
        elif name in TESTKIT:
            allowed = set(LANGUAGES) | set(RUNTIME)
        else:
            allowed = set(packages) - set(EXTENSIONS) - set(TESTKIT)
        for dep in package["dependencies"]:
            target = dep["name"]
            if "path" not in dep and target not in packages: continue
            expected = Path(packages[target]["manifest_path"]).parent if target in packages else None
            permitted = allowed | (set(TESTKIT) if name in CORE | EXTENSIONS and dep["kind"] == "dev" else set())
            if target not in permitted or "path" not in dep or Path(dep["path"]).resolve() != expected:
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
    if announce: print("PASS: folder-derived language, pass, runtime, facade, extension and testkit boundaries", flush=True)
    return packages


def negative_dependency_checks(data):
    mutations = [("cq-physical-realization", "cq-storage", None, False),
                 ("cq-physical-realization", "cq-state-providers", "build", False),
                 ("cq-physical-lowering", "cq-lang-ra", None, False),
                 ("cq-lang-ix", "cq-storage", None, False),
                 ("cq-lang-ix", "cq-lang-physical", None, False),
                 ("cq-physical-lowering", "cq-lang-vo", "dev", False),
                 ("cq-physical-realization", "cq-storage", None, True)]
    if EXTENSIONS:
        extension = next(iter(EXTENSIONS))
        mutations += [(extension, "cq-order", None, False),
                      (extension, "cq-lang-ra", "dev", False),
                      (extension, "mini-linq", "build", False),
                      ("mini-linq", extension, "dev", False),
                      ("cq-storage", extension, None, False),
                      ("cq-lang-cq", extension, None, True)]
    for testkit in TESTKIT:
        mutations += [(testkit, "cq-order", kind, remote)
                      for kind in (None, "dev", "build") for remote in (False, True)]
        mutations += [(testkit, "mini-linq", kind, False) for kind in (None, "dev", "build")]
        mutations += [(testkit, "cq-stage", "dev", False)]
        if EXTENSIONS:
            mutations += [(testkit, next(iter(EXTENSIONS)), "dev", False)]
        mutations += [(owner, testkit, kind, remote)
                      for owner in CORE | EXTENSIONS for kind in (None, "build") for remote in (False, True)]
        mutations += [(owner, testkit, "dev", True) for owner in CORE | EXTENSIONS]
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
    # Core and extension passes can both test against the public corpus.
    for owner in CORE | EXTENSIONS:
        for testkit in TESTKIT:
            altered = copy.deepcopy(data)
            package = next(p for p in altered["packages"] if p["name"] == owner)
            package["dependencies"].append({"name": testkit, "kind": "dev", "optional": False,
                                            "path": str(ROOT / TESTKIT[testkit])})
            check(altered, announce=False)
    if TESTKIT: print("PASS: core and extension testkit dev-dependencies are permitted", flush=True)


def closure(packages, name):
    selected, pending = set(), [name]
    while pending:
        current = pending.pop()
        if current in selected: continue
        selected.add(current)
        # Only the crate under test activates dev-dependencies. This includes
        # a public testkit and its language dependencies after caller migration.
        deps = packages[current]["dependencies"] if current == name else production(packages[current])
        pending.extend(d["name"] for d in deps if d["name"] in PATHS)
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
    for filename in ("README.md", "LICENSE-MIT", "LICENSE-APACHE"):
        if (base / filename).exists(): shutil.copy2(base / filename, output / filename)
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
    for name in LANGUAGES | RUNTIME | FAMILIES | EXTENSIONS | TESTKIT:
        if only and name not in only: continue
        selected = closure(packages, name) | {"cq-syntax"}
        with tempfile.TemporaryDirectory(prefix=f"mini-linq-{name}-") as temporary:
            destination = Path(temporary)
            write_workspace(destination, selected)
            for package in selected: copy_component(destination, package, selected)
            if name not in TESTKIT:
                shutil.copytree(ROOT / "test-support", destination / "test-support")
                shutil.copytree(ROOT / "teaching", destination / "teaching")
                # Runtime source is a generated-code execution fixture, never a compiler dependency.
                shutil.copytree(ROOT / "runtime/cq-storage/src", destination / "runtime/cq-storage/src", dirs_exist_ok=True)
            env = os.environ.copy(); env["CARGO_TARGET_DIR"] = str(ROOT / "target/component-isolation"); env["CARGO_TERM_COLOR"] = "never"
            command = ["cargo", "test", "-p", name, "--no-default-features", "--offline"]
            with (logdir / f"{name}.log").open("w") as log:
                result = subprocess.run(command, cwd=destination, env=env, stdout=log, stderr=subprocess.STDOUT)
            if result.returncode: raise ValueError(f"isolation failed: {name}; see {logdir / (name + '.log')}")
            if name in FAMILIES | LANGUAGES | EXTENSIONS | TESTKIT:
                if not re.search(r"test result: ok\. [1-9][0-9]* passed", (logdir / f"{name}.log").read_text()):
                    raise ValueError(f"isolation executed no tests: {name}")
            detail = "with only: " + ", ".join(sorted(selected - {name}))
            print(f"PASS: {name} {detail}", flush=True)


CQ = 'struct Q; relation R(a:i32); answer(x) :- R(x);'
RA = 'relational { relation R(a:i32); r = rename R {a -> x}; output r as answer(x); }'
IX = RA + ' indexes {}'
POLICED_IX = IX + ' policy {}'
PHYSICAL = 'physical { relation R(a:i32); r = pipe canon rename R {a -> x}; return r as answer(x); }'
POLICED_PHYSICAL = PHYSICAL + ' policy {}'
STRUCTURES = PHYSICAL + ' structures {}'


STATE_BINDINGS = r"""
macro_rules! distinct_state {
    (operator $id:ident; new) => { ::std::collections::HashSet::new() };
    (operator $id:ident; insert($state:expr, $row:expr)) => { $state.insert(Clone::clone($row)) };
}
#[allow(unused_imports)]
pub(crate) use distinct_state;
#[allow(unused_imports)]
macro_rules! indexes {
    (probe) => { ::cq_storage::HashIndex<'_, _, _> };
    (semi) => { ::std::collections::HashSet<_> };
    (ordered) => { ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> };
    (trie $inner:ty) => { ::cq_storage::Trie<'_, _, $inner> };
}
#[allow(unused_imports)]
pub(crate) use indexes as index_provider;
#[allow(unused_imports)]
mod wiring { pub(crate) use super::{distinct_state,index_provider}; }
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
            "physical-lowering": (POLICED_IX, "select_structures", ["physical", "pipe", "policy"]),
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
    let rows: Vec<_> = raw::{implementation}! {{ {STRUCTURES} policy {{}} }}.collect();
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
    check_independent_relations()
    check_extension_sources()
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
