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
from fnmatch import fnmatch
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
TESTKIT = inventory("testkit")
CORE = LANGUAGES | FAMILIES | RUNTIME | FACADES
PATHS = CORE | TESTKIT
# The crate declares the languages its pass transforms, not an arbitrary allowlist.
EDGES = {}
for name, path in FAMILIES.items():
    spec = tomllib.loads((ROOT / path / "Cargo.toml").read_text())["package"]["metadata"]["cq"]
    EDGES[name] = spec


def feature_files(pattern, excluded):
    # Prune ignored build trees before walking them, preserving the same file set.
    for directory, children, files in os.walk(ROOT / "features"):
        children[:] = [name for name in children if name not in excluded]
        for name in files:
            if fnmatch(name, pattern):
                yield Path(directory) / name


# External consumer crates are identified from their standalone manifests.
FEATURE_NAMES = {tomllib.loads(p.read_text())["package"]["name"]
    for p in feature_files("Cargo.toml", {"target", ".git", "vendor"})
    if "package" in tomllib.loads(p.read_text())}

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
    for path in sorted((ROOT / "passes").glob("*/src/**/*.rs")):
        for line in recomputing_checks(path.read_text()):
            raise ValueError(f"{path.relative_to(ROOT)}:{line}: check must not call compile")
    print("PASS: pass check bodies do not call compile", flush=True)


def forbidden_import(source, feature_names):
    masked = mask_rust(source)
    names = "|".join(re.escape(name.replace("-", "_")) for name in feature_names)
    if names and re.search(rf"\b({names})\s*::|\b(?:use|extern\s+crate)\b[^;]*\b({names})\b", masked):
        return "core imports an external feature"
    return None


def check_feature_sources():
    for name, path in CORE.items():
        for source in (ROOT / path / "src").rglob("*.rs"):
            text = source.read_text()
            violation = forbidden_import(text, FEATURE_NAMES)
            if violation or re.search(r"(?:features|extensions|workloads)/", text):
                raise ValueError(f"{source.relative_to(ROOT)}: core names an optional feature")
            if re.search(r"#\[doc\(hidden\)\][\s\S]{0,150}?\bpub\b", text):
                raise ValueError(f"{source.relative_to(ROOT)}: hidden public API in core")
    # pub(crate) is a Rust privacy boundary; forbid consumers naming it explicitly.
    private = []
    for name, path in CORE.items():
        for source in (ROOT / path / "src").rglob("*.rs"):
            module = source.relative_to(ROOT / path / "src").with_suffix("").parts
            qualified = name.replace("-", "_") + "::" + "::".join(x for x in module if x not in {"lib", "mod"})
            private += [(qualified.rstrip(":"), symbol) for symbol in re.findall(
                r"(?m)^pub\(crate\)\s+(?:fn|struct|enum|trait|type|const|mod)\s+(\w+)", mask_rust(source.read_text()))]
    for source in feature_files("*.rs", {"target", ".git"}):
        text = mask_rust(source.read_text())
        for crate, symbol in private:
            if re.search(rf"\b{re.escape(crate)}\s*::\s*{symbol}\b", text):
                raise ValueError(f"{source.relative_to(ROOT)} uses private core item {crate}::{symbol}")
    print("PASS: core has no feature references or hidden public APIs; no consumer uses private core items", flush=True)

def check(data, announce=True):
    packages = {p["name"]: p for p in data["packages"]}
    for name, path in PATHS.items():
        package = packages[name]
        internal = {d["name"] for d in production(package) if "path" in d or d["name"] in packages}
        if name in LANGUAGES:
            base = {"cq-lang-vo": "cq-lang-cq", "cq-lang-ix": "cq-lang-ra", "cq-lang-structures": "cq-lang-physical"}.get(name)
            allowed = ({"cq-syntax", "cq-stage", base} - {None, name})
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
            allowed = ({"cq-syntax"} - {name}) | ({"cq-storage"} if name == "cq-load" else set())
        elif name in TESTKIT:
            allowed = set(LANGUAGES) | set(RUNTIME)
        else:
            allowed = set(packages) - set(TESTKIT)
        for dep in package["dependencies"]:
            target = dep["name"]
            if target in FEATURE_NAMES or (dep.get("path") and Path(dep["path"]).resolve().is_relative_to(ROOT / "features")):
                raise ValueError(f"core depends on an optional feature: {name} -> {target}")
            if dep["kind"] == "dev" and target in CORE and name in CORE:
                continue
            if dep["kind"] == "dev" and target == "core-pass-fixtures":
                continue
            if "path" not in dep and target not in packages: continue
            expected = Path(packages[target]["manifest_path"]).parent if target in packages else None
            permitted = allowed | (set(TESTKIT) if name in CORE and dep["kind"] == "dev" else set())
            if target not in permitted or "path" not in dep or Path(dep["path"]).resolve() != expected:
                raise ValueError(f"forbidden {dep['kind'] or 'normal'} dependency: {name} -> {target}")
        for target in package["targets"]:
            if set(target["kind"]) & {"test", "example", "bench"}: continue
            if not Path(target["src_path"]).resolve().is_relative_to(ROOT / path):
                raise ValueError(f"source escapes its crate: {name}: {target['src_path']}")
    for name in FACADES:
        if packages[name]["features"]:
            raise ValueError(f"facade Cargo features are forbidden: {name}")
    if announce: print("PASS: folder-derived language, pass, runtime, facade and testkit boundaries", flush=True)
    return packages


def negative_dependency_checks(data):
    mutations = [("cq-physical-realization", "cq-storage", None, False),
                 ("cq-physical-realization", "cq-state-providers", "build", False),
                 ("cq-physical-lowering", "cq-lang-ra", None, False),
                 ("cq-lang-ix", "cq-storage", None, False),
                 ("cq-lang-ix", "cq-lang-physical", None, False),
                 ("cq-physical-lowering", "cq-lang-vo", "build", False),
                 ("cq-physical-realization", "cq-storage", None, True)]
    for owner in ("cq-lang-ra", "cq-lang-physical"):
        mutations += [(owner, language, kind, remote)
                      for language in LANGUAGES if language not in {"cq-syntax", owner}
                      for kind in (None, "build") for remote in (False, True)]
    if FEATURE_NAMES:
        feature = sorted(FEATURE_NAMES)[0]
        mutations += [(owner, feature, kind, True)
                      for owner in CORE | TESTKIT for kind in (None, "dev", "build")]
    for testkit in TESTKIT:
        mutations += [(testkit, "cq-order", kind, remote)
                      for kind in (None, "dev", "build") for remote in (False, True)]
        mutations += [(testkit, "mini-linq", kind, False) for kind in (None, "dev", "build")]
        mutations += [(testkit, "cq-stage", "dev", False)]
        mutations += [(owner, testkit, kind, remote)
                      for owner in CORE for kind in (None, "build") for remote in (False, True)]
        mutations += [(owner, testkit, "dev", True) for owner in CORE]
    for owner, target, kind, remote in mutations:
        altered = copy.deepcopy(data)
        package = next(p for p in altered["packages"] if p["name"] == owner)
        dep = {"name": target, "kind": kind, "optional": True, "rename": "innocent_alias", "target": "cfg(unix)"}
        if not remote: dep["path"] = str(ROOT / PATHS[target])
        package["dependencies"].append(dep)
        try: check(altered, announce=False)
        except ValueError: pass
        else: raise ValueError(f"guard accepted forbidden edge: {owner} -> {target}")
    for owner in ("cq-lang-ra", "cq-physical-lowering", "mini-linq", "mini-linq-macros"):
        altered = copy.deepcopy(data)
        next(p for p in altered["packages"] if p["name"] == owner)["features"]["hidden"] = []
        try: check(altered, announce=False)
        except ValueError: pass
        else: raise ValueError(f"guard accepted language/pass feature: {owner}")
    print(f"PASS: {len(mutations)} forbidden dependency and 4 feature mutations rejected", flush=True)
    # Core passes can test against the public corpus.
    for owner in CORE:
        for testkit in TESTKIT:
            altered = copy.deepcopy(data)
            package = next(p for p in altered["packages"] if p["name"] == owner)
            package["dependencies"].append({"name": testkit, "kind": "dev", "optional": False,
                                            "path": str(ROOT / TESTKIT[testkit])})
            check(altered, announce=False)
    if TESTKIT: print("PASS: core testkit dev-dependencies are permitted", flush=True)


def closure(packages, name):
    selected, pending = set(), [name]
    while pending:
        current = pending.pop()
        if current in selected: continue
        selected.add(current)
        # Only the crate under test activates dev-dependencies.
        deps = packages[current]["dependencies"] if current == name else production(packages[current])
        pending.extend(d["name"] for d in deps if d["name"] in PATHS)
    return selected


def write_workspace(destination, selected):
    original = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]
    lines = ["[workspace]", 'resolver = "3"', 'exclude = ["tools/fixtures"]', "members = " + toml([str(PATHS[p]) for p in sorted(selected)]), "", "[workspace.package]"]
    lines += [f"{k} = {toml(v)}" for k, v in original["package"].items()]
    lines += ["", "[workspace.dependencies]"]
    lines += [f"{k} = {toml(v)}" for k, v in original["dependencies"].items()
              if not isinstance(v, dict) or "path" not in v or k in selected]
    (destination / "Cargo.toml").write_text("\n".join(lines) + "\n")
    shutil.copy2(ROOT / "rust-toolchain.toml", destination / "rust-toolchain.toml")


def copy_component(destination, name, selected, tested):
    base = ROOT / PATHS[name]
    output = destination / PATHS[name]
    shutil.copytree(base / "src", output / "src")
    if (base / "tests").exists(): shutil.copytree(base / "tests", output / "tests")
    for filename in ("LICENSE-MIT", "LICENSE-APACHE"):
        if (base / filename).exists(): shutil.copy2(base / filename, output / filename)
    manifest = tomllib.loads((base / "Cargo.toml").read_text())
    tests = manifest.pop("test", [])
    # Only src/tests are copied into an isolated component, not examples.
    manifest.pop("example", None)
    manifest["package"].pop("metadata", None)
    # Match closure(): dependency crates do not activate their own test deps.
    if name != tested: manifest.pop("dev-dependencies", None)
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
    for name in CORE | TESTKIT:
        if only and name not in only: continue
        selected = closure(packages, name) | {"cq-syntax"}
        with tempfile.TemporaryDirectory(prefix=f"mini-linq-{name}-") as temporary:
            destination = Path(temporary)
            write_workspace(destination, selected)
            for package in selected: copy_component(destination, package, selected, name)
            if name not in TESTKIT:
                if "cq-reduce" in selected:
                    shutil.copytree(ROOT / "teaching", destination / "teaching")
                if "mini-linq" in selected:
                    shutil.copytree(ROOT / "tools/fixtures/pass-macros", destination / "tools/fixtures/pass-macros", ignore=shutil.ignore_patterns("target", "Cargo.lock"))
                # Runtime source is a generated-code execution fixture, never a compiler dependency.
                shutil.copytree(ROOT / "runtime/cq-storage/src", destination / "runtime/cq-storage/src", dirs_exist_ok=True)
            env = os.environ.copy(); env["CARGO_TARGET_DIR"] = str(ROOT / "target/component-isolation"); env["CARGO_TERM_COLOR"] = "never"
            command = ["cargo", "test", "-p", name, "--no-default-features", "--offline"]
            with (logdir / f"{name}.log").open("w") as log:
                result = subprocess.run(command, cwd=destination, env=env, stdout=log, stderr=subprocess.STDOUT)
            if result.returncode: raise ValueError(f"isolation failed: {name}; see {logdir / (name + '.log')}")
            if name in FAMILIES | LANGUAGES | TESTKIT:
                if not re.search(r"test result: ok\. [1-9][0-9]* passed", (logdir / f"{name}.log").read_text()):
                    raise ValueError(f"isolation executed no tests: {name}")
            detail = "with only: " + ", ".join(sorted(selected - {name}))
            print(f"PASS: {name} {detail}", flush=True)


def frontends():
    subprocess.run(["cargo", "test", "-p", "mini-linq", "--test", "pipeline", "--locked"], cwd=ROOT, check=True)
    print("PASS: public default/custom pipeline and surface boundaries", flush=True)


def consumers():
    for target in ("physical_query", "wcoj", "denotation", "openings", "provenance", "dispatch", "pass_contracts"):
        subprocess.run(["cargo", "test", "-p", "mini-linq", "--test", target, "--locked"], cwd=ROOT, check=True, stdout=subprocess.DEVNULL)
        print(f"PASS: consumer {target}", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--isolate", action="store_true")
    parser.add_argument("--frontends", action="store_true")
    parser.add_argument("--consumers", action="store_true")
    parser.add_argument("--only", action="append", default=[])
    args = parser.parse_args()
    data = metadata(); packages = check(data)
    negative_dependency_checks(data); check_independent_relations(); check_feature_sources()
    if args.isolate: isolate(packages, args.only)
    if args.frontends: frontends()
    if args.consumers: consumers()


if __name__ == "__main__":
    main()
