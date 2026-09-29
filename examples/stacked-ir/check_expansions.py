#!/usr/bin/env python3
"""Execute the reference and stacked fixtures, then compare all ten expansions."""
from pathlib import Path
import os
import re
import subprocess
base = Path(__file__).resolve().parent
root = base.parents[1]
env = os.environ.copy()
env["CARGO_TARGET_DIR"] = str(root / "target/scenario-examples")
subprocess.run(["cargo", "test", "--manifest-path", str(base / "Cargo.toml"), "--locked"], env=env, check=True)
mods = re.findall(r"^mod (\w+) \{", (root / "facade/mini-linq/tests/physical_query.rs").read_text(), re.M)
for name in mods:
    outputs = []
    for target in ["reference", "stacked"]:
        outputs.append(subprocess.check_output(["cargo", "expand", "--manifest-path", str(base / "Cargo.toml"), "--test", target, name + "::run"], env=env, stderr=subprocess.DEVNULL))
    assert outputs[0] == outputs[1], name + ": expansion changed"
print(f"PASS: {len(mods)} stacked-IR expansions byte-identical")
