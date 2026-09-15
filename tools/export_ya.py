#!/usr/bin/env python3
"""Extract the selected YA source closure; optionally verify both exercise cuts."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

try:
    import tomllib
except ModuleNotFoundError:  # Python 3.10 staff environments.
    import tomli as tomllib

ROOT = Path(__file__).resolve().parents[1]
PROFILE = ROOT / "teaching/ya"


def local_path(text: str) -> Path:
    path = Path(text)
    if path.is_absolute() or ".." in path.parts:
        raise ValueError(f"profile path must stay within its root: {text}")
    return path


def toml_value(value: object) -> str:
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, str):
        return json.dumps(value)
    if isinstance(value, list):
        return "[" + ", ".join(map(toml_value, value)) + "]"
    if isinstance(value, dict):
        return (
            "{ "
            + ", ".join(f"{key} = {toml_value(item)}" for key, item in value.items())
            + " }"
        )
    raise ValueError(f"unsupported manifest value: {value!r}")


def run(
    command: list[str], directory: Path, env: dict[str, str], missing: str | None = None
) -> str:
    result = subprocess.run(
        command,
        cwd=directory,
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    accepted = (
        result.returncode == 0
        if missing is None
        else result.returncode != 0 and missing in result.stdout
    )
    if not accepted:
        print(result.stdout, end="")
        raise RuntimeError(
            f"unexpected result ({result.returncode}): {' '.join(command)}"
        )
    count = sum(map(int, re.findall(r"test result: ok\. (\d+) passed", result.stdout)))
    tested = f" ({count} tests)" if count else ""
    print(
        f"{'EXPECTED EXERCISE FAILURE' if missing else 'PASS'}: {' '.join(command)}{tested}",
        flush=True,
    )
    return result.stdout


def manifests(profile: dict) -> dict[str, bytes]:
    original = tomllib.loads((ROOT / "Cargo.toml").read_text())
    dependencies = original["workspace"]["dependencies"]
    selected = set(profile["packages"])
    selected.update(dep for deps in profile["packages"].values() for dep in deps)
    lines = [
        "[workspace]",
        'resolver = "3"',
        "members = " + toml_value([f"crates/{name}" for name in profile["packages"]]),
        "",
        "[workspace.package]",
    ]
    lines.extend(
        f"{key} = {toml_value(value)}"
        for key, value in original["workspace"]["package"].items()
    )
    lines.extend(["", "[workspace.dependencies]"])
    for name in sorted(selected):
        value = dependencies[name]
        if isinstance(value, dict) and "path" in value:
            if name not in profile["packages"] or value["path"] != f"crates/{name}":
                raise ValueError(f"unselected workspace dependency: {name}")
        lines.append(f"{name} = {toml_value(value)}")
    output = {"Cargo.toml": ("\n".join(lines) + "\n").encode()}
    for name, deps in profile["packages"].items():
        lines = [
            "[package]",
            f"name = {toml_value(name)}",
            "version.workspace = true",
            "edition.workspace = true",
            "rust-version.workspace = true",
            "license.workspace = true",
            "publish = false",
            "",
            "[dependencies]",
        ]
        lines.extend(f"{dep}.workspace = true" for dep in deps)
        if name == "mini-linq":
            lines.extend(
                [
                    "",
                    "[lib]",
                    "proc-macro = true",
                    "",
                    "[[example]]",
                    'name = "ya-chain"',
                    'path = "examples/ya_chain.rs"',
                    "",
                    "[[example]]",
                    'name = "semijoin"',
                    'path = "examples/semijoin.rs"',
                ]
            )
        output[f"crates/{name}/Cargo.toml"] = ("\n".join(lines) + "\n").encode()
    return output


def verify(destination: Path, profile: dict, env: dict[str, str]) -> None:
    stubs = {path: (destination / path).read_bytes() for path in profile["exercises"]}
    run(["cargo", "check", "--workspace", "--locked"], destination, env)
    run(["cargo", "test", "-p", "cq-ir", "--locked"], destination, env)
    for exercise in profile["exercises"].values():
        run(exercise["command"], destination, env, exercise["missing"])
    try:
        for path in stubs:
            (destination / path).write_bytes((ROOT / path).read_bytes())
        run(
            ["cargo", "test", "--workspace", "--all-targets", "--locked"],
            destination,
            env,
        )
        for example in ["ya-chain", "semijoin"]:
            run(
                ["cargo", "run", "-p", "mini-linq", "--example", example, "--locked"],
                destination,
                env,
            )
    finally:
        for path, content in stubs.items():
            (destination / path).write_bytes(content)
    assert all(
        (destination / path).read_bytes() == content for path, content in stubs.items()
    )
    print(
        "PASS: both student stubs restored; reference files and build artifacts remain outside the export",
        flush=True,
    )


def export(destination: Path, should_verify: bool) -> None:
    if destination.exists():
        raise ValueError(f"destination already exists: {destination}")
    if destination == ROOT or ROOT.is_relative_to(destination):
        raise ValueError("destination must not contain the source checkout")
    profile = json.loads((PROFILE / "profile.json").read_text())
    files: dict[str, tuple[bytes, str]] = {}

    def add(target: str, content: bytes, origin: str) -> None:
        local_path(target)
        if target in files:
            raise ValueError(f"duplicate exported path: {target}")
        files[target] = (content, origin)

    for name in profile["copy"]:
        source = ROOT / local_path(name)
        if source.is_symlink():
            raise ValueError(f"source symlink is not permitted: {name}")
        add(name, source.read_bytes(), name)
    for target, name in profile["templates"].items():
        source = PROFILE / "templates" / local_path(name)
        add(target, source.read_bytes(), f"teaching/ya/templates/{name}")
    for target, content in manifests(profile).items():
        add(target, content, "selected workspace dependencies")
    add(".gitignore", b"/target/\n", "release build exclusion")
    for exercise in profile["exercises"]:
        if exercise not in profile["templates"] or exercise in profile["copy"]:
            raise ValueError(
                f"reference implementation would leak into export: {exercise}"
            )
    destination.mkdir(parents=True)
    for name, (content, _) in files.items():
        target = destination / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(content)
    # Build output is external even during verification with the reference
    # overlay. The final starter must not contain compiled private answers.
    with tempfile.TemporaryDirectory(prefix="mini-linq-ya-build-") as temporary:
        env = os.environ.copy()
        env["CARGO_TARGET_DIR"] = str(Path(temporary) / "target")
        env["CARGO_TERM_COLOR"] = "never"
        run(["cargo", "generate-lockfile", "--offline"], destination, env)
        files["Cargo.lock"] = (
            (destination / "Cargo.lock").read_bytes(),
            "selected Cargo dependency resolution",
        )
        if should_verify:
            verify(destination, profile, env)
    inventory = {
        "profile": "YA graph planning and materialized hash access",
        "exercises": list(profile["exercises"]),
        "verified": should_verify,
        "files": {
            name: {"origin": origin, "sha256": hashlib.sha256(content).hexdigest()}
            for name, (content, origin) in sorted(files.items())
        },
    }
    (destination / "EXPORT.json").write_text(json.dumps(inventory, indent=2) + "\n")
    actual = {
        str(path.relative_to(destination))
        for path in destination.rglob("*")
        if path.is_file()
    }
    if actual != set(files) | {"EXPORT.json"}:
        raise RuntimeError("export acquired files outside its explicit inventory")
    for name, (content, _) in files.items():
        if (destination / name).read_bytes() != content:
            raise RuntimeError(f"verification left a changed source file: {name}")
    print(f"Exported {len(files)} files to {destination}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destination", type=Path, help="new destination directory")
    parser.add_argument(
        "--verify",
        action="store_true",
        help="verify foundation, missing exercises, and reference closure",
    )
    args = parser.parse_args()
    try:
        export(args.destination.resolve(), args.verify)
    except (OSError, ValueError, RuntimeError) as error:
        parser.exit(1, f"error: {error}\n")
