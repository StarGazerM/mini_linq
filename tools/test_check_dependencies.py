#!/usr/bin/env python3
"""Regression coverage for offline feature-lock dependency alignment."""
import contextlib
import io
from pathlib import Path
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
        self.check(root_packages, feature_packages, "datalog")
        with self.assertRaisesRegex(SystemExit, "hashbrown 0.14.5"):
            self.check(root_packages, feature_packages)


if __name__ == "__main__":
    unittest.main()
