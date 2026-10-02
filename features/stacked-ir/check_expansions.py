#!/usr/bin/env python3
"""Execute feature-owned fixtures and compare every run expansion byte for byte."""
import difflib
import hashlib
import os
from pathlib import Path
import re
import subprocess

BASE = Path(__file__).resolve().parent


def inventory():
    expected = (BASE / "physical_query_cases.txt").read_text().splitlines()
    if not expected or len(expected) != len(set(expected)):
        raise SystemExit("empty or duplicate scenario inventory")
    suites = [(BASE / f"tests/{target}.rs").read_text() for target in ("reference", "stacked")]
    for suite in suites:
        modules = re.findall(r"^mod (\w+) \{", suite, re.M)
        if modules != expected:
            raise SystemExit(f"scenario inventory differs: {modules}")
    tests = [re.findall(r"#\[test\]\s+fn (\w+)", suite) for suite in suites]
    if not tests[0] or tests[0] != tests[1]:
        raise SystemExit("executed test inventory differs")
    # Require identical sources apart from insertion of the Trace chain. This
    # pins the same inputs, oracle assertions and surrounding Rust in both runs.
    unstacked = suites[1].replace(
        "::stacked_ir::enter! { pipeline { ::stacked_ir::leave, ::mini_linq::passes::select,",
        "::mini_linq::passes::select! { pipeline {",
    )
    unstacked = re.sub(r"physical\s*=\s*\[\s*::stacked_ir::round_trip\s*\];", "", unstacked)
    unstacked = re.sub(r",\s*::stacked_ir::round_trip(?=\s*\])", "", unstacked)
    # rustfmt may add a trailing call comma when the inserted pipeline widens
    # a collect invocation. Captured Rust below is still compared as raw bytes.
    def formatted_source(source):
        return re.sub(r"\s+", "", source).replace("},)", "})")

    if formatted_source(suites[0]) != formatted_source(unstacked):
        raise SystemExit("fixture sources differ beyond the Trace chain")
    return expected


def main():
    modules = inventory()
    env = os.environ.copy()
    env.setdefault("CARGO_TARGET_DIR", str(BASE / "target"))
    env["CARGO_TERM_COLOR"] = "never"
    subprocess.run(
        ["cargo", "test", "--locked", "--offline", "-p", "stacked-ir", "--test", "reference", "--test", "stacked"],
        cwd=BASE, env=env, check=True,
    )
    captures = BASE / "target/expansions"
    hashes = {target: [] for target in ("reference", "stacked")}
    for name in modules:
        outputs = []
        for target in ("reference", "stacked"):
            result = subprocess.run(
                ["cargo", "expand", "--locked", "--offline", "--color", "never", "-p", "stacked-ir", "--test", target, name + "::run"],
                cwd=BASE, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            )
            if result.returncode:
                raise SystemExit(result.stderr.decode())
            expanded = result.stdout
            if not expanded.strip():
                raise SystemExit(f"empty expansion: {target}/{name}")
            directory = captures / target
            directory.mkdir(parents=True, exist_ok=True)
            (directory / f"{name}.rs").write_bytes(expanded)
            hashes[target].append(f"{hashlib.sha256(expanded).hexdigest()}  {name}.rs")
            outputs.append(expanded)
        if outputs[0] != outputs[1]:
            diff = "".join(difflib.unified_diff(
                outputs[0].decode().splitlines(keepends=True),
                outputs[1].decode().splitlines(keepends=True),
                fromfile=f"reference/{name}.rs", tofile=f"stacked/{name}.rs",
            ))
            (captures / f"{name}.diff").write_text(diff)
            raise SystemExit(diff)
        print(f"PASS: {name} expansion byte-identical", flush=True)
    for target, lines in hashes.items():
        (captures / target / "SHA256SUMS").write_text("\n".join(lines) + "\n")
    print(f"PASS: {len(modules)} stacked-IR expansions byte-identical; both answer suites passed")


if __name__ == "__main__":
    main()
