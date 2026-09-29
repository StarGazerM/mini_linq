#!/usr/bin/env python3
"""Capture or compare every physical_query module's expanded run function."""
from pathlib import Path
import argparse
import difflib
import hashlib
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path, help="directory for captured expansions")
    parser.add_argument("--compare", type=Path, help="previous capture to compare byte for byte")
    args = parser.parse_args()
    args.directory.mkdir(parents=True, exist_ok=True)
    source = ROOT / "facade/mini-linq/tests/physical_query.rs"
    modules = re.findall(r"^mod (\w+) \{", source.read_text(), re.M)
    if not modules:
        raise SystemExit("no physical_query modules found")
    if args.compare and {p.stem for p in args.compare.glob("*.rs")} != set(modules):
        raise SystemExit("baseline module inventory differs")
    hashes = []
    for name in modules:
        expanded = subprocess.check_output(
            ["cargo", "expand", "-p", "mini-linq", "--test", "physical_query", name + "::run"],
            cwd=ROOT, stderr=subprocess.DEVNULL,
        )
        if not expanded.strip():
            raise SystemExit(f"empty expansion: {name}")
        path = args.directory / (name + ".rs")
        path.write_bytes(expanded)
        if args.compare:
            baseline = (args.compare / path.name).read_bytes()
            if baseline != expanded:
                diff = "".join(difflib.unified_diff(
                    baseline.decode().splitlines(keepends=True),
                    expanded.decode().splitlines(keepends=True),
                    fromfile=str(args.compare / path.name), tofile=str(path),
                ))
                (args.directory / (name + ".diff")).write_text(diff)
                raise SystemExit(diff)
        hashes.append(f"{hashlib.sha256(expanded).hexdigest()}  {path.name}")
    (args.directory / "SHA256SUMS").write_text("\n".join(hashes) + "\n")
    print(f"PASS: {len(modules)} expansions " + ("byte-identical" if args.compare else "captured"))


if __name__ == "__main__":
    main()
