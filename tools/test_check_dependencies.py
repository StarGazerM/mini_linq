#!/usr/bin/env python3
"""Regression coverage for dependency boundaries and offline lock alignment."""
import contextlib
import copy
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import check_dependencies


SOURCE = "registry+https://github.com/rust-lang/crates.io-index"


def lock(packages):
    return "version = 4\n" + "".join(
        f'\n[[package]]\nname = "{name}"\nversion = "{version}"\n'
        + (f'source = "{source}"\n' if source else "")
        for name, version, source in packages
    )


class FeatureLocks(unittest.TestCase):
    def check(self, root_packages, feature_packages, feature="extensions"):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "Cargo.lock").write_text(lock(root_packages))
            workspace = root / "features" / feature
            workspace.mkdir(parents=True)
            (workspace / "Cargo.lock").write_text(lock(feature_packages))
            with patch.object(check_dependencies, "ROOT", root), contextlib.redirect_stdout(io.StringIO()):
                check_dependencies.check_feature_locks()

    def test_shared_version(self):
        self.check([("syn", "2.0.119", SOURCE)], [("syn", "2.0.119", SOURCE)])

    def test_shared_version_drift(self):
        with self.assertRaisesRegex(SystemExit, "zerocopy 0.8.63"):
            self.check([("zerocopy", "0.8.59", SOURCE)], [("zerocopy", "0.8.63", SOURCE)])

    def test_shared_source_drift(self):
        with self.assertRaisesRegex(SystemExit, "syn 2.0.119"):
            self.check([("syn", "2.0.119", SOURCE)], [("syn", "2.0.119", "git+https://example.com/syn")])

    def test_feature_only_and_local_packages(self):
        self.check([("syn", "2.0.119", SOURCE)], [("ascent", "0.8.0", SOURCE), ("cq-local", "0.1.0", None)])

    def test_multiple_root_versions(self):
        self.check([("syn", "2.0.119", SOURCE), ("syn", "3.0.3", SOURCE)], [("syn", "3.0.3", SOURCE)])

    def test_ascent_hashbrown_exception_is_scoped(self):
        root_packages = [("hashbrown", "0.15.5", SOURCE)]
        feature_packages = [("hashbrown", "0.14.5", SOURCE)]
        for feature in ("datalog", "datalog-parallel"):
            with self.subTest(feature=feature):
                self.check(root_packages, feature_packages, feature)
                with self.assertRaisesRegex(SystemExit, "hashbrown 0.14.4"):
                    self.check(root_packages, [("hashbrown", "0.14.4", SOURCE)], feature)
        with self.assertRaisesRegex(SystemExit, "hashbrown 0.14.5"):
            self.check(root_packages, feature_packages)


class SequenceDependencies(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data = json.loads(subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--format-version=1", "--locked"],
            cwd=check_dependencies.ROOT,
        ))

    def check_package(self, name, production_only=False):
        data = copy.deepcopy(self.data)
        if production_only:
            for package in data["packages"]:
                package["dependencies"] = [
                    dep for dep in package["dependencies"] if dep["kind"] != "dev"
                ]
        with patch.object(check_dependencies, "PATHS", {name: check_dependencies.PATHS[name]}):
            check_dependencies.check(data, announce=False)

    def test_pull_adapters_are_permitted(self):
        self.check_package("cq-sequence-pull", production_only=True)

    def test_sequence_core_edges_are_permitted(self):
        for name in ("cq-sequence-lowering", "cq-sequence-plan", "cq-sequence-pull"):
            with self.subTest(package=name):
                self.check_package(name)

    def test_core_feature_edges_remain_forbidden(self):
        for name in ("cq-sequence-lowering", "cq-sequence-plan", "cq-sequence-pull"):
            for kind in (None, "dev", "build"):
                data = copy.deepcopy(self.data)
                package = next(p for p in data["packages"] if p["name"] == name)
                dependency = copy.deepcopy(package["dependencies"][0])
                dependency.update(
                    name="cq-workloads", kind=kind, optional=True,
                    path=str(check_dependencies.ROOT / "features/workloads"),
                )
                package["dependencies"].append(dependency)
                with self.subTest(package=name, kind=kind), patch.object(
                    check_dependencies, "PATHS", {name: check_dependencies.PATHS[name]}
                ), self.assertRaisesRegex(ValueError, "core depends on an optional feature"):
                    check_dependencies.check(data, announce=False)


if __name__ == "__main__":
    unittest.main()
