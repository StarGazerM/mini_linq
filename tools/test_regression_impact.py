#!/usr/bin/env python3
"""Impact contracts using synthetic metadata and Git output, never Cargo builds."""
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from regression_impact import Impact


class ImpactTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.files = {}
        self.historical = {}
        self.changed = []
        self.untracked = []
        self.commands = []
        self.fail_contexts = set()
        self.put("Cargo.toml", "[workspace]\nmembers = []\n")
        for name, directory in {
            "a": "crates/a", "b": "crates/b", "dev": "crates/dev",
            "build": "crates/build", "runtime": "runtime/shared",
            "feature": "features/one", "unrelated": "features/two",
            "inspection": "features/inspection",
        }.items():
            self.put(f"{directory}/Cargo.toml", f'[package]\nname = "{name}"\nversion = "0.1.0"\n')
        for directory in ("features/one", "features/two", "features/inspection"):
            self.put(f"{directory}/Cargo.toml", self.files[f"{directory}/Cargo.toml"] + "\n[workspace]\n")
        self.packages = {
            name: self.package(name, directory) for name, directory in {
                "a": "crates/a", "b": "crates/b", "dev": "crates/dev",
                "build": "crates/build", "runtime": "runtime/shared",
                "feature": "features/one", "unrelated": "features/two",
                "inspection": "features/inspection",
            }.items()
        }
        edges = {"a": ["dev", "build", "runtime"], "dev": ["b"], "feature": ["runtime"],
                 "inspection": ["runtime"]}
        self.metadata = {
            "Cargo.toml": self.graph(".", ["a", "b", "dev", "build", "runtime"], edges),
            "features/one/Cargo.toml": self.graph("features/one", ["feature"], edges),
            "features/two/Cargo.toml": self.graph("features/two", ["unrelated"], edges),
            "features/inspection/Cargo.toml": self.graph("features/inspection", ["inspection"], edges),
        }
        self.a = ["a", "test", "laws", "crates/a/tests/laws.rs"]
        self.b = ["b", "lib", "unit", "crates/b/src/lib.rs"]
        self.feature = ["feature", "manifest-test", "all", "features/one/Cargo.toml"]
        self.unrelated = ["unrelated", "manifest-test", "all", "features/two/Cargo.toml"]
        self.architecture = ["architecture", "scenario", "deps", "tools/check_dependencies.py"]
        self.public_api = ["architecture", "scenario", "api", "tools/test_public_api.py"]
        self.scenario = ["inspection", "scenario", "inspect", "features/inspection/cq-inspection/check.py"]
        self.rows = [self.a, self.b, self.feature, self.unrelated, self.architecture, self.public_api, self.scenario]
        self.addCleanup(patch.stopall)
        patch("regression_impact.subprocess.run", side_effect=self.run_command).start()

    def put(self, path, content):
        self.files[path] = content
        destination = self.root / path
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(content)

    def package(self, name, directory):
        return {"id": name, "name": name, "source": None,
                "manifest_path": str(self.root / directory / "Cargo.toml"),
                "targets": [{"name": "cq-inspection", "kind": ["example"]}] if name == "inspection" else []}

    def graph(self, workspace, members, edges):
        selected = set(members)
        pending = list(members)
        while pending:
            for dependency in edges.get(pending.pop(), []):
                if dependency not in selected:
                    selected.add(dependency)
                    pending.append(dependency)
        return {
            "workspace_root": str(self.root / workspace), "workspace_members": members,
            "workspace_default_members": members,
            "packages": [self.packages[name] for name in sorted(selected)],
            "resolve": {"nodes": [
                {"id": name, "dependencies": edges.get(name, []), "deps": [
                    {"pkg": dependency, "dep_kinds": [{"kind": "dev" if dependency == "dev" else "build",
                                                        "target": "cfg(windows)"}]}
                    for dependency in edges.get(name, [])]}
                for name in sorted(selected)]},
        }

    def run_command(self, command, **kwargs):
        self.commands.append((command, kwargs))
        if command[0] == "cargo":
            manifest = Path(command[command.index("--manifest-path") + 1]).relative_to(self.root).as_posix()
            if manifest in self.fail_contexts:
                return subprocess.CompletedProcess(command, 101, "", "offline dependency unavailable")
            return subprocess.CompletedProcess(command, 0, json.dumps(self.metadata[manifest]), "")
        if command[1] == "diff":
            output = "\0".join(self.changed)
        elif command[1] == "ls-files":
            output = "\0".join(self.files if "--cached" in command else self.untracked)
        elif command[1] == "ls-tree":
            output = "\0".join(self.historical)
        elif command[1] == "show":
            path = command[2].split(":", 1)[1]
            if path not in self.historical:
                return subprocess.CompletedProcess(command, 128, "", "path absent in historical tree")
            output = self.historical[path]
        else:
            self.fail(f"Unexpected command: {command}")
        return subprocess.CompletedProcess(command, 0, output, "")

    def impact(self, changed, until=None):
        self.changed = changed
        return Impact(self.root, self.rows, "base", until)

    def test_documentation_only_selects_nothing_and_needs_no_metadata(self):
        impact = self.impact(["doc/design.rs", "README.md", "crates/a/notes.md", "CHANGELOG.old"])
        self.assertTrue(all(not impact.reasons(row) for row in self.rows))
        self.assertFalse(any(command[0] == "cargo" for command, _ in self.commands))

    def test_direct_source_selects_and_unrelated_workspace_skips(self):
        impact = self.impact([self.a[3]])
        self.assertTrue(impact.reasons(self.a))
        self.assertEqual(impact.reasons(self.unrelated), [])
        self.assertEqual(impact.reasons(self.feature), [])

    def test_transitive_dev_and_build_dependencies_select_consumers(self):
        for source in ("crates/b/src/lib.rs", "crates/dev/src/lib.rs", "crates/build/build.rs"):
            with self.subTest(source=source):
                impact = self.impact([source])
                self.assertTrue(impact.reasons(self.a))
                self.assertEqual(impact.reasons(self.feature), [])

    def test_shared_runtime_selects_each_dependent_workspace(self):
        impact = self.impact(["runtime/shared/src/lib.rs"])
        for row in (self.a, self.feature, self.scenario):
            self.assertTrue(impact.reasons(row))
        for row in (self.b, self.unrelated):
            self.assertEqual(impact.reasons(row), [])

    def test_unknown_paths_and_configuration_select_everything(self):
        for path in ("unknown/new.rs", "tools/build.py", ".cargo/config.toml", "test-data/input.csv", "Cargo.lock"):
            with self.subTest(path=path):
                impact = self.impact([path])
                self.assertTrue(all(impact.reasons(row) for row in self.rows))

    def test_metadata_failure_selects_context_even_for_unrelated_known_crate(self):
        self.fail_contexts.add("features/two/Cargo.toml")
        self.put("features/two/Cargo.lock", 'version = 4\n[[package]]\nname = "unrelated"\nversion = "0.1.0"\n')
        impact = self.impact(["crates/a/src/lib.rs"])
        self.assertTrue(impact.reasons(self.unrelated))
        self.assertEqual(impact.reasons(self.feature), [])
        context = next(context for context in impact.describe()["contexts"] if context["manifest"] == self.unrelated[3])
        self.assertEqual(context["lockfile_local_packages"], ["features/two"])

    def test_deleted_and_renamed_files_preserve_old_and_new_owners(self):
        # These paths intentionally do not exist on disk. --no-renames produces
        # both sides of a rename instead of losing the old crate's dependency.
        impact = self.impact(["crates/a/src/deleted.rs", "crates/b/src/old.rs", "features/one/src/new.rs"])
        self.assertTrue(impact.reasons(self.a))
        self.assertTrue(impact.reasons(self.b))
        self.assertTrue(impact.reasons(self.feature))
        self.assertEqual(impact.reasons(self.unrelated), [])
        diff = next(command for command, _ in self.commands if command[1] == "diff")
        self.assertIn("--no-renames", diff)
        self.assertIn("-z", diff)

    def test_deleted_crate_manifest_fails_closed(self):
        impact = self.impact(["gone/Cargo.toml", "gone/src/lib.rs"])
        self.assertTrue(all(impact.reasons(row) for row in self.rows))

    def test_untracked_files_are_added_to_working_tree_diff(self):
        self.untracked = ["features/one/src/new.rs", "notes.md"]
        impact = self.impact(["crates/a/src/lib.rs"])
        self.assertEqual(impact.changed_files, sorted(self.changed + self.untracked))
        self.assertTrue(impact.reasons(self.feature))
        listing = [command for command, _ in self.commands if command[1] == "ls-files"]
        self.assertTrue(any("--exclude-standard" in command and "--cached" not in command for command in listing))

    def test_workspace_split_gate_runs_on_root_manifest_changes(self):
        row = ["architecture", "scenario", "workspace-split", "tools/test_check_dependencies.py"]
        self.rows.append(row)
        self.assertTrue(self.impact(["Cargo.toml"]).reasons(row))

    def test_architecture_and_unknown_scenarios_are_repository_wide(self):
        unknown = ["custom", "scenario", "unknown", "features/custom/check.py"]
        self.rows.append(unknown)
        impact = self.impact(["features/two/src/lib.rs"])
        for row in (self.architecture, self.public_api, unknown):
            self.assertTrue(impact.reasons(row))
        self.assertEqual(impact.reasons(self.scenario), [])

    def test_documented_scenario_uses_workspace_and_discovery_cwd(self):
        impact = self.impact(["features/inspection/data/expected.txt"])
        self.assertTrue(impact.reasons(self.scenario))
        self.assertEqual(impact.reasons(self.feature), [])
        commands = [(command, kwargs) for command, kwargs in self.commands if command[0] == "cargo"]
        self.assertTrue(any(kwargs["cwd"] == self.root / "features/inspection" for _, kwargs in commands))
        for command, _ in commands:
            self.assertIn("--locked", command)
            self.assertIn("--offline", command)
            self.assertIn("--all-features", command)
            self.assertNotIn("--no-deps", command)

    def historical_snapshot(self):
        self.historical = dict(self.files)
        for workspace, names in {
            ".": ["a", "b", "dev", "build", "runtime"],
            "features/one": ["feature", "runtime"], "features/two": ["unrelated"],
            "features/inspection": ["inspection", "runtime"],
        }.items():
            path = str(Path(workspace) / "Cargo.lock")
            self.historical[path] = "version = 4\n" + "".join(
                f'[[package]]\nname = "{name}"\nversion = "0.1.0"\n' for name in names)

    def test_historical_range_excludes_untracked_and_uses_exact_endpoints(self):
        self.historical_snapshot()
        self.untracked = ["unknown.rs"]
        impact = self.impact(["features/one/src/lib.rs"], until="end")
        self.assertEqual(impact.changed_files, ["features/one/src/lib.rs"])
        self.assertTrue(impact.reasons(self.feature))
        self.assertEqual(impact.reasons(self.unrelated), [])
        diff = next(command for command, _ in self.commands if command[1] == "diff")
        self.assertEqual(diff[-3:], ["base", "end", "--"])

    def test_historical_lock_preserves_removed_local_dependency(self):
        self.historical_snapshot()
        self.historical["features/one/Cargo.lock"] = (
            'version = 4\n[[package]]\nname = "feature"\nversion = "0.1.0"\n'
            'dependencies = ["b 0.1.0"]\n'
            '[[package]]\nname = "b"\nversion = "0.1.0"\n'
        )
        impact = self.impact(["crates/b/src/old.rs"], until="end")
        self.assertTrue(impact.reasons(self.feature))
        self.assertEqual(impact.reasons(self.unrelated), [])

    def test_missing_historical_lock_is_not_a_proof_of_independence(self):
        self.historical_snapshot()
        del self.historical["features/two/Cargo.lock"]
        impact = self.impact(["crates/a/src/lib.rs"], until="end")
        self.assertTrue(impact.reasons(self.unrelated))

    def test_incomplete_metadata_and_change_detection_fail_closed(self):
        self.metadata["features/two/Cargo.toml"]["resolve"] = None
        impact = self.impact(["crates/a/src/lib.rs"])
        self.assertTrue(impact.reasons(self.unrelated))
        with patch("regression_impact.subprocess.run", side_effect=OSError("git unavailable")):
            impact = Impact(self.root, self.rows, "base")
        self.assertTrue(all(impact.reasons(row) for row in self.rows))

    def test_historical_ancestor_lock_cannot_cover_excluded_package(self):
        self.historical_snapshot()
        self.historical["features/two/Cargo.toml"] = (
            '[package]\nname = "unrelated"\nversion = "0.1.0"\n'
        )
        impact = self.impact(["crates/a/src/lib.rs"], until="end")
        self.assertTrue(impact.reasons(self.unrelated))

    def test_documentation_names_do_not_hide_rust_modules(self):
        for path in ("crates/a/src/doc/mod.rs", "crates/a/src/changelog.rs"):
            with self.subTest(path=path):
                impact = self.impact([path])
                self.assertTrue(impact.reasons(self.a))

    def test_demonstration_ranges_preserve_revision_expressions(self):
        self.historical_snapshot()
        self.changed = ["features/one/src/lib.rs"]
        for since, until in (
            ("17a14b93", "c82e9abf"),
            ("c82e9abf", "6b7b1097"),
            ("c0f79500^", "c0f79500"),
        ):
            with self.subTest(since=since, until=until):
                self.commands.clear()
                impact = Impact(self.root, self.rows, since, until)
                self.assertTrue(impact.reasons(self.feature))
                self.assertEqual(impact.reasons(self.unrelated), [])
                diff = next(command for command, _ in self.commands if command[1] == "diff")
                self.assertEqual(diff[-3:], [since, until, "--"])

    def gpu_workspace(self):
        self.put("features/gpu/Cargo.toml", '[workspace]\nmembers = ["datalog", "tpch"]\n')
        for name, directory, target in (
            ("gpu-datalog", "features/gpu/datalog", "datalog"),
            ("gpu-tpch", "features/gpu/tpch", "tpch"),
        ):
            self.put(f"{directory}/Cargo.toml", f'[package]\nname = "{name}"\nversion = "0.1.0"\n')
            self.packages[name] = self.package(name, directory)
            self.packages[name]["targets"] = [{"name": target, "kind": ["test"]}]
        graph = self.graph("features/gpu", ["gpu-datalog", "gpu-tpch"],
                           {"gpu-datalog": ["feature"], "gpu-tpch": ["runtime"]})
        self.metadata["features/gpu/Cargo.toml"] = graph
        datalog = ["gpu-datalog", "manifest-target-test", "datalog", "features/gpu/Cargo.toml"]
        tpch = ["gpu-tpch", "manifest-target-test", "tpch", "features/gpu/Cargo.toml"]
        self.rows.extend([datalog, tpch])
        return datalog, tpch

    def test_matching_target_not_sibling_workspace_members_controls_closure(self):
        datalog, tpch = self.gpu_workspace()
        impact = self.impact(["features/one/src/lib.rs"])
        self.assertTrue(impact.reasons(datalog))
        self.assertEqual(impact.reasons(tpch), [])

    def test_feature_target_uses_explicit_package_and_test(self):
        self.gpu_workspace()
        row = ["gpu-datalog", "manifest-feature-test", "datalog@all-features",
               "features/gpu/Cargo.toml"]
        self.rows.append(row)
        impact = self.impact(["features/one/src/lib.rs"])
        self.assertTrue(impact.reasons(row))
        impact = self.impact(["runtime/shared/src/lib.rs"])
        self.assertEqual(impact.reasons(row), [])

    def test_historical_lock_walk_excludes_unrelated_sibling_packages(self):
        datalog, tpch = self.gpu_workspace()
        self.historical_snapshot()
        self.historical["features/gpu/Cargo.lock"] = (
            'version = 4\n[[package]]\nname = "gpu-datalog"\nversion = "0.1.0"\n'
            'dependencies = ["feature"]\n'
            '[[package]]\nname = "gpu-tpch"\nversion = "0.1.0"\n'
            'dependencies = ["runtime"]\n'
            '[[package]]\nname = "feature"\nversion = "0.1.0"\n'
            '[[package]]\nname = "runtime"\nversion = "0.1.0"\n'
        )
        impact = self.impact(["features/one/src/lib.rs"], until="end")
        self.assertTrue(impact.reasons(datalog))
        self.assertEqual(impact.reasons(tpch), [])

    def test_manifest_test_selects_package_not_workspace_defaults(self):
        self.gpu_workspace()
        row = ["gpu-tpch", "manifest-test", "all", "features/gpu/Cargo.toml"]
        self.rows.append(row)
        impact = self.impact(["features/one/src/lib.rs"])
        self.assertEqual(impact.reasons(row), [])
        impact = self.impact(["runtime/shared/src/lib.rs"])
        self.assertTrue(impact.reasons(row))

    def test_manifest_run_uses_binary_owner_not_library_only_sibling(self):
        self.gpu_workspace()
        self.packages["gpu-tpch"]["targets"].append({"name": "tpch", "kind": ["bin"]})
        row = ["gpu-tpch", "manifest-run", "tpch", "features/gpu/Cargo.toml"]
        self.rows.append(row)
        impact = self.impact(["features/one/src/lib.rs"])
        self.assertEqual(impact.reasons(row), [])
        impact = self.impact(["runtime/shared/src/lib.rs"])
        self.assertTrue(impact.reasons(row))

    def test_submanifest_uses_resolve_root_not_all_workspace_members(self):
        _, tpch = self.gpu_workspace()
        graph = dict(self.metadata["features/gpu/Cargo.toml"])
        graph["resolve"] = dict(graph["resolve"], root="gpu-tpch")
        self.metadata["features/gpu/tpch/Cargo.toml"] = graph
        row = [tpch[0], "manifest-test", "all", "features/gpu/tpch/Cargo.toml"]
        self.rows.append(row)
        impact = self.impact(["features/one/src/lib.rs"])
        self.assertEqual(impact.reasons(row), [])
        del graph["workspace_default_members"]
        impact = self.impact(["features/one/src/lib.rs"])
        self.assertEqual(impact.reasons(row), [])


if __name__ == "__main__":
    unittest.main()
