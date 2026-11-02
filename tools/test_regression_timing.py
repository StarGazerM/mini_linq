#!/usr/bin/env python3
"""Timing and immutable passing-evidence contracts; no Cargo compilation."""
import contextlib
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import test_regressions as runner


class TimingTests(unittest.TestCase):
    def test_nested_builds_are_unioned_and_clipped(self):
        self.assertEqual(runner.interval_seconds([(0, 3), (1, 2), (5, 9), (8, 12)], 1, 10), 7)
        self.assertEqual(runner.interval_seconds([], 0, 10), 0)

    def test_cargo_shim_preserves_output_status_and_boundary(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            cargo = root / "real-cargo"
            cargo.write_text("#!/usr/bin/env python3\nimport sys,time\n"
                             "print('    Finished `test` profile [unoptimized] target(s)', file=sys.stderr, flush=True)\n"
                             "time.sleep(0.03)\nprint('test output')\nsys.exit(7)\n")
            cargo.chmod(0o755)
            trace = root / "timings.jsonl"
            env = {**os.environ, "REGRESSION_REAL_CARGO": str(cargo),
                   "REGRESSION_CARGO_TIMINGS": str(trace)}
            result = subprocess.run([sys.executable, str(runner.ROOT / "tools/regression_cargo.py"), "test"],
                                    env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 7)
            self.assertEqual(result.stdout, "test output\n")
            self.assertIn("Finished `test` profile", result.stderr)
            record = json.loads(trace.read_text())
            self.assertEqual(record["status"], 7)
            self.assertEqual(record["boundary"], "finished")
            self.assertGreater(record["cargo_end"], record["end"])

    def test_missing_source_records_failed_evidence(self):
        with tempfile.TemporaryDirectory() as temporary:
            result = runner.execute(["absent", "test", "case", "missing"], Path(temporary))
            self.assertEqual(result[3], 1)
            self.assertEqual(result[8], "failed")
            self.assertAlmostEqual(result[5] + result[6], result[7])
            record = json.loads(Path(result[4]).with_suffix(".json").read_text())
            self.assertIsNone(record["identity"])
            self.assertEqual(record["status"], 1)

    def test_zero_impact_is_success(self):
        with tempfile.TemporaryDirectory() as temporary, \
                patch.object(sys, "argv", ["test_regressions.py", "--output", temporary]), \
                patch.object(runner, "run", return_value=[]), \
                contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(runner.main(), 0)


class EvidenceTests(unittest.TestCase):
    def test_latest_evidence_uses_mtime_recursively(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.assertIsNone(runner.latest_evidence(root))
            paths = [root / "z" / "evidence.json", root / "a" / "nested" / "evidence.json"]
            for index, path in enumerate(paths):
                path.parent.mkdir(parents=True)
                path.write_text("{}")
                os.utime(path, ns=(index + 1, index + 1))
            self.assertEqual(runner.latest_evidence(root), paths[1])

    def test_save_baseline_requires_complete_success_and_preserves_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "run"
            source.mkdir()
            (source / "evidence.json").write_text("evidence")
            (source / "case.log").write_text("passed")
            destination = root / "baselines"
            for revision, records in (("abc", []), (None, [{"status": 0}]),
                                       ("abc", [{"status": 0}, {"status": 1}])):
                self.assertIsNone(runner.save_baseline(source, destination, revision, records))
                self.assertFalse(destination.exists())
            saved = runner.save_baseline(source, destination, "1234567890abcdef", [{"status": 0}])
            self.assertEqual(saved.name, "1234567890ab")
            self.assertEqual((saved / "case.log").read_text(), "passed")
            self.assertEqual((saved / "evidence.json").read_text(), "evidence")
            with self.assertRaises(FileExistsError):
                runner.save_baseline(source, destination, "1234567890abcdef", [{"status": 0}])

    def test_list_latest_checks_identities_and_explains_missing_records(self):
        # Take the real inventory row so this test follows source relocations.
        import csv
        with (runner.ROOT / "tools/regression-targets.tsv").open() as source:
            row = next(row for row in csv.reader(source, delimiter="\t")
                       if row[:3] == ["cq-storage", "test", "access_laws"])
        for identity, records, decision in (
                ({"key": "same"}, {tuple(row): {"identity": {"key": "same"}, "run_id": "base"}}, "REUSE"),
                ({"key": "different"}, {tuple(row): {"identity": {"key": "same"}, "run_id": "base"}}, "EXECUTE"),
                (None, {}, "EXECUTE")):
            module = unittest.mock.Mock()
            module.Identity.return_value.fingerprint.return_value = identity
            output = io.StringIO()
            with patch.object(sys, "argv", ["runner", "--reuse-latest", "/baseline", "--list",
                                            "cq-storage/access_laws"]), \
                    patch.object(runner, "latest_evidence", return_value=Path("/baseline/evidence.json")), \
                    patch.object(runner, "load_evidence", return_value=records), \
                    patch.dict(sys.modules, {"regression_identity": module}), \
                    patch.dict(runner.RUN_OPTIONS, {}, clear=True), \
                    patch.object(runner, "run") as run, contextlib.redirect_stdout(output):
                self.assertEqual(runner.main(), 0)
                run.assert_not_called()
            self.assertIn(f"{decision} cq-storage/access_laws:", output.getvalue())

    def test_only_intact_executed_passing_evidence_is_reusable(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            log = root / "case.log"
            log.write_text("test result: ok\n")
            row = ["p", "test", "case", "source.rs"]
            record = {"row": row, "status": 0, "outcome": "passed", "identity": {"key": "k"},
                      "log": log.name, "log_sha256": hashlib.sha256(log.read_bytes()).hexdigest()}
            evidence = root / "evidence.json"
            def save(value):
                evidence.write_text(json.dumps({"version": 1, "run_id": "baseline", "targets": [value]}))
            save(record)
            self.assertEqual(runner.load_evidence(evidence)[tuple(row)]["run_id"], "baseline")
            for altered in ({**record, "status": 1}, {**record, "identity": None},
                            {**record, "outcome": "reused older"}):
                save(altered)
                self.assertEqual(runner.load_evidence(evidence), {})
            save(record)
            log.write_text("corrupted\n")
            self.assertEqual(runner.load_evidence(evidence), {})

    def test_reuse_requires_equal_current_identity(self):
        row = ["p", "test", "case", "source.rs"]
        prior = {"identity": {"key": "same"}, "run_id": "baseline"}
        for identity, expected in (({"key": "same"}, "reused baseline"),
                                   ({"key": "runtime-changed"}, "passed"), (None, "passed")):
            with self.subTest(identity=identity), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                (root / "source.rs").write_text("// test input\n")
                module = unittest.mock.Mock()
                module.Identity.return_value.fingerprint.return_value = identity
                module.Identity.return_value.last_reason = "unsupported"
                with patch.object(runner, "ROOT", root), \
                        patch.dict(runner.RUN_OPTIONS, {"reuse": {tuple(row): prior}}, clear=True), \
                        patch.dict(sys.modules, {"regression_identity": module}), \
                        patch.object(runner, "timing_environment", return_value={}), \
                        patch.object(runner.subprocess, "run",
                                     return_value=unittest.mock.Mock(returncode=0)) as command:
                    result = runner.execute(row, root)
                self.assertEqual(result[8], expected)
                self.assertEqual(command.call_count, int(expected == "passed"))

    def test_changing_inputs_during_execution_discard_identity(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "source.rs").write_text("// test input\n")
            module = unittest.mock.Mock()
            module.Identity.return_value.fingerprint.side_effect = [{"key": "before"}, {"key": "after"}]
            with patch.object(runner, "ROOT", root), \
                    patch.dict(runner.RUN_OPTIONS, {"record_identities": True}, clear=True), \
                    patch.dict(sys.modules, {"regression_identity": module}), \
                    patch.object(runner, "timing_environment", return_value={}), \
                    patch.object(runner.subprocess, "run", return_value=unittest.mock.Mock(returncode=0)):
                result = runner.execute(["p", "test", "case", "source.rs"], root)
            record = json.loads(Path(result[4]).with_suffix(".json").read_text())
            self.assertIsNone(record["identity"])


if __name__ == "__main__":
    unittest.main()
