"""Relocation and escape regressions; the Cargo gate also builds the fixtures."""
from pathlib import Path
import tempfile
import unittest

import check_external as gate


class FeatureClosureTests(unittest.TestCase):
    def test_transitive_siblings_inherited_core_and_layout(self):
        with tempfile.TemporaryDirectory() as tmp:
            consumer, closure = gate.copy_feature_closure(
                gate.ROOT / "tools/fixtures/external-siblings/consumer",
                Path(tmp), {"cq-storage": "0.1.0"})
            self.assertEqual(consumer, closure / "consumer")
            self.assertEqual({str(p.relative_to(closure)) for p in gate.manifests(closure)}, {
                "consumer/Cargo.toml", "support/Cargo.toml",
                "support/nested/Cargo.toml", "leaf/Cargo.toml"})
            for manifest in gate.manifests(closure):
                spec = gate.tomllib.loads(manifest.read_text())
                for _, value in gate.path_dependencies(spec):
                    path = (manifest.parent / value["path"]).resolve()
                    self.assertTrue(path.is_relative_to(closure))
                    self.assertTrue((path / "Cargo.toml").is_file())
            nested = gate.tomllib.loads((closure / "support/nested/Cargo.toml").read_text())
            self.assertEqual(nested["dependencies"]["cq-storage"], {"version": "=0.1.0"})

    def test_absolute_sibling_is_relocated(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            consumer = root / "sources/consumer"
            sibling = root / "sources/sibling"
            for path in (consumer, sibling):
                path.mkdir(parents=True)
                (path / "src").mkdir()
                (path / "src/lib.rs").write_text("")
            gate.write_manifest(sibling / "Cargo.toml", {
                "package": {"name": "sibling", "version": "0.1.0"}, "workspace": {}})
            gate.write_manifest(consumer / "Cargo.toml", {
                "package": {"name": "consumer", "version": "0.1.0"}, "workspace": {},
                "target": {"cfg(unix)": {"build-dependencies": {
                    "sibling": {"path": str(sibling)}}}},
                "patch": {"crates-io": {"core-alias": {
                    "package": "cq-storage", "path": "../../checkout/runtime/cq-storage"}}}})
            destination, closure = gate.copy_feature_closure(consumer, root / "copy", {"cq-storage": "0.1.0"})
            spec = gate.tomllib.loads((destination / "Cargo.toml").read_text())
            dependency = spec["target"]["cfg(unix)"]["build-dependencies"]["sibling"]
            self.assertEqual(dependency["path"], "../sibling")
            self.assertTrue((closure / "sibling/Cargo.toml").is_file())
            self.assertFalse(spec["patch"]["crates-io"])

    def test_dependency_escape_is_rejected_before_cargo(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            consumer = root / "sources/consumer"
            consumer.mkdir(parents=True)
            outside = root / "outside"
            outside.mkdir()
            gate.write_manifest(outside / "Cargo.toml", {
                "package": {"name": "escape", "version": "0.1.0"}})
            gate.write_manifest(consumer / "Cargo.toml", {
                "package": {"name": "consumer", "version": "0.1.0"}, "workspace": {},
                "dependencies": {"escape": {"path": "../../outside", "optional": True}}})
            with self.assertRaisesRegex(RuntimeError, "escapes feature siblings"):
                gate.copy_feature_closure(consumer, root / "copy", {})


if __name__ == "__main__":
    unittest.main()
