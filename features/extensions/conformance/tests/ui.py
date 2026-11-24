#!/usr/bin/env python3
"""Compile public consumers and pin negative cases to their rejecting source spans."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

PACKAGE = Path(__file__).resolve().parents[1]
CARGO = os.environ.get("CARGO", "cargo")
env = dict(os.environ, CARGO_TERM_COLOR="never")
metadata = json.loads(subprocess.check_output(
    [CARGO, "metadata", "--offline", "--format-version=1", "--manifest-path", str(PACKAGE / "Cargo.toml")],
    env=env,
))
# Generated Cargo invocations must inherit the workspace's source configuration,
# including packaged sources when the build directory is overridden externally.
LOGS = Path(metadata["workspace_root"]) / "target/extension-ui"
LOGS.mkdir(parents=True, exist_ok=True)
DIRECTORY = Path(tempfile.mkdtemp(prefix="consumer-", dir=LOGS))
env["CARGO_TARGET_DIR"] = str(LOGS / "build")


def compile_expression(extension, source):
    if extension == "union":
        return f"mini_linq::mini_linq! {{ {source} }}"
    if extension in {"aggregate", "recursion"}:
        source = f"{extension}! {{ {source} }}"
    return f"mini_linq::compile! {{ pipeline {{ surface = [cq_ext_{extension}::{extension}]; }} {source} }}"


cases = [
    ("union_head", "union", "struct U; answer(x) :- A(x); other(x) :- B(x);"),
    ("union_arity", "union", "struct U; answer(x) :- A(x); answer(x,y) :- B(x,y);"),
    ("aggregate_key", "aggregate", "by(missing); n=count(); struct G; relation A(x:i32); rows(x) :- A(x);"),
    ("aggregate_arity", "aggregate", "by(x); n=count(x); struct G; relation A(x:i32); rows(x) :- A(x);"),
    ("recursive_nonlinear", "recursion", "struct R; relation E(x:i32,y:i32); relation reach(x:i32,y:i32); reach(x,y) :- E(x,y); reach(x,z) :- reach(x,y),reach(y,z);"),
    ("recursive_infinite", "recursion", "struct R; relation E(x:i32,y:i32); relation reach(x:i32,y:i32); reach(x,y) :- E(x,y); reach(*x+1,z) :- reach(x,y),E(y,z);"),
    ("recursive_copy_syntax", "recursion", "struct R; relation E(x:i32,y:i32); relation reach(x:i32,y:i32); reach(x,y) :- E(x,y); reach(*x,z) :- reach(x,y),E(y,z);"),
    ("scalar_shadow", "correlated", "struct C; relation A(x:i32); answer(x) :- A(x),scalar!{x:i32=(0); select(1); A(y)};"),
    ("exists_empty", "correlated", "struct C; relation A(x:i32); answer(x) :- A(x),exists!{if true};"),
]
expressions = [(name, compile_expression(extension, source)) for name, extension, source in cases]
expressions += [
    ("unexpanded", "mini_linq::compile! { pipeline { surface = [cq_ext_aggregate::aggregate, cq_ext_recursion::recursion, cq_ext_correlated::correlated]; } struct C; relation A(x:i32); answer(x) :- A(x), name!{opaque tokens}; }"),
    ("empty_queue", "cq_ext_correlated::correlated! { pipeline {}; struct U; answer(x) :- A(x); }"),
    ("surface_arguments", "cq_ext_correlated::correlated! { arguments { unexpected }; pipeline {mini_linq::passes::plan}; struct U; answer(x) :- A(x); }"),
]

# compile_error! diagnostics have no rustc error code. Identify each rejection
# by its primary source span instead of accepting an unrelated downstream error
# or pinning production prose. The marker is unique in the authored expression;
# the width is the rejected token/construct, not the enclosing macro invocation.
rejection_spans = {
    "union_head": ("other(x)", len("other(x)")),
    "union_arity": ("answer(x,y)", len("answer(x,y)")),
    "aggregate_key": ("missing", len("missing")),
    "aggregate_arity": ("count(x)", len("count")),
    "recursive_nonlinear": (
        "reach(x,z) :- reach(x,y),reach(y,z);",
        len("reach(x,z) :- reach(x,y),reach(y,z);"),
    ),
    "recursive_infinite": ("*x+1", len("*x+1")),
    "recursive_copy_syntax": ("*x", len("*x")),
    "scalar_shadow": ("x:i32=(0)", len("x")),
    "exists_empty": (
        "struct C; relation A(x:i32); answer(x) :- A(x),exists!{if true};",
        len("struct C; relation A(x:i32); answer(x) :- A(x),exists!{if true};"),
    ),
    "unexpanded": ("name!{opaque tokens}", len("name")),
    "empty_queue": (
        "struct U; answer(x) :- A(x);",
        len("struct U; answer(x) :- A(x);"),
    ),
    "surface_arguments": ("unexpected", len("unexpected")),
}
assert set(rejection_spans) == {name for name, _ in expressions}
expected_spans = {}

# Resolve public package identities from the running workspace. A packaged core
# dependency stays a registry dependency; a local core dependency retains its
# public path. No source files or private test modules are imported.
packages = {package["name"]: package for package in metadata["packages"]}
manifest = '[package]\nname = "extension-ui"\nversion = "0.0.0"\nedition = "2024"\n[workspace]\n[dependencies]\n'
for name in ["cq-ext-aggregate", "cq-ext-recursion", "cq-ext-correlated", "mini-linq", "cq-storage"]:
    package = packages[name]
    fields = f'version = {json.dumps("=" + package["version"])}'
    if package["source"] is None:
        fields += f', path = {json.dumps(str(Path(package["manifest_path"]).parent))}'
    manifest += f'{name} = {{ {fields} }}\n'
for name, expression in expressions:
    fixture = f"fn main() {{ let _ = {expression}; }}\n"
    marker, width = rejection_spans[name]
    assert fixture.count(marker) == 1, (name, marker)
    start = len(fixture[:fixture.index(marker)].encode("utf-8"))
    expected_spans[name] = (start, start + width)
    (DIRECTORY / f"{name}.rs").write_text(fixture)
    manifest += f'[[bin]]\nname = "{name}"\npath = "{name}.rs"\n'
smoke = compile_expression("union", "struct U; relation A(x:i32); answer(x) :- A(x); answer(x) :- A(x);")
(DIRECTORY / "smoke.rs").write_text(f"fn main() {{ let A = [(1_i32,)]; let _: Vec<_> = ({smoke}).collect(); }}\n")
manifest += '[[bin]]\nname = "smoke"\npath = "smoke.rs"\n'
(DIRECTORY / "Cargo.toml").write_text(manifest)
shutil.copy2(Path(metadata["workspace_root"]) / "Cargo.lock", DIRECTORY / "Cargo.lock")
subprocess.run([CARGO, "metadata", "--offline", "--format-version=1"], cwd=DIRECTORY, env=env, check=True, capture_output=True)
smoke = subprocess.run([CARGO, "check", "--offline", "--locked", "--bin", "smoke"], cwd=DIRECTORY, env=env, capture_output=True, text=True)
if smoke.returncode:
    raise SystemExit(f"positive consumer failed: {smoke.stderr}")
for name, _ in expressions:
    result = subprocess.run([CARGO, "check", "--offline", "--locked", "--bin", name, "--message-format=json"], cwd=DIRECTORY, env=env, capture_output=True, text=True)
    (DIRECTORY / f"{name}.jsonl").write_text(result.stdout)
    errors = [message["message"] for line in result.stdout.splitlines()
              if (message := json.loads(line)).get("reason") == "compiler-message"
              and message["message"]["level"] == "error"]
    start, end = expected_spans[name]
    matched = any(
        error["code"] is None
        and any(
            span["is_primary"]
            and (DIRECTORY / span["file_name"]).resolve() == (DIRECTORY / f"{name}.rs").resolve()
            and span["byte_start"] == start
            # Macro forwarding may retain only the first token's span. It must
            # still start at, and stay inside, the exact rejected construct.
            and start < span["byte_end"] <= end
            for span in error["spans"]
        )
        for error in errors
    )
    if result.returncode == 0 or not matched:
        raise SystemExit(
            f"{name}: expected an uncoded primary diagnostic at bytes "
            f"{start}..{end}; errors={errors}; {result.stderr}"
        )
print(f"PASS: positive consumer and {len(expressions)} extension compile-fail cases; logs: {DIRECTORY}")
