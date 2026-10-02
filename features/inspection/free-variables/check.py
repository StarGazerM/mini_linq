#!/usr/bin/env python3
"""Test and run the standalone public free-variable inspection example."""
from pathlib import Path
import subprocess

workspace = Path(__file__).resolve().parents[1]
for command in [
    ["cargo", "test", "--locked", "--example", "free-variables"],
    ["cargo", "run", "--locked", "--example", "free-variables"],
]:
    subprocess.run(command, cwd=workspace, check=True)
