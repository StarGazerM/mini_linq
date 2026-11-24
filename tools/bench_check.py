"""Transport for the shared native timing checker; no tuple-checking algorithm here."""
from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess


def env(plan):
    """Environment for a timing producer using bench_check::Policy::from_env."""
    return {
        "BENCH_CHECK_MODE": plan["mode"],
        "BENCH_CHECK_SEED": str(plan["seed"]),
        "BENCH_CHECK_SIZE": str(plan["sample_size"]),
        "BENCH_CHECK_SAMPLE": str(plan["sample"]),
    }


def _binary():
    configured = os.environ.get("BENCH_CHECK_BIN")
    if configured:
        return configured
    candidates = []
    if os.environ.get("BUNDLE_DIR"):
        candidates.append(Path(os.environ["BUNDLE_DIR"]) / "bin" / "bench-check")
    root = Path(__file__).resolve().parent.parent
    candidates.extend((root / "target" / "release" / "bench-check",
                       root / "target" / "debug" / "bench-check"))
    for path in candidates:
        if path.is_file() and os.access(path, os.X_OK):
            return str(path)
    raise FileNotFoundError("build bench-check or set BENCH_CHECK_BIN to its executable")


class Checker:
    """One controller process, one reference index per immutable reference ID.

    A key identifies binary content, dataset/query and execution configuration.
    References and first-full state stay alive across fresh worker processes.
    All comparisons and file tuple parsing are performed by the native helper.
    """

    def __init__(self, binary=None, seed=20261103, sample_size=1000):
        self._process = subprocess.Popen(
            [os.fspath(binary) if binary is not None else _binary()],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1,
        )
        try:
            self._request("configure", seed=seed, sample_size=sample_size)
        except BaseException:
            self.close()
            raise

    def _request(self, op, **arguments):
        process = self._process
        if process is None:
            raise RuntimeError("checker is closed")
        process.stdin.write(json.dumps({"op": op, **arguments}, separators=(",", ":")) + "\n")
        process.stdin.flush()
        line = process.stdout.readline()
        if not line:
            raise RuntimeError("native benchmark checker exited without a response")
        response = json.loads(line)
        if response.get("ok") is not True:
            raise RuntimeError(response.get("error", "native benchmark check failed"))
        return response["result"]

    def register(self, reference_id, relations):
        return self._request("register", reference_id=reference_id, relations=relations)

    def register_files(self, reference_id, files, format="tsv"):
        return self._request("register_files", reference_id=reference_id,
                             files={name: os.fspath(path) for name, path in files.items()},
                             format=format)

    def register_witness(self, reference_id, path):
        return self._request("register_witness", reference_id=reference_id, path=os.fspath(path))

    def plan(self, key, reference_id, ordinal, seed=None):
        options = {} if seed is None else {"seed": seed}
        return self._request("plan", key=key, reference_id=reference_id, ordinal=ordinal, **options)

    def check(self, key, reference_id, ordinal, witness):
        actual = {"path": os.fspath(witness)} if isinstance(witness, (str, os.PathLike)) else {"witness": witness}
        return self._request("check", key=key, reference_id=reference_id, ordinal=ordinal, **actual)

    def check_files(self, key, reference_id, ordinal, files, format="tsv"):
        return self._request("check_files", key=key, reference_id=reference_id, ordinal=ordinal,
                             files={name: os.fspath(path) for name, path in files.items()},
                             format=format)

    env = staticmethod(env)

    def close(self):
        process, self._process = self._process, None
        if process is not None:
            process.stdin.close()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
            finally:
                process.stdout.close()

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()
