#!/usr/bin/env python3
"""Run the feature's explicit regression target with its required features."""
import os
from pathlib import Path
import subprocess
root = Path(__file__).resolve().parent
raise SystemExit(subprocess.call([os.environ.get("CARGO", "cargo"), "test", "--locked", "--manifest-path", str(root / 'Cargo.toml'), '-p', 'cq-workloads', '--features', 'counters', '--test', 'growth']))
