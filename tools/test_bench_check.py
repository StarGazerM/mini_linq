"""Exercise the Python transport against the real compiled shared checker."""
import json
from pathlib import Path
import tempfile
import unittest

from bench_check import Checker


class BenchCheckTests(unittest.TestCase):
    def test_first_full_is_per_binary_configuration_and_reference_is_cached(self):
        with Checker(sample_size=2) as checker:
            checker.register("dataset", {"r": [[str(i)] for i in range(4)], "empty": []})
            first = checker.plan("binary-a/t1", "dataset", 0)
            self.assertEqual(first["mode"], "full")
            self.assertEqual(Checker.env(first)["BENCH_CHECK_MODE"], "full")
            with self.assertRaises(RuntimeError):
                checker.check("binary-a/t1", "dataset", 0,
                              {"r": {"count": 4, "rows": [["0"], ["1"], ["2"], ["9"]]},
                               "empty": {"count": 0, "rows": []}})
            self.assertEqual(checker.plan("binary-a/t1", "dataset", 1)["mode"], "full")
            passed = checker.check("binary-a/t1", "dataset", 1,
                                   {"r": {"count": 4, "rows": [[str(i)] for i in range(4)]},
                                    "empty": {"count": 0, "rows": []}})
            self.assertEqual(passed["first_full_sample"], 1)
            self.assertEqual(checker.plan("binary-a/t1", "dataset", 2)["mode"], "sampled")
            sampled = checker.check("binary-a/t1", "dataset", 2,
                                    {"r": {"count": 4, "rows": [["0"], ["3"]]},
                                     "empty": {"count": 0, "rows": []}})
            self.assertEqual(sampled["relations"]["r"]["sampled_count"], 2)
            self.assertEqual(sampled["first_full_sample"], 1)
            self.assertEqual(checker.plan("binary-a/t24", "dataset", 0)["mode"], "full")
            self.assertEqual(checker.plan("binary-b/t1", "dataset", 0)["mode"], "full")
            # Proves reference registration does not reopen per sample.
            self.assertTrue(checker.register_files("dataset", {"r": "/not/a/file"})["cached"])

    def test_native_file_adapter_counts_and_samples_actual_rows(self):
        with tempfile.TemporaryDirectory() as directory, Checker(sample_size=10) as checker:
            path = Path(directory) / "r.tsv"
            path.write_text("".join(f"{i}\tvalue-{i}\n" for i in range(100)))
            checker.register_files("r", {"answer": path})
            full = checker.plan("binary", "r", 0)
            self.assertEqual(full["mode"], "full")
            checker.check_files("binary", "r", 0, {"answer": path})
            sampled = checker.plan("binary", "r", 1, seed=17)
            self.assertEqual(sampled["seed"], 17)
            result = checker.check_files("binary", "r", 1, {"answer": path})
            self.assertEqual(result["relations"]["answer"]["actual_count"], 100)
            self.assertEqual(result["relations"]["answer"]["sampled_count"], 10)
            checker.plan("binary", "r", 2)
            path.write_text("changed\tvalue\n" * 100)
            with self.assertRaises(RuntimeError):
                checker.check_files("binary", "r", 2, {"answer": path})

    def test_witness_files_keep_exact_symbols_and_relation_counts(self):
        with tempfile.TemporaryDirectory() as directory, Checker() as checker:
            path = Path(directory) / "reference.json"
            witness = {"symbols": {"count": 2, "rows": [["1"], ["1.0"]]}}
            path.write_text(json.dumps(witness))
            checker.register_witness("symbols", path)
            checker.plan("binary", "symbols", 0)
            checker.check("binary", "symbols", 0, path)
            checker.plan("binary", "symbols", 1)
            with self.assertRaises(RuntimeError):
                checker.check("binary", "symbols", 1,
                              {"symbols": {"count": 3, "rows": [["1"], ["1.0"], ["2"]]}})
            checker.plan("binary", "symbols", 2)
            with self.assertRaises(RuntimeError):
                checker.check("binary", "symbols", 2, {})


if __name__ == "__main__":
    unittest.main()
