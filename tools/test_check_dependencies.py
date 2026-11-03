#!/usr/bin/env python3
"""Regression coverage for closed-core boundaries and consolidated ownership."""
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


class FeatureOwnership(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.manifest = self.root / "features/extensions/support/Cargo.toml"
        self.manifest.parent.mkdir(parents=True)
        self.manifest.write_text('[package]\nname = "support"\nversion = "0.1.0"\n')
        self.data = {"workspace_root": str(self.root), "workspace_members": ["support-id"],
                     "packages": [{"name": "support", "id": "support-id", "manifest_path": str(self.manifest)}]}

    def check(self):
        with patch.object(check_dependencies, "ROOT", self.root), contextlib.redirect_stdout(io.StringIO()):
            check_dependencies.check_feature_ownership(self.data)

    def test_consolidated_feature_membership(self):
        self.check()

    def test_missing_root_membership_is_rejected(self):
        self.data["workspace_members"] = []
        with self.assertRaises(ValueError):
            self.check()

    def test_nested_workspace_is_rejected(self):
        self.manifest.write_text(self.manifest.read_text() + '\n[workspace]\n')
        with self.assertRaises(ValueError):
            self.check()

    def test_nested_lock_is_rejected_at_any_depth(self):
        (self.manifest.parent / "Cargo.lock").write_text("version = 4\n")
        with self.assertRaises(ValueError):
            self.check()

    def test_target_override_is_rejected(self):
        config = self.manifest.parent / ".cargo/config.toml"
        config.parent.mkdir()
        config.write_text('[build]\ntarget-dir = "../target"\n')
        with self.assertRaises(ValueError):
            self.check()

    def test_wrong_metadata_workspace_is_rejected(self):
        self.data["workspace_root"] = str(self.manifest.parent)
        with self.assertRaises(ValueError):
            self.check()

    def test_independent_external_fixture_and_build_artifacts_are_ignored(self):
        for relative in ("tools/fixtures/external", "features/target/generated"):
            fixture = self.root / relative
            fixture.mkdir(parents=True)
            (fixture / "Cargo.toml").write_text('[workspace]\n')
            (fixture / "Cargo.lock").write_text('version = 4\n')
        self.check()


class ClosedCoreDependencies(unittest.TestCase):
    def check(self, dependency):
        core_path = check_dependencies.ROOT / "facade/mini-linq"
        feature_path = check_dependencies.ROOT / "features/example"
        packages = [
            {"name": "mini-linq", "manifest_path": str(core_path / "Cargo.toml"),
             "dependencies": [dependency], "features": {}, "targets": []},
            {"name": "consumer", "manifest_path": str(feature_path / "Cargo.toml"),
             "dependencies": [], "features": {}, "targets": []},
            {"name": "benchmark", "manifest_path": str(check_dependencies.ROOT / "experiments/cpu/Cargo.toml"),
             "dependencies": [], "features": {}, "targets": []},
        ]
        with patch.object(check_dependencies, "PATHS", {"mini-linq": Path("facade/mini-linq")}), \
                patch.object(check_dependencies, "FACADES", {"mini-linq": Path("facade/mini-linq")}), \
                patch.object(check_dependencies, "FEATURE_NAMES", {"consumer"}):
            check_dependencies.check({"packages": packages}, announce=False)

    def test_all_feature_edge_forms_are_rejected_even_inside_workspace(self):
        for kind in (None, "dev", "build"):
            for optional in (False, True):
                for renamed in (None, "innocent_alias"):
                    for local in (False, True):
                        dependency = {"name": "consumer", "kind": kind, "optional": optional,
                                      "rename": renamed, "target": "cfg(unix)"}
                        if local:
                            dependency["path"] = str(check_dependencies.ROOT / "features/example")
                        with self.subTest(kind=kind, optional=optional, renamed=renamed, local=local), \
                                self.assertRaises(ValueError):
                            self.check(dependency)

    def test_feature_path_rejects_unknown_or_aliased_package_name(self):
        with self.assertRaises(ValueError):
            self.check({"name": "unknown", "kind": "dev", "path": str(check_dependencies.ROOT / "features/example")})

    def test_workspace_membership_does_not_allow_facade_consumer_edges(self):
        with self.assertRaises(ValueError):
            self.check({"name": "benchmark", "kind": None,
                        "path": str(check_dependencies.ROOT / "experiments/cpu")})


class SequenceDependencies(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data = json.loads(subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--format-version=1", "--locked"],
            cwd=check_dependencies.ROOT,
        ))

    def test_default_members_exclude_explicit_instrumentation(self):
        instrumentation = {"counters", "operator-timing", "phase-timing", "hole-timing"}
        excluded = {
            package["id"] for package in self.data["packages"]
            if any(dependency["kind"] in (None, "dev")
                   and not dependency["optional"]
                   and instrumentation.intersection(dependency["features"])
                   for dependency in package["dependencies"])
        }
        self.assertEqual(
            set(self.data["workspace_default_members"]),
            set(self.data["workspace_members"]) - excluded,
        )
        excluded_names = {package["name"] for package in self.data["packages"]
                          if package["id"] in excluded}
        rows = [line.split("\t") for line in
                (check_dependencies.ROOT / "tools/regression-targets.tsv").read_text().splitlines()
                if line and not line.startswith("#")]
        package_gates = {row[0] for row in rows if row[1] == "manifest-test"}
        self.assertLessEqual(excluded_names, package_gates)

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
                ), self.assertRaises(ValueError):
                    check_dependencies.check(data, announce=False)


if __name__ == "__main__":
    unittest.main()
