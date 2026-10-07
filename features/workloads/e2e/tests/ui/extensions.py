#!/usr/bin/env python3
"""Invoke extension macros in isolated consumers and pin semantic diagnostics."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

from consumer import WORKSPACE as ROOT, dependencies
CARGO = os.environ.get("CARGO", "cargo")
LOGS = ROOT / "target/extension-ui"
LOGS.mkdir(parents=True, exist_ok=True)
DIRECTORY = Path(tempfile.mkdtemp(prefix="consumer-", dir=LOGS))
env = dict(os.environ, CARGO_TARGET_DIR=str(LOGS / "target"), CARGO_TERM_COLOR="never")
cases = [
    ("union_head", "mini_linq::mini_linq", "struct U; answer(x) :- A(x); other(x) :- B(x);", "same head relation"),
    ("union_arity", "mini_linq::mini_linq", "struct U; answer(x) :- A(x); answer(x,y) :- B(x,y);", "same head relation and arity"),
    ("negative_self", "cq_ext_negation::negation", "struct N; relation A(x:i32); relation answer(x:i32); answer(*x) :- A(x), neg!{answer(x)};", "not the query result"),
    ("negative_dependency", "cq_ext_negation::negation", "struct N; relation A(x:i32); relation S(x:i32,y:i32); answer(x) :- A(x), neg!{S(x,_); depends {S -> T; T -> answer;}};", "unstratified negation"),
    ("negative_scope", "cq_ext_negation::negation", "struct N; relation A(x:i32); relation S(x:i32,y:i32); answer(x) :- A(x), neg!{S(y,_)};", "not bound before"),
    ("aggregate_key", "cq_ext_aggregate::aggregate", "by(missing); n=count(); struct G; relation A(x:i32); rows(x) :- A(x);", "group key is absent"),
    ("aggregate_arity", "cq_ext_aggregate::aggregate", "by(x); n=count(x); struct G; relation A(x:i32); rows(x) :- A(x);", "argument count mismatch"),
    ("recursive_nonlinear", "cq_ext_recursion::recursion", "struct R; relation E(x:i32,y:i32); relation reach(x:i32,y:i32); reach(*x,*y) :- E(x,y); reach(*x,*z) :- reach(x,y),reach(y,z);", "nonlinear recursion"),
    ("recursive_infinite", "cq_ext_recursion::recursion", "struct R; relation E(x:i32,y:i32); relation reach(x:i32,y:i32); reach(*x,*y) :- E(x,y); reach(*x+1,*z) :- reach(x,y),E(y,z);", "active-domain column"),
    ("scalar_shadow", "cq_ext_correlated::correlated", "struct C; relation A(x:i32); answer(x) :- A(x),scalar!{x:i32=(0); select(1); A(y)};", "shadows an outer binding"),
    ("exists_empty", "cq_ext_correlated::correlated", "struct C; relation A(x:i32); answer(x) :- A(x),exists!{if true};", "needs a positive atom"),
    ("unexpanded", "mini_linq::mini_linq", "struct C; relation A(x:i32); answer(x) :- A(x),name!{opaque tokens};", "name!"),
]
manifest = '[package]\nname = "extension-ui"\nversion = "0.0.0"\nedition = "2024"\n[workspace]\n[dependencies]\n'
manifest += dependencies(CARGO, ["mini-linq", "cq-storage", *["cq-ext-" + name for name in ["negation", "aggregate", "recursion", "correlated"]]], env)
for name, macro, source, _ in cases:
    path = DIRECTORY / f"{name}.rs"
    if macro.startswith("cq_ext_"):
        family = macro.split("::")[1]
        authored = f"{family}! {{ {source} }}" if family in {"union", "aggregate", "recursion"} else source
        invocation = f"::mini_linq::compile! {{ pipeline {{ surface = [::{macro}]; }} {authored} }}"
    else:
        invocation = f"::{macro}! {{ {source} }}"
    path.write_text(f"fn main() {{ let _ = {invocation}; }}\n")
    manifest += f'[[bin]]\nname = "{name}"\npath = "{name}.rs"\n'
(DIRECTORY / "Cargo.toml").write_text(manifest)
shutil.copy2(ROOT / "Cargo.lock", DIRECTORY / "Cargo.lock")
subprocess.run([CARGO, "metadata", "--offline", "--format-version=1"], cwd=DIRECTORY, env=env,
               check=True, capture_output=True)
for name, _, _, needle in cases:
    result = subprocess.run([CARGO, "check", "--offline", "--locked", "--bin", name, "--message-format=json"],
                            cwd=DIRECTORY, env=env, capture_output=True, text=True)
    (DIRECTORY / f"{name}.jsonl").write_text(result.stdout)
    errors = [message["message"]["message"] for line in result.stdout.splitlines()
              if (message := json.loads(line)).get("reason") == "compiler-message"
              and message["message"]["level"] == "error"]
    if result.returncode == 0 or not any(needle in error for error in errors):
        raise SystemExit(f"{name}: expected {needle!r}; errors={errors}; {result.stderr}")
print(f"PASS: {len(cases)} extension compile-fail cases; logs: {DIRECTORY}")
