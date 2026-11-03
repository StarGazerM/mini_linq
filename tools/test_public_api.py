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

    def test_tuple_field_ids_are_stable_without_changing_tuple_types(self):
        def dump(offset):
            def item(name, kind, value):
                return {"name": name, "crate_id": 0, "visibility": "public", "inner": {kind: value}}
            return {"root": offset, "paths": {}, "index": {
                str(offset): item("demo", "module", {"items": [offset + 1, offset + 3, offset + 6]}),
                str(offset + 1): item("Pair", "struct", {"kind": {"tuple": [offset + 2, None]}, "impls": []}),
                str(offset + 2): item(None, "struct_field", {"primitive": "usize"}),
                str(offset + 3): item("Choice", "enum", {"variants": [offset + 4], "impls": []}),
                str(offset + 4): item("Some", "variant", {"kind": {"tuple": [offset + 5]}}),
                str(offset + 5): item(None, "struct_field", {"primitive": "u32"}),
                str(offset + 6): item("Tuple", "type_alias", {"type": {"tuple": [{"primitive": "u8"}, {"primitive": "u16"}]}}),
            }}
        original = snapshot(dump(0))
        self.assertEqual(original, snapshot(dump(7000)))
        self.assertIn("demo::Pair::0", original)
        self.assertIn("demo::Choice::Some::0", original)
        changed = dump(7000)
        changed["index"]["7006"]["inner"]["type_alias"]["type"]["tuple"][0] = {"primitive": "u32"}
        self.assertNotEqual(original, snapshot(changed))


if __name__ == "__main__":
    unittest.main()
