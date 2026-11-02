#!/usr/bin/env python3
"""Transparent Cargo timing shim used only by the regression runner.

Cargo's Finished-profile message separates build wall time from execution.
Nested Cargo intervals are unioned by the runner, never double counted. The
original argv, stdout, stderr and exit status are preserved (no extra builds).
"""
import fcntl
import json
import os
import re
import subprocess
import sys
import time

ANSI = re.compile(rb"\x1b\[[0-9;]*m")
FINISHED = re.compile(rb"^\s*Finished\s+.*\bprofile\b")
BUILD_ONLY = {"build", "check", "rustc", "rustdoc", "expand"}
NO_BUILD = {"metadata", "locate-project", "--version", "-V", "--help", "help"}


def main():
    argv = sys.argv[1:]
    command = next((arg for arg in argv if not arg.startswith("+")), "")
    started = time.monotonic()
    boundary = None
    process = subprocess.Popen([os.environ["REGRESSION_REAL_CARGO"], *argv],
                               stderr=subprocess.PIPE)
    for line in iter(process.stderr.readline, b""):
        if boundary is None and FINISHED.match(ANSI.sub(b"", line)):
            boundary = time.monotonic()
        sys.stderr.buffer.write(line)
        sys.stderr.buffer.flush()
    status = process.wait()
    ended = time.monotonic()
    if command not in NO_BUILD:
        if command in BUILD_ONLY or boundary is None:
            boundary = ended
        record = {"start": started, "end": boundary, "cargo_end": ended,
                  "command": command, "status": status,
                  "boundary": "finished" if boundary < ended else "command-end"}
        with open(os.environ["REGRESSION_CARGO_TIMINGS"], "a") as output:
            fcntl.flock(output, fcntl.LOCK_EX)
            output.write(json.dumps(record) + "\n")
    return status if status >= 0 else 128 - status


if __name__ == "__main__":
    raise SystemExit(main())
