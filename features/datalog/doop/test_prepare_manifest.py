import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from prepare_manifest import certify, decode, prepare, schema


class PrepareManifestTests(unittest.TestCase):
    def test_verified_keys_are_checked_on_decoded_rows(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "r.csv"
            data = b"2\t7\r\n1\t9\r\n1\t8\r\n"
            path.write_bytes(data)
            result = certify(path, ["i32", "i32"], [[0, 1], [0]], True)
            self.assertEqual(result, {"types": ["i32", "i32"], "verified_keys": [{
                "columns": [0, 1], "set": "verified", "sha256": hashlib.sha256(data).hexdigest()
            }]})
            path.write_bytes(b"+2\t07\n2\t7\n")
            self.assertEqual(certify(path, ["i32", "i32"], [[0, 1]], True)["verified_keys"], [])

    def test_decoder_matches_empty_and_integer_semantics(self):
        self.assertEqual(decode(b"\n", ["string"], False), ("",))
        self.assertIsNone(decode(b"\r\n", ["i32"], True))
        self.assertEqual(decode(b"-2147483648\t+2147483647", ["i32", "i32"], True),
                         (-2147483648, 2147483647))
        for data in [b"2147483648", b" 1", b"1_0", b"1\t2", b"\xff"]:
            with self.assertRaises(ValueError):
                decode(data, ["i32"], False)

    def test_raw_missing_inputs_and_full_row_duplicates(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "FormalParam.facts").write_bytes(b"1\tx\ty\n+1\tx\ty\n")
            result = prepare(root, "raw")
            self.assertEqual(result, {"version": 1, "relations": {
                "FormalParam.facts": {"types": ["i32", "string", "string"], "verified_keys": []}
            }})
            with self.assertRaises(ValueError):
                prepare(root, "raw", {"Unknown.facts": [[0]]})

    def test_prepared_manifest_and_external_output_do_not_rewrite_inputs(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            data = root / "data"
            data.mkdir()
            for filename in schema("prepared"):
                (data / filename).write_bytes(b"")
            (data / "DirectSuperclass.csv").write_bytes(b"2\t7\n1\t8\n")
            before = {path.name: path.read_bytes() for path in data.iterdir()}
            output = root / "certificate.json"
            subprocess.run([sys.executable, str(Path(__file__).with_name("prepare_manifest.py")),
                            str(data), "--format", "prepared", "--manifest", str(output),
                            "--key", "DirectSuperclass.csv:0"], check=True)
            manifest = json.loads(output.read_text())
            self.assertEqual([key["columns"] for key in
                              manifest["relations"]["DirectSuperclass.csv"]["verified_keys"]],
                             [[0, 1], [0]])
            self.assertEqual(before, {path.name: path.read_bytes() for path in data.iterdir()})

    def test_invalid_keys_are_not_certified(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "r.csv"
            path.write_bytes(b"1\n")
            for key in [[], [1], [-1], [0, 0]]:
                with self.assertRaises(ValueError):
                    certify(path, ["i32"], [key], True)


if __name__ == "__main__":
    unittest.main()
