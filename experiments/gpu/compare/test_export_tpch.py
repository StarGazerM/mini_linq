"""Read-only prepared-key manifest regressions."""
import hashlib
import json
from pathlib import Path
import struct
import tempfile
import unittest

from export_tpch import certify_manifest


class VerifiedManifestTests(unittest.TestCase):
    def test_unique_and_duplicate_keys_are_checked_without_rewriting_sources(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            data = struct.pack('<iii', 3, 1, 2)
            (root / 'orders.o_orderkey.bin').write_bytes(data)
            manifest = {'schema_version': 2, 'sf': 1, 'byte_order': 'little',
                        'relations': {'orders': {'rows': 3, 'columns': [{
                            'name': 'o_orderkey', 'type': 'i32',
                            'encoding': 'plain', 'logical_type': 'Integer',
                            'file': 'orders.o_orderkey.bin'}]}}}
            source = root / 'manifest.json'
            source.write_text(json.dumps(manifest))
            before = source.read_bytes()
            destination = root / 'overrides' / 'sf1.json'
            certify_manifest(root, destination)
            result = json.loads(destination.read_text())
            self.assertEqual(source.read_bytes(), before)
            self.assertEqual((root / 'orders.o_orderkey.bin').read_bytes(), data)
            self.assertEqual(result['relations']['orders']['verified_keys'], [{
                'set': 'verified', 'rows': 3, 'types': {'o_orderkey': 'i32'},
                'columns': {'o_orderkey': hashlib.sha256(data).hexdigest()}}])
            (root / 'orders.o_orderkey.bin').write_bytes(struct.pack('<iii', 3, 3, 2))
            certify_manifest(root, destination)
            result = json.loads(destination.read_text())
            self.assertEqual(result['relations']['orders']['verified_keys'], [])


if __name__ == '__main__':
    unittest.main()
