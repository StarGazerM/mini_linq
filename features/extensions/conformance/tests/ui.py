#!/usr/bin/env python3
"""Compile public consumers, including the migrated extension diagnostic cases."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
CARGO = os.environ.get("CARGO", "cargo")
LOGS = ROOT / "target/extension-ui"
LOGS.mkdir(parents=True, exist_ok=True)
DIRECTORY = Path(tempfile.mkdtemp(prefix="consumer-", dir=LOGS))
env = dict(os.environ, CARGO_TARGET_DIR=str(LOGS / "build"), CARGO_TERM_COLOR="never")


def compile_expression(extension, source):
    if extension == "union":
        return f"mini_linq::mini_linq! {{ {source} }}"
    if extension in {"aggregate", "recursion"}:
        source = f"{extension}! {{ {source} }}"
    return f"mini_linq::compile! {{ pipeline {{ surface = [cq_ext_{extension}::{extension}]; }} {source} }}"


cases = [
    ("union_head", "union", "struct U; answer(x) :- A(x); other(x) :- B(x);", "same head relation"),
    ("union_arity", "union", "struct U; answer(x) :- A(x); answer(x,y) :- B(x,y);", "head relation and arity"),
    ("negative_self", "negation", "struct N; relation A(x:i32); relation answer(x:i32); answer(*x) :- A(x), neg!{answer(x)};", "not the query result"),
    ("negative_dependency", "negation", "struct N; relation A(x:i32); relation S(x:i32,y:i32); answer(x) :- A(x), neg!{S(x,_); depends {S -> T; T -> answer;}};", "unstratified negation"),
    ("negative_scope", "negation", "struct N; relation A(x:i32); relation S(x:i32,y:i32); answer(x) :- A(x), neg!{S(y,_)};", "not bound before"),
    ("aggregate_key", "aggregate", "by(missing); n=count(); struct G; relation A(x:i32); rows(x) :- A(x);", "group key is absent"),
    ("aggregate_arity", "aggregate", "by(x); n=count(x); struct G; relation A(x:i32); rows(x) :- A(x);", "argument count mismatch"),
    ("recursive_nonlinear", "recursion", "struct R; relation E(x:i32,y:i32); relation reach(x:i32,y:i32); reach(*x,*y) :- E(x,y); reach(*x,*z) :- reach(x,y),reach(y,z);", "nonlinear recursion"),
    ("recursive_infinite", "recursion", "struct R; relation E(x:i32,y:i32); relation reach(x:i32,y:i32); reach(*x,*y) :- E(x,y); reach(*x+1,*z) :- reach(x,y),E(y,z);", "active-domain column"),
    ("scalar_shadow", "correlated", "struct C; relation A(x:i32); answer(x) :- A(x),scalar!{x:i32=(0); select(1); A(y)};", "shadows an outer binding"),
    ("exists_empty", "correlated", "struct C; relation A(x:i32); answer(x) :- A(x),exists!{if true};", "needs a positive atom"),
]
expressions = [(name, compile_expression(extension, source), needle) for name, extension, source, needle in cases]
expressions += [
    ("unexpanded", "mini_linq::compile! { pipeline { surface = [cq_ext_negation::negation, cq_ext_aggregate::aggregate, cq_ext_recursion::recursion, cq_ext_correlated::correlated]; } struct C; relation A(x:i32); answer(x) :- A(x), name!{opaque tokens}; }", "name!"),
    ("empty_queue", "cq_ext_negation::negation! { pipeline {}; struct U; answer(x) :- A(x); }", "requires a remaining pipeline"),
    ("surface_arguments", "cq_ext_negation::negation! { arguments { unexpected }; pipeline {mini_linq::passes::plan_written}; struct U; answer(x) :- A(x); }", "accepts no arguments"),
]

# Resolve public package identities from the running workspace. A packaged core
# dependency stays a registry dependency; a local core dependency retains its
# public path. No source files or private test modules are imported.
metadata = json.loads(subprocess.check_output(
    [CARGO, "metadata", "--offline", "--format-version=1"], cwd=ROOT, env=env
))
packages = {package["name"]: package for package in metadata["packages"]}
manifest = '[package]\nname = "extension-ui"\nversion = "0.0.0"\nedition = "2024"\n[workspace]\n[dependencies]\n'
for name in ["cq-ext-negation", "cq-ext-aggregate", "cq-ext-recursion", "cq-ext-correlated", "mini-linq", "cq-storage"]:
    package = packages[name]
    fields = f'version = {json.dumps("=" + package["version"])}'
    if package["source"] is None:
        fields += f', path = {json.dumps(str(Path(package["manifest_path"]).parent))}'
    manifest += f'{name} = {{ {fields} }}\n'
for name, expression, _ in expressions:
    (DIRECTORY / f"{name}.rs").write_text(f"fn main() {{ let _ = {expression}; }}\n")
    manifest += f'[[bin]]\nname = "{name}"\npath = "{name}.rs"\n'
smoke = compile_expression("union", "struct U; relation A(x:i32); answer(x) :- A(x); answer(x) :- A(x);")
(DIRECTORY / "smoke.rs").write_text(f"fn main() {{ let A = [(1_i32,)]; let _: Vec<_> = ({smoke}).collect(); }}\n")
manifest += '[[bin]]\nname = "smoke"\npath = "smoke.rs"\n'
(DIRECTORY / "Cargo.toml").write_text(manifest)
shutil.copy2(ROOT / "Cargo.lock", DIRECTORY / "Cargo.lock")
subprocess.run([CARGO, "metadata", "--offline", "--format-version=1"], cwd=DIRECTORY, env=env, check=True, capture_output=True)
smoke = subprocess.run([CARGO, "check", "--offline", "--locked", "--bin", "smoke"], cwd=DIRECTORY, env=env, capture_output=True, text=True)
if smoke.returncode:
    raise SystemExit(f"positive consumer failed: {smoke.stderr}")
for name, _, needle in expressions:
    result = subprocess.run([CARGO, "check", "--offline", "--locked", "--bin", name, "--message-format=json"], cwd=DIRECTORY, env=env, capture_output=True, text=True)
    (DIRECTORY / f"{name}.jsonl").write_text(result.stdout)
    errors = [message["message"]["message"] for line in result.stdout.splitlines()
              if (message := json.loads(line)).get("reason") == "compiler-message"
              and message["message"]["level"] == "error"]
    if result.returncode == 0 or not any(needle in error for error in errors):
        raise SystemExit(f"{name}: expected {needle!r}; errors={errors}; {result.stderr}")
print(f"PASS: positive consumer and {len(expressions)} extension compile-fail cases; logs: {DIRECTORY}")
