#!/usr/bin/env python3
"""Package core and test ready feature workspaces outside the checkout.

The local directory source contains Cargo-produced .crate archives plus locked
third-party vendors. No core path dependency is available to feature builds.
PENDING lists paths relative to features/. The fixture always exercises the gate.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib

ROOT = Path(__file__).resolve().parents[1]


def run(command, cwd, log, env=None):
    with log.open("a") as output:
        result = subprocess.run(command, cwd=cwd, env=env, stdout=output, stderr=subprocess.STDOUT)
    if result.returncode:
        raise RuntimeError(f"{' '.join(command)} failed; {log}\n" + "\n".join(log.read_text().splitlines()[-35:]))


def toml(value):
    if isinstance(value, str): return json.dumps(value)
    if isinstance(value, bool): return str(value).lower()
    if isinstance(value, dict): return "{ " + ", ".join(f"{json.dumps(k)} = {toml(v)}" for k, v in value.items()) + " }"
    if isinstance(value, list): return "[" + ", ".join(map(toml, value)) + "]"
    return str(value)


def write_manifest(path, spec):
    lines = []
    def table(prefix, values):
        scalars = {k:v for k,v in values.items() if not isinstance(v, dict) and not (isinstance(v,list) and v and isinstance(v[0],dict))}
        if prefix: lines.append("[" + ".".join(json.dumps(k) for k in prefix) + "]")
        lines.extend(f"{json.dumps(k)} = {toml(v)}" for k,v in scalars.items())
        lines.append("")
        for k,v in values.items():
            if isinstance(v,dict): table(prefix+[k],v)
            elif isinstance(v,list) and v and isinstance(v[0],dict):
                for entry in v:
                    lines.append("[[" + ".".join(json.dumps(x) for x in prefix+[k]) + "]]")
                    lines.extend(f"{json.dumps(a)} = {toml(b)}" for a,b in entry.items()); lines.append("")
    table([],spec);path.write_text("\n".join(lines))


def manifests(workspace):
    return sorted(p for p in workspace.rglob("Cargo.toml") if not any(part in {"target", ".git", "vendor"} for part in p.relative_to(workspace).parts))


def point_core(workspace, names, paths=None):
    for manifest in manifests(workspace):
        spec = tomllib.loads(manifest.read_text())
        def rewrite(values):
            for section in ("dependencies", "dev-dependencies", "build-dependencies"):
                for alias, value in values.get(section, {}).items():
                    package = value.get("package", alias) if isinstance(value,dict) else alias
                    if package in names:
                        old = value if isinstance(value,dict) else {}
                        dep = {k:v for k,v in old.items() if k not in {"path", "git", "branch", "rev", "tag", "registry", "workspace"}}
                        dep["version"] = "=" + names[package]
                        if paths is not None: dep["path"] = str(paths[package])
                        values[section][alias] = dep
            for target in values.get("target",{}).values(): rewrite(target)
        rewrite(spec)
        if "workspace" in spec: rewrite(spec["workspace"])
        # Root-checkout patches must not make an external feature pass.
        for registry, patches in spec.get("patch",{}).items():
            for package in list(patches):
                if package in names: del patches[package]
        write_manifest(manifest,spec)


def package_core(directory, log):
    data = json.loads(subprocess.check_output(["cargo","metadata","--no-deps","--format-version=1","--locked"],cwd=ROOT))
    packages = [p for p in data["packages"] if Path(p["manifest_path"]).parent.parent.name in {"languages","passes","runtime","facade","testkit"}]
    names = {p["name"]:p["version"] for p in packages}
    source = directory/"source"; source.mkdir()
    # Pin the derive fork too. cargo package normalizes git dependencies to a
    # version; our directory source preserves the actual locked implementation.
    vendor = directory/"vendor"
    run(["cargo","vendor","--locked","--offline",str(vendor)], ROOT, log)
    original = tomllib.loads((ROOT/"Cargo.toml").read_text())["workspace"]
    root = {"workspace":{"members":[],"resolver":"3","package":original["package"],"dependencies":original["dependencies"]}, "patch":{"crates-io":{}}}
    for p in packages:
        relative = Path(p["manifest_path"]).parent.relative_to(ROOT);root["workspace"]["members"].append(str(relative))
        destination=source/relative;shutil.copytree(Path(p["manifest_path"]).parent,destination,ignore=shutil.ignore_patterns("target",".git"))
        manifest=tomllib.loads((destination/"Cargo.toml").read_text())
        manifest.pop("dev-dependencies",None);manifest.pop("test",None);manifest.pop("example",None);manifest.pop("bench",None)
        manifest["package"]["autotests"]=False;manifest["package"]["autoexamples"]=False;manifest["package"]["autobenches"]=False
        write_manifest(destination/"Cargo.toml",manifest)
        root["patch"]["crates-io"][p["name"]]={"path":str(relative)}
    for name, dep in root["workspace"]["dependencies"].items():
        if isinstance(dep,dict) and name in names: dep["version"]=names[name]
        if isinstance(dep,dict) and "git" in dep:
            candidates=[p for p in vendor.iterdir() if p.is_dir() and tomllib.loads((p/"Cargo.toml").read_text())["package"]["name"]==name]
            if len(candidates)!=1: raise RuntimeError(f"missing pinned vendor: {name}")
            dep.clear();dep.update({"path":str(candidates[0]),"version":tomllib.loads((candidates[0]/"Cargo.toml").read_text())["package"]["version"]})
            root["patch"]["crates-io"][name]={"path":str(candidates[0])}
    write_manifest(source/"Cargo.toml",root)
    env=os.environ.copy();env.setdefault("RUSTUP_TOOLCHAIN", tomllib.loads((ROOT/"rust-toolchain.toml").read_text())["toolchain"]["channel"]);env["CARGO_TARGET_DIR"]=str(directory/"package-target")
    # This is actual Cargo packaging, never a source-tree copy presented as a package.
    run(["cargo","package","--workspace","--allow-dirty","--no-verify","--offline"],source,log,env)
    archives=list((directory/"package-target/package").glob("*.crate"))
    found=set()
    for archive in archives:
        with tarfile.open(archive) as tar: tar.extractall(vendor)
        package=vendor/archive.name.removesuffix(".crate")
        spec=tomllib.loads((package/"Cargo.toml").read_text());name=spec["package"]["name"]
        if name not in names: continue
        found.add(name)
        files={str(p.relative_to(package)):hashlib.sha256(p.read_bytes()).hexdigest() for p in package.rglob("*") if p.is_file() and p.name!=".cargo-checksum.json"}
        (package/".cargo-checksum.json").write_text(json.dumps({"files":files,"package":hashlib.sha256(archive.read_bytes()).hexdigest()}))
    if found!=set(names): raise RuntimeError(f"package inventory differs: {set(names)-found}")
    print(f"PASS: cargo packaged {len(found)} core crates",flush=True)
    return names,vendor


def feature_workspaces():
    pending={line.strip() for line in (ROOT/"features/PENDING").read_text().splitlines() if line.strip() and not line.lstrip().startswith("#")}
    ready=[]
    for manifest in manifests(ROOT/"features"):
        directory=manifest.parent;relative=directory.relative_to(ROOT/"features")
        if any(relative==Path(p) or Path(p) in relative.parents for p in pending):continue
        spec=tomllib.loads(manifest.read_text())
        if "workspace" in spec:ready.append(directory)
        elif not any(parent in ready for parent in directory.parents):
            raise RuntimeError(f"ready feature has no standalone workspace: {relative}")
    return ready


def test_feature(feature, directory, names, vendor, log):
    destination=directory/"feature";shutil.copytree(feature,destination,ignore=shutil.ignore_patterns("target",".git","Cargo.lock"))
    point_core(destination,names)
    (destination/".cargo").mkdir(exist_ok=True)
    (destination/".cargo/config.toml").write_text('[source.crates-io]\nreplace-with = "packaged"\n[source.packaged]\ndirectory = '+json.dumps(str(vendor))+'\n')
    env=os.environ.copy();env.setdefault("RUSTUP_TOOLCHAIN", tomllib.loads((ROOT/"rust-toolchain.toml").read_text())["toolchain"]["channel"]);env["CARGO_TARGET_DIR"]=str(ROOT/"target/external");env["CARGO_TERM_COLOR"]="never"
    # Metadata must resolve before build and every local crate must be in the
    # copied workspace; a leftover absolute/in-tree dependency is a gate failure.
    with log.open("a") as err:
        output=subprocess.check_output(["cargo","metadata","--format-version=1","--offline"],cwd=destination,env=env,stderr=err)
    data=json.loads(output)
    for package in data["packages"]:
        manifest=Path(package["manifest_path"]).resolve()
        if manifest.is_relative_to(ROOT):raise RuntimeError(f"feature escaped to in-tree crate: {manifest}")
        if package["source"] is None and not manifest.is_relative_to(destination):raise RuntimeError(f"feature escaped copied workspace: {manifest}")
    run(["cargo","build","--workspace","--all-targets","--offline","--locked"],destination,log,env)
    run(["cargo","test","--workspace","--offline","--locked"],destination,log,env)
    print(f"PASS: outside-repo build + test: {feature.relative_to(ROOT)}",flush=True)


def main():
    argparse.ArgumentParser(description=__doc__).parse_args()
    logdir=ROOT/"target/external-logs";logdir.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="mini-linq-packaged-") as tmp:
        directory=Path(tmp);names,vendor=package_core(directory,logdir/"package.log")
        features=[ROOT/"tools/fixtures/external"]+feature_workspaces()
        for i,feature in enumerate(features):
            target=directory/f"consumer-{i}";target.mkdir();test_feature(feature,target,names,vendor,logdir/f"consumer-{i}.log")
        negative_env=os.environ.copy();negative_env["CARGO_TARGET_DIR"]=str(ROOT/"target/external-negative")
        run(["cargo","build","--manifest-path",str(ROOT/"tools/fixtures/in-tree-only/Cargo.toml"),"--offline"],ROOT,logdir/"negative-in-tree.log",negative_env)
        target=directory/"negative";target.mkdir()
        try: test_feature(ROOT/"tools/fixtures/in-tree-only",target,names,vendor,logdir/"negative.log")
        except (RuntimeError, subprocess.CalledProcessError) as error:
            if "couldn't read" not in (logdir/"negative.log").read_text(): raise RuntimeError("negative fixture failed for the wrong reason") from error
            print("PASS: external gate rejects a feature that only builds in-tree",flush=True)
        else: raise RuntimeError("gate accepted the in-tree-only fixture")
    print(f"PASS: packaged public API; {len(features)-1} ready features and mandatory fixture",flush=True)


if __name__=="__main__":main()
