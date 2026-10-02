#!/usr/bin/env python3
"""Test and run the standalone CQ inspection example."""
from pathlib import Path
import subprocess

workspace = Path(__file__).resolve().parents[1]
for command in [
    ["cargo", "test", "--locked", "--example", "cq-inspection"],
    ["cargo", "run", "--locked", "--example", "cq-inspection"],
]:
    subprocess.run(command, cwd=workspace, check=True)
