#!/usr/bin/env python3
"""Scheduler contracts; no Cargo builds or real regression targets."""
import contextlib
import io
from pathlib import Path
import tempfile
import threading
import unittest
from unittest.mock import patch

import test_regressions as runner


class SchedulerTests(unittest.TestCase):
    def test_independent_groups_overlap_without_shared_group_overlap(self):
        rows = [[group, "test", name, "source"] for group, name in
                [("a", "one"), ("a", "two"), ("b", "three"), ("b", "four")]]
        barrier = threading.Barrier(2)
        lock = threading.Lock()
        active = set()
        peak = 0

        def execute(row, logdir):
            nonlocal peak
            group = row[0]
            with lock:
                self.assertNotIn(group, active)
                active.add(group)
                peak = max(peak, len(active))
            barrier.wait(timeout=5)
            with lock:
                active.remove(group)
            return (*row[:3], int(row[2] == "two"), str(logdir / row[2]))

        with patch.object(runner, "resources_for", side_effect=lambda row: frozenset({row[0]})), \
                patch.object(runner, "execute", side_effect=execute), \
                contextlib.redirect_stdout(io.StringIO()):
            results = runner.run(rows, 2, Path("logs"))
        self.assertEqual(peak, 2)
        self.assertEqual([result[:3] for result in results], [tuple(row[:3]) for row in rows])
        self.assertEqual([result[3] for result in results], [0, 1, 0, 0])

    def test_one_job_preserves_inventory_order(self):
        rows = [["p", "test", str(index), "source"] for index in range(4)]
        dispatched = []

        def execute(row, logdir):
            dispatched.append(row[2])
            return (*row[:3], 0, "log")

        with patch.object(runner, "resources_for", return_value=frozenset()), \
                patch.object(runner, "execute", side_effect=execute), \
                contextlib.redirect_stdout(io.StringIO()):
            runner.run(rows, 1, Path("logs"))
        self.assertEqual(dispatched, ["0", "1", "2", "3"])

    def test_workspace_or_target_directory_conflicts(self):
        self.assertTrue(runner.conflicts({"workspace-a", "target"}, {"workspace-b", "target"}))
        self.assertTrue(runner.conflicts({"workspace", "target-a"}, {"workspace", "target-b"}))
        self.assertFalse(runner.conflicts({"workspace-a", "target-a"}, {"workspace-b", "target-b"}))
        self.assertTrue(runner.conflicts({"*"}, {"target"}))
        self.assertTrue(runner.conflicts({"target"}, {"*"}))

    def test_metadata_honors_workspace_and_target_directory(self):
        result = unittest.mock.Mock(returncode=0, stdout='{"workspace_root":"/workspace","target_directory":"/shared"}')
        runner.cargo_resources.cache_clear()
        with patch.object(runner.subprocess, "run", return_value=result) as command:
            self.assertEqual(runner.cargo_resources("Cargo.toml", "."), {"/workspace", "/shared"})
            self.assertEqual(command.call_args.kwargs["cwd"], runner.ROOT)
        runner.cargo_resources.cache_clear()

    def test_metadata_failure_serializes_but_does_not_skip_target(self):
        runner.cargo_resources.cache_clear()
        with patch.object(runner.subprocess, "run", return_value=unittest.mock.Mock(returncode=101)):
            self.assertEqual(runner.cargo_resources("Cargo.toml", "."), {"*"})
        runner.cargo_resources.cache_clear()

    def test_missing_source_has_same_status_and_log_name(self):
        with tempfile.TemporaryDirectory() as directory:
            row = ["p", "test", "shard_*", "missing-regression-source"]
            result = runner.execute(row, Path(directory))
            self.assertEqual(result[3], 1)
            self.assertEqual(Path(result[4]).name, "p-shard_all.log")
            self.assertEqual(Path(result[4]).read_text(), "Missing regression source: missing-regression-source\n")

    def test_commands_preserve_original_flags(self):
        cases = {
            "test": ["cargo", "test", "--locked", "-p", "p", "--all-features", "--test", "t"],
            "example": ["cargo", "test", "--locked", "-p", "p", "--all-features", "--example", "t"],
            "lib": ["cargo", "test", "--locked", "-p", "p", "--lib"],
            "manifest-test": ["cargo", "test", "--locked", "--manifest-path", "source"],
            "oracle-test": ["cargo", "test", "--locked", "--manifest-path", "source", "--features", "oracle"],
            "manifest-target-test": ["cargo", "test", "--locked", "--manifest-path", "source", "--test", "t"],
            "scenario": ["python3", "source"],
            "manifest-run": ["cargo", "run", "--locked", "--manifest-path", "source"],
            "oracle": ["cargo", "run", "--locked", "--manifest-path", "source", "--features", "oracle", "--", "t"],
        }
        for kind, expected in cases.items():
            with self.subTest(kind=kind):
                self.assertEqual(runner.command_for("p", kind, "t", "source"), expected)
        with self.assertRaises(ValueError):
            runner.command_for("p", "unknown", "t", "source")

    def test_empty_selection(self):
        self.assertEqual(runner.run([], 2, Path("logs")), [])


if __name__ == "__main__":
    unittest.main()
