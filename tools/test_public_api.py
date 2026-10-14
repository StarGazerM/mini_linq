"""Regression for rustdoc's unnamed public use items and local glob exports."""
import copy
import unittest
from check_public_api import snapshot


class Reexports(unittest.TestCase):
    def test_reexported_private_module_type_and_glob_fields_are_snapshotted(self):
        def item(name, kind, value, visibility="public"):
            return {"name": name, "crate_id": 0, "visibility": visibility, "inner": {kind: value}}
        data = {"root": 0, "paths": {}, "index": {
            "0": item("demo", "module", {"items": [1, 4, 5]}),
            "1": item("private", "module", {"items": [2]}, "default"),
            "2": item("Filter", "struct", {"kind": {"plain": {"fields": [3]}}, "impls": []}),
            "3": item("budget", "struct_field", {"primitive": "usize"}),
            "4": item(None, "use", {"source": "private::Filter", "name": "Alias", "id": 2, "is_glob": False}),
            "5": item(None, "use", {"source": "private", "name": "private", "id": 1, "is_glob": True}),
        }}
        original = snapshot(data)
        self.assertIn("demo::Filter::budget", original)
        self.assertNotIn("demo::private::Filter", original)
        changed = copy.deepcopy(data)
        changed["index"]["3"]["inner"]["struct_field"] = {"primitive": "u32"}
        self.assertNotEqual(original, snapshot(changed))
        data["index"]["0"]["inner"]["module"]["items"].remove(5)
        self.assertIn("demo::Alias::budget", snapshot(data))


if __name__ == "__main__":
    unittest.main()
