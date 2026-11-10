"""Relocation and escape regressions; the Cargo gate also builds the fixtures."""
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

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

    def test_symlinked_output_directory_is_not_a_closure_escape(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            consumer = root / "sources/consumer"
            consumer.mkdir(parents=True)
            gate.write_manifest(consumer / "Cargo.toml", {
                "package": {"name": "consumer", "version": "0.1.0"}, "workspace": {}})
            actual = root / "actual-output"
            actual.mkdir()
            alias = root / "aliased-output"
            alias.symlink_to(actual, target_is_directory=True)
            destination, closure = gate.copy_feature_closure(consumer, alias, {})
            manifest = (destination / "Cargo.toml").resolve()
            self.assertTrue(manifest.is_relative_to(closure))
            self.assertEqual(closure, actual.resolve() / "features")

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
            with self.assertRaises(RuntimeError):
                gate.copy_feature_closure(consumer, root / "copy", {})

    def test_packaging_preserves_root_lock_before_offline_resolution(self):
        class PackagingReached(Exception):
            pass

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "checkout"
            root.mkdir()
            gate.write_manifest(root / "Cargo.toml", {
                "workspace": {"package": {}, "dependencies": {}}})
            gate.write_manifest(root / "rust-toolchain.toml", {
                "toolchain": {"channel": "stable"}})
            locked = 'version = 4\n\n[[package]]\nname = "zip"\nversion = "6.0.0"\n'
            (root / "Cargo.lock").write_text(locked)
            destination = Path(tmp) / "packaged"
            destination.mkdir()

            def run(command, cwd, log, env=None):
                if command[1] == "vendor":
                    Path(command[-1]).mkdir()
                elif command[1] == "package":
                    self.assertIn("--offline", command)
                    self.assertEqual((cwd / "Cargo.lock").read_text(), locked)
                    raise PackagingReached
                else:
                    self.fail(f"unexpected command: {command}")

            with patch.object(gate, "ROOT", root), \
                    patch.object(gate.subprocess, "check_output", return_value=b'{"packages": []}'), \
                    patch.object(gate, "run", side_effect=run), \
                    self.assertRaises(PackagingReached):
                gate.package_core(destination, Path(tmp) / "package.log")


class ConsolidatedClosureTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name) / "checkout"
        self.root.mkdir()
        self.output = Path(self.temporary.name) / "outside"
        self.root_patch = patch.object(gate, "ROOT", self.root)
        self.root_patch.start()
        self.addCleanup(self.root_patch.stop)

    def manifest(self, path, spec):
        manifest = self.root / path / "Cargo.toml"
        manifest.parent.mkdir(parents=True, exist_ok=True)
        gate.write_manifest(manifest, spec)
        return manifest

    def test_directory_group_materializes_inheritance_and_retains_sibling_assets(self):
        self.manifest(".", {"workspace": {
            "members": ["features/datalog/app", "features/extensions/support", "runtime/cq-storage"],
            "package": {"version": "0.2.0", "edition": "2024", "rust-version": "1.85.1", "license": "MIT"},
            "dependencies": {
                "support-alias": {"package": "support", "path": "features/extensions/support", "features": ["base"], "default-features": False},
                "core-alias": {"package": "cq-storage", "path": "runtime/cq-storage"},
                "unused": {"path": "benchmarks/not-copied"}},
            "lints": {"rust": {"unsafe_code": "forbid"}}}})
        self.manifest("features/datalog/app", {
            "package": {"name": "app", "workspace": "../../..", "version": {"workspace": True}, "edition": {"workspace": True}, "rust-version": {"workspace": True}, "license": {"workspace": True}},
            "target": {"cfg(unix)": {"build-dependencies": {
                "support-alias": {"workspace": True, "features": ["extra"], "optional": True}}}},
            "lints": {"workspace": True}})
        self.manifest("features/extensions/support", {
            "package": {"name": "support", "version": {"workspace": True}},
            "dependencies": {"core-alias": {"workspace": True, "features": ["counters"]}}})
        self.manifest("runtime/cq-storage", {"package": {"name": "cq-storage", "version": "0.1.0"}})
        golden = self.root / "features/extensions/golden/expected.csv"
        golden.parent.mkdir()
        golden.write_text("1,2\n")
        self.assertEqual(gate.feature_groups(), [self.root / "features/datalog", self.root / "features/extensions"])
        destination, closure = gate.copy_feature_closure(self.root / "features/datalog", self.output, {"cq-storage": "0.1.0"})
        root = gate.tomllib.loads((destination / "Cargo.toml").read_text())
        self.assertEqual(root["workspace"], {"members": ["app"], "resolver": "3"})
        app = gate.tomllib.loads((destination / "app/Cargo.toml").read_text())
        self.assertEqual(app["package"], {"name": "app", "version": "0.2.0", "edition": "2024", "rust-version": "1.85.1", "license": "MIT"})
        self.assertEqual(app["lints"], {"rust": {"unsafe_code": "forbid"}})
        dep = app["target"]["cfg(unix)"]["build-dependencies"]["support-alias"]
        self.assertEqual(dep, {"package": "support", "path": "../../extensions/support", "features": ["base", "extra"], "default-features": False, "optional": True})
        sibling = gate.tomllib.loads((closure / "extensions/support/Cargo.toml").read_text())
        self.assertEqual(sibling["dependencies"]["core-alias"], {"package": "cq-storage", "version": "=0.1.0", "features": ["counters"]})
        sibling_root = gate.tomllib.loads((closure / "extensions/Cargo.toml").read_text())
        self.assertEqual(sibling_root["workspace"]["members"], ["support"])
        self.assertNotIn("workspace", sibling)
        self.assertEqual((closure / "extensions/golden/expected.csv").read_text(), "1,2\n")
        for manifest in gate.manifests(closure):
            text = manifest.read_text()
            self.assertNotIn(str(self.root), text)
            self.assertNotIn("runtime/cq-storage", text)
            self.assertNotIn("benchmarks", text)
        self.assertEqual({str(p.relative_to(closure)) for p in gate.manifests(closure)}, {
            "datalog/Cargo.toml", "datalog/app/Cargo.toml",
            "extensions/Cargo.toml", "extensions/support/Cargo.toml"})

    def test_inherited_nonconsumer_path_is_rejected(self):
        self.manifest(".", {"workspace": {"dependencies": {"escape": {"path": "benchmarks/escape"}}}})
        self.manifest("features/app", {"package": {"name": "app", "version": "0.1.0"}, "dev-dependencies": {"escape": {"workspace": True}}})
        self.manifest("benchmarks/escape", {"package": {"name": "escape", "version": "0.1.0"}})
        with self.assertRaises(RuntimeError):
            gate.copy_feature_closure(self.root / "features/app", self.output, {})

    def test_inherited_package_files_are_copied_not_linked_to_root(self):
        self.manifest(".", {"workspace": {"package": {"readme": "README.txt", "license-file": "LICENSE"}}})
        (self.root / "README.txt").write_text("consumer readme")
        (self.root / "LICENSE").write_text("consumer license")
        self.manifest("features/app", {"package": {"name": "app", "version": "0.1.0", "readme": {"workspace": True}, "license-file": {"workspace": True}}})
        destination, _ = gate.copy_feature_closure(self.root / "features/app", self.output, {})
        package = gate.tomllib.loads((destination / "Cargo.toml").read_text())["package"]
        self.assertEqual((destination / package["readme"]).read_text(), "consumer readme")
        self.assertEqual((destination / package["license-file"]).read_text(), "consumer license")

    def test_directory_groups_remain_selectable_without_virtual_manifests(self):
        for path in ("features/datalog/doop/srd", "features/extensions/support", "features/workloads"):
            self.manifest(path, {"package": {"name": path.rsplit("/", 1)[-1], "version": "0.1.0"}})
        self.assertEqual([path.name for path in gate.feature_groups()], ["datalog", "extensions", "workloads"])


class PackagedIsolationTests(unittest.TestCase):
    def test_nested_consumer_language_pass_and_facade_are_not_core(self):
        paths = [
            "languages/cq-lang-ra/Cargo.toml",
            "passes/cq-order/Cargo.toml",
            "io/cq-load/Cargo.toml",
            "io/cq-io-macros/Cargo.toml",
            "features/datalog/languages/dl-source/Cargo.toml",
            "features/datalog/passes/schedule/Cargo.toml",
            "features/datalog/facade/macros/Cargo.toml",
        ]
        packages = [{"manifest_path": str(gate.ROOT / path)} for path in paths]
        self.assertEqual(gate.core_packages(packages), packages[:4])

    def test_packaging_workspace_drops_every_uncopied_local_dependency(self):
        packages = [{"manifest_path": str(gate.ROOT / "runtime/cq-storage/Cargo.toml")}]
        dependencies = {
            "storage-alias": {"package": "cq-storage", "path": "runtime/cq-storage"},
            "consumer": {"path": "features/workloads"},
            "misleading-core-alias": {"package": "cq-storage", "path": "features/other"},
            "syn": "2",
            "quote": {"version": "1"},
        }
        filtered = gate.packaging_dependencies(dependencies, packages)
        self.assertEqual(set(filtered), {"storage-alias", "syn", "quote"})
        filtered["storage-alias"]["version"] = "0.1.0"
        self.assertNotIn("version", dependencies["storage-alias"])

    def test_existing_positive_and_negative_fixture_topologies_are_preserved(self):
        names = {}
        for folder in ("languages", "passes", "runtime", "io", "facade", "testkit"):
            for manifest in (gate.ROOT / folder).glob("*/Cargo.toml"):
                name = gate.tomllib.loads(manifest.read_text())["package"]["name"]
                names[name] = "0.1.0"
        for fixture in ("external", "in-tree-only"):
            with self.subTest(fixture=fixture), tempfile.TemporaryDirectory() as tmp:
                source = gate.ROOT / "tools/fixtures" / fixture
                destination, closure = gate.copy_feature_closure(source, Path(tmp), names)
                original = gate.tomllib.loads((source / "Cargo.toml").read_text())
                copied = gate.tomllib.loads((destination / "Cargo.toml").read_text())
                self.assertEqual(copied["workspace"], original["workspace"])
                self.assertEqual((destination / "src/lib.rs").read_text(), (source / "src/lib.rs").read_text())
                if fixture == "in-tree-only":
                    self.assertFalse((destination / "src/../../../../Cargo.toml").resolve().exists())
                for manifest in gate.manifests(closure):
                    for package, dependency in gate.path_dependencies(gate.tomllib.loads(manifest.read_text())):
                        self.assertNotIn(package, names)
                        self.assertTrue((manifest.parent / dependency["path"]).resolve().is_relative_to(closure))

    def test_metadata_rejects_checkout_and_nonpackaged_core(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "checkout"
            root.mkdir()
            gate.write_manifest(root / "Cargo.toml", {"workspace": {}})
            gate.write_manifest(root / "rust-toolchain.toml", {"toolchain": {"channel": "stable"}})
            feature = root / "features/app"
            feature.mkdir(parents=True)
            gate.write_manifest(feature / "Cargo.toml", {"package": {"name": "app", "version": "0.1.0"}})
            vendor = Path(tmp) / "vendor"
            cases = [
                (root / "runtime/core/Cargo.toml", None, "in-tree crate"),
                (Path(tmp) / "uncopied/core/Cargo.toml", None, "escaped copied closure"),
                (vendor / "wrong-core/Cargo.toml", "registry+example", "did not resolve from packaged source"),
            ]
            for index, (manifest, source, message) in enumerate(cases):
                metadata = {"packages": [{"name": "core", "manifest_path": str(manifest), "source": source}]}
                with self.subTest(case=message), patch.object(gate, "ROOT", root), \
                        patch.object(gate.subprocess, "check_output", return_value=gate.json.dumps(metadata).encode()), \
                        patch.object(gate, "run") as run, self.assertRaises(RuntimeError):
                    gate.test_feature(feature, Path(tmp) / f"copy-{index}", {"core": "0.1.0"}, vendor,
                                      Path(tmp) / "external.log", Path(tmp) / "target")
                run.assert_not_called()


if __name__ == "__main__":
    unittest.main()
