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



class IoDependencies(unittest.TestCase):
    def packages(self, owner, target, kind=None, optional=False, rename=None, target_cfg=None,
                 local=True):
        dependency = {"name": "target", "kind": kind, "optional": optional,
                      "rename": rename, "target": target_cfg}
        if local:
            dependency["path"] = str(check_dependencies.ROOT / target)
        return {
            "owner": {"name": "owner",
                      "manifest_path": str(check_dependencies.ROOT / owner / "Cargo.toml"),
                      "dependencies": [dependency], "features": {}, "targets": []},
            "target": {"name": "target",
                       "manifest_path": str(check_dependencies.ROOT / target / "Cargo.toml"),
                       "dependencies": [], "features": {}, "targets": []},
        }

    def test_both_directions_reject_every_cargo_edge_form(self):
        for compiler in ("languages/new-language", "passes/new-pass", "facade/new-facade",
                         "facade/mini-linq-macros", "languages/nested/new-language"):
            for loader in ("io/cq-load", "io/cq-io-macros", "io/future/nested-loader"):
                for owner, target in ((compiler, loader), (loader, compiler)):
                    for kind in (None, "dev", "build"):
                        for optional in (False, True):
                            for rename in (None, "innocent_alias"):
                                for target_cfg in (None, "cfg(unix)"):
                                    for local in (False, True):
                                        with self.subTest(owner=owner, target=target, kind=kind,
                                                          optional=optional, rename=rename,
                                                          target_cfg=target_cfg, local=local), \
                                                self.assertRaisesRegex(ValueError, "compiler depends on IO|IO depends on compiler"):
                                            check_dependencies.check_io_boundaries(self.packages(
                                                owner, target, kind, optional, rename, target_cfg, local))

    def test_mini_linq_exception_is_one_way_and_exact(self):
        for kind in (None, "dev", "build"):
            check_dependencies.check_io_boundaries(
                self.packages("facade/mini-linq", "io/cq-load", kind))
            for facade in ("facade/mini-linq", "facade/mini-linq/nested", "facade/mini-linq-other"):
                with self.subTest(facade=facade, kind=kind), \
                        self.assertRaisesRegex(ValueError, "IO depends on compiler"):
                    check_dependencies.check_io_boundaries(self.packages("io/cq-load", facade, kind))
            for facade in ("facade/mini-linq/nested", "facade/mini-linq-other"):
                with self.subTest(facade=facade, kind=kind), \
                        self.assertRaisesRegex(ValueError, "compiler depends on IO"):
                    check_dependencies.check_io_boundaries(self.packages(facade, "io/cq-load", kind))

    def test_dependency_paths_cannot_hide_behind_unknown_package_names(self):
        for owner, target in (("passes/new-pass", "io/new-loader"),
                              ("io/new-loader", "languages/new-language")):
            packages = self.packages(owner, target, "dev")
            del packages["target"]
            with self.subTest(owner=owner), self.assertRaises(ValueError):
                check_dependencies.check_io_boundaries(packages)

    def test_neutral_runtime_and_feature_local_compiler_folders_are_permitted(self):
        for owner, target in (
            ("io/cq-load", "runtime/cq-evidence"),
            ("languages/cq-syntax", "runtime/cq-evidence"),
            ("io/cq-load", "runtime/cq-storage"),
            ("io/cq-load", "io/cq-io-macros"),
            ("features/datalog/languages/source", "io/cq-load"),
            ("features/datalog/passes/print", "io/cq-load"),
            ("features/datalog/facade/query", "io/cq-load"),
        ):
            with self.subTest(owner=owner, target=target):
                check_dependencies.check_io_boundaries(self.packages(owner, target))

    def test_main_checker_enforces_boundary_before_dev_allowances(self):
        packages = self.packages("facade/mini-linq-macros", "io/cq-load", "dev")
        with patch.object(check_dependencies, "PATHS", {}), \
                self.assertRaisesRegex(ValueError, "compiler depends on IO"):
            check_dependencies.check({"packages": list(packages.values())}, announce=False)

    def test_main_checker_accepts_user_facade_io_dependency(self):
        for kind in (None, "dev", "build"):
            packages = self.packages("facade/mini-linq", "io/cq-load", kind)
            with self.subTest(kind=kind), \
                    patch.object(check_dependencies, "PATHS", {"owner": Path("facade/mini-linq")}), \
                    patch.object(check_dependencies, "FACADES", {"owner": Path("facade/mini-linq")}), \
                    patch.object(check_dependencies, "IO", {"target": Path("io/cq-load")}):
                check_dependencies.check({"packages": list(packages.values())}, announce=False)

    def test_main_checker_accepts_io_and_neutral_runtime_dependencies(self):
        for target in ("io/cq-io-macros", "runtime/cq-evidence", "runtime/cq-storage"):
            for kind in (None, "dev", "build"):
                packages = self.packages("io/cq-load", target, kind)
                io_paths = {"owner": Path("io/cq-load")}
                runtimes = {}
                (io_paths if target.startswith("io/") else runtimes)["target"] = Path(target)
                with self.subTest(target=target, kind=kind), \
                        patch.object(check_dependencies, "PATHS", {"owner": Path("io/cq-load")}), \
                        patch.object(check_dependencies, "FACADES", {}), \
                        patch.object(check_dependencies, "IO", io_paths), \
                        patch.object(check_dependencies, "RUNTIME", runtimes):
                    check_dependencies.check({"packages": list(packages.values())}, announce=False)

    def test_runtime_io_is_only_a_development_dependency(self):
        for kind in (None, "dev", "build"):
            packages = self.packages("runtime/cq-storage", "io/cq-load", kind)
            with self.subTest(kind=kind), \
                    patch.object(check_dependencies, "PATHS", {"owner": Path("runtime/cq-storage")}), \
                    patch.object(check_dependencies, "FACADES", {}), \
                    patch.object(check_dependencies, "RUNTIME", {"owner": Path("runtime/cq-storage")}), \
                    patch.object(check_dependencies, "IO", {"target": Path("io/cq-load")}):
                if kind == "dev":
                    check_dependencies.check({"packages": list(packages.values())}, announce=False)
                else:
                    with self.assertRaises(ValueError):
                        check_dependencies.check({"packages": list(packages.values())}, announce=False)


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
