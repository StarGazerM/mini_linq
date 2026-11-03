"""Identity regressions use synthetic Cargo responses; never build a Rust crate."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

TOOLS = Path(__file__).resolve().parent
if str(TOOLS) not in sys.path:
    sys.path.insert(0, str(TOOLS))
SPEC = importlib.util.spec_from_file_location("regression_identity", TOOLS / "regression_identity.py")
identity = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(identity)


class CargoFixture:
    def __init__(self, root):
        self.root = root
        self.calls = []
        self.version = "rustc fixture 1"
        self.outputs = {}
        self.build_records = []
        self.environments = []
        self.packages = []
        self.units = []
        for name, kind, mode in (("app", "test", "test"), ("runtime", "lib", "build"),
                                 ("macros", "proc-macro", "build"), ("compiler", "lib", "build")):
            directory = root / name
            (directory / "src").mkdir(parents=True)
            (directory / "Cargo.toml").write_text(f'[package]\nname="{name}"\nversion="1.0.0"\n')
            source = directory / "src/lib.rs"
            source.write_text("pub fn pure() {}\n")
            target = {"name": name, "kind": [kind], "crate_types": [kind], "src_path": str(source),
                      "edition": "2024", "test": True, "doctest": False}
            self.packages.append({"id": name, "name": name, "version": "1", "source": None,
                                  "manifest_path": str(directory / "Cargo.toml"), "targets": [target]})
            self.units.append({"pkg_id": name, "target": target, "features": [], "platform": None,
                               "profile": {"opt_level": "0", "test": mode == "test"},
                               "mode": mode, "dependencies": []})
            self.outputs[name] = f"fn {name}() {{}}\n"
        self.units[0]["dependencies"] = [{"index": 1, "extern_crate_name": "runtime"},
                                         {"index": 2, "extern_crate_name": "macros"}]
        # Runtime library expansion is necessary too, not just the selected test.
        self.units[1]["dependencies"] = [{"index": 2, "extern_crate_name": "macros"}]
        self.units[2]["dependencies"] = [{"index": 3, "extern_crate_name": "compiler"}]
        self.roots = [0]
        self.mismatch = False
        self.on_expand = None
        self.row = ["app", "test", "app", "app/src/lib.rs"]
        (root / "Cargo.toml").write_text('[workspace]\nmembers=["app"]\n')
        (root / "Cargo.lock").write_text("lock fixture\n")

    def run(self, command, env):
        self.calls.append(command)
        self.environments.append(env.copy())
        if command[0] in {"rustc", "cargo", "rustfmt"} and command[1] in {"-vV", "--version"}:
            return self.version + command[0]
        if command[1] == "metadata":
            return json.dumps({"packages": self.packages, "workspace_default_members": ["app"],
                               "target_directory": str(self.root / "target")})
        if "--unit-graph" in command:
            roots = self.roots
            units = copy.deepcopy(self.units)
            if command[1] == "rustc":
                package = command[command.index("-p") + 1]
                roots = [next(i for i, unit in enumerate(units) if unit["pkg_id"] == package)]
                if self.mismatch:
                    units[roots[0]]["features"] = ["wrong-context"]
            return json.dumps({"version": 1, "units": units, "roots": roots})
        if command[1] == "rustc" and "-Zunpretty=expanded" in command:
            if self.on_expand:
                self.on_expand()
            return self.outputs[command[command.index("-p") + 1]] + "".join(
                json.dumps(record) + "\n" for record in self.build_records)
        raise AssertionError(f"unexpected command: {command}")

    def expansion_calls(self):
        return [command for command in self.calls if "-Zunpretty=expanded" in command]


class IdentityTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.fixture = CargoFixture(self.root)
        self.identity = identity.Identity(self.root)
        self.addCleanup(patch.stopall)
        patch.object(self.identity, "_run", side_effect=self.fixture.run).start()
        patch.object(self.identity, "_configs", return_value={}).start()
        patch.dict(os.environ, {}, clear=True).start()
        patch.object(identity.gate, "normalize", side_effect=lambda text, *args: text).start()

    def fingerprint(self, row=None):
        value = self.identity.fingerprint(row or self.fixture.row)
        self.assertIsNotNone(value, self.identity.last_reason)
        json.dumps(value)
        return value

    def test_compiler_only_identical_expansion_reuses_with_fresh_macro_output(self):
        before = self.fingerprint()
        self.assertEqual(len(self.fixture.expansion_calls()), 2)
        self.assertEqual(before, self.fingerprint())
        self.assertEqual(len(self.fixture.expansion_calls()), 4)
        (self.root / "compiler/src/lib.rs").write_text("pub fn rewritten_compiler() {}")
        self.assertEqual(before, self.fingerprint())
        self.assertEqual(len(self.fixture.expansion_calls()), 6)
        (self.root / "macros/src/lib.rs").write_text("pub fn rewritten_macro() {}")
        self.assertEqual(before, self.fingerprint())
        self.assertEqual(len(self.fixture.expansion_calls()), 8)

    def test_bounded_expansion_cache_rechecks_source_bytes(self):
        self.fixture.units = self.fixture.units[:2]
        self.fixture.units[0]["dependencies"] = [{"index": 1, "extern_crate_name": "runtime"}]
        self.fixture.units[1]["dependencies"] = []
        before = self.fingerprint()
        self.assertEqual(before, self.fingerprint())
        self.assertEqual(len(self.fixture.expansion_calls()), 1)
        (self.root / "app/src/lib.rs").write_text("pub fn changed() {}")
        self.assertNotEqual(before, self.fingerprint())
        self.assertEqual(len(self.fixture.expansion_calls()), 2)

    def test_macro_generated_dynamic_inputs_are_unproven(self):
        self.fixture.outputs["app"] = "fn test() { std::process::Command::new(\"cargo\"); }"
        self.assertIsNone(self.identity.fingerprint(self.fixture.row))

    def test_include_inputs_require_fresh_expansion_even_without_proc_macros(self):
        self.fixture.units = self.fixture.units[:2]
        self.fixture.units[0]["dependencies"] = [{"index": 1, "extern_crate_name": "runtime"}]
        self.fixture.units[1]["dependencies"] = []
        (self.root / "app/src/lib.rs").write_text('const INPUT: &str = include_str!("input.txt");')
        before = self.fingerprint()
        self.fixture.outputs["app"] = 'const INPUT: &str = "changed external input";'
        self.assertNotEqual(before, self.fingerprint())
        self.assertEqual(len(self.fixture.expansion_calls()), 2)

    def test_runtime_test_and_input_changes_veto_identical_expansion(self):
        before = self.fingerprint()
        for name in ("runtime/src/lib.rs", "app/src/lib.rs", "app/input.txt", "fixture.sql", "Cargo.lock"):
            path = self.root / name
            old = path.read_bytes() if path.exists() else None
            path.write_text("changed input")
            self.assertNotEqual(before, self.fingerprint(), name)
            if old is None:
                path.unlink()
            else:
                path.write_bytes(old)

    def test_runtime_library_expansion_change_vetoes_compiler_reuse(self):
        before = self.fingerprint()
        (self.root / "compiler/src/lib.rs").write_text("pub fn compiler_change() {}")
        self.fixture.outputs["runtime"] = "fn runtime() { different(); }"
        self.assertNotEqual(before, self.fingerprint())

    def test_features_toolchain_and_environment_invalidate(self):
        before = self.fingerprint()
        self.fixture.units[0]["features"] = ["extra"]
        self.assertNotEqual(before, self.fingerprint())
        self.fixture.units[0]["features"] = []
        self.fixture.version = "new compiler"
        self.assertNotEqual(before, self.fingerprint())
        self.fixture.version = "rustc fixture 1"
        os.environ["TEST_RUNTIME_INPUT"] = "changed"
        self.assertNotEqual(before, self.fingerprint())

    def test_regularly_linked_compiler_crate_is_a_runtime_input(self):
        self.fixture.units[0]["dependencies"].append({"index": 3, "extern_crate_name": "compiler"})
        before = self.fingerprint()
        (self.root / "compiler/src/lib.rs").write_text("pub fn compiler_change() {}")
        self.assertNotEqual(before, self.fingerprint())
        self.assertEqual(len(self.fixture.expansion_calls()), 4)

    def test_dev_dependency_runtime_unit_is_included(self):
        self.fixture.units[0]["dependencies"] = [{"index": 3, "extern_crate_name": "dev_helper"}]
        before = self.fingerprint()
        (self.root / "compiler/src/lib.rs").write_text("pub fn changed_dev_helper() {}")
        self.assertNotEqual(before, self.fingerprint())

    def test_unknown_scenario_and_unit_modes_are_not_reusable(self):
        self.assertIsNone(self.identity.fingerprint(["app", "scenario", "check", "app/src/lib.rs"]))
        self.assertEqual(self.fixture.calls, [])
        self.fixture.units[0]["mode"] = "doctest"
        self.assertIsNone(self.identity.fingerprint(self.fixture.row))

    def test_manifest_tests_decline_enabled_doctests_before_expanding(self):
        self.fixture.packages[0]["targets"][0]["doctest"] = True
        row = ["app", "manifest-test", "all", "app/Cargo.toml"]
        self.assertIsNone(self.identity.fingerprint(row))
        self.assertEqual(self.fixture.expansion_calls(), [])

    def test_manifest_doctest_check_ignores_unselected_workspace_siblings(self):
        self.fixture.packages[1]["targets"][0]["doctest"] = True
        row = ["app", "manifest-test", "all", "Cargo.toml"]
        self.fingerprint(row)
        self.assertTrue(self.fixture.expansion_calls())

    def test_oracle_metadata_qualifies_feature_for_root_manifest(self):
        self.identity._metadata(["app", "oracle", "golden", "Cargo.toml"], {})
        command = next(command for command in self.fixture.calls
                       if command[1] == "metadata")
        self.assertEqual(command[-2:], ["--features", "app/oracle"])

    def test_manifest_tests_cover_all_roots_not_inventory_label(self):
        self.fixture.units[1]["mode"] = "test"
        self.fixture.units[1]["target"]["kind"] = ["bin"]
        self.fixture.roots = [0, 1]
        row = ["app", "manifest-test", "not-a-target-selector", "app/Cargo.toml"]
        self.fingerprint(row)
        commands = self.fixture.expansion_calls()
        self.assertEqual(len(commands), 2)
        self.assertTrue(any("--bin" in command for command in commands))
        self.assertTrue(all(command[command.index("--profile") + 1] == "test" for command in commands))

    def test_harness_profile_and_features_follow_unit_graph(self):
        self.fixture.units[0]["features"] = ["oracle", "extra"]
        self.fingerprint()
        command = self.fixture.expansion_calls()[0]
        self.assertIn("--test", command)
        self.assertEqual(command[command.index("--profile") + 1], "test")
        self.assertIn("--all-features", command)
        self.assertEqual(command[-2:], ["--", "-Zunpretty=expanded"])

    def test_example_lib_and_run_selectors(self):
        for kind, selector, mode in (("example", "--example", "test"), ("lib", "--lib", "test"),
                                     ("manifest-run", "--bin", "build"), ("oracle", "--bin", "build")):
            with self.subTest(kind=kind):
                self.fixture.units[0]["target"]["kind"] = [{"manifest-run": "bin", "oracle": "bin"}.get(kind, kind)]
                self.fixture.units[0]["mode"] = mode
                row = ["app", kind, "golden", "app/Cargo.toml"]
                self.fingerprint(row)
                self.assertIn(selector, self.fixture.expansion_calls()[-2])
        self.assertEqual(identity._command(["app", "oracle", "golden", "app/Cargo.toml"])[-4:],
                         ["--features", "oracle", "--", "golden"])

    def test_dependency_graph_mismatch_fails_closed(self):
        self.fixture.mismatch = True
        self.assertIsNone(self.identity.fingerprint(self.fixture.row))
        self.assertEqual(self.fixture.expansion_calls(), [])

    def test_missing_runtime_build_records_and_dynamic_compilation_fail_closed(self):
        self.fixture.units[1]["target"]["kind"] = ["custom-build"]
        self.fixture.units[1]["mode"] = "run-custom-build"
        self.assertIsNone(self.identity.fingerprint(self.fixture.row))
        self.fixture.units[1]["target"]["kind"] = ["lib"]
        self.fixture.units[1]["mode"] = "build"
        (self.root / "app/src/lib.rs").write_text('fn test() { trybuild::TestCases::new(); }')
        self.assertIsNone(self.identity.fingerprint(self.fixture.row))

    def test_compiler_build_scripts_disable_expansion_cache(self):
        self.fixture.units[3]["target"]["kind"] = ["custom-build"]
        before = self.fingerprint()
        self.assertEqual(before, self.fingerprint())
        self.assertEqual(len(self.fixture.expansion_calls()), 4)

    def test_mid_expansion_change_cannot_become_passing_evidence(self):
        self.fixture.on_expand = lambda: (self.root / "app/src/lib.rs").write_text("changed during expansion")
        self.assertIsNone(self.identity.fingerprint(self.fixture.row))

    def test_failures_only_disable_identity(self):
        with patch.object(self.identity, "_run", side_effect=OSError("compiler missing")):
            self.assertIsNone(self.identity.fingerprint(self.fixture.row))

    def test_symlinked_sources_are_unproven(self):
        (self.root / "app/alias.rs").symlink_to(self.root / "app/src/lib.rs")
        self.assertIsNone(self.identity.fingerprint(self.fixture.row))

    def test_identity_excludes_documentation_only_changes(self):
        before = self.fingerprint()
        (self.root / "app/README.MD").write_text("documentation")
        self.assertEqual(before, self.fingerprint())

    def test_runtime_build_outputs_and_declared_inputs_are_hashed(self):
        unit = copy.deepcopy(self.fixture.units[1])
        unit["target"]["kind"] = ["custom-build"]
        unit["mode"] = "run-custom-build"
        unit["dependencies"] = []
        self.fixture.units.append(unit)
        self.fixture.units[1]["dependencies"].append({"index": 4, "extern_crate_name": "build_script_build"})
        output = self.root / "target/regression-identity/debug/build/runtime/out"
        output.mkdir(parents=True)
        (output.parent / "output").write_text("cargo:rerun-if-changed=input.txt\ncargo:rerun-if-env-changed=BUILD_MODE\n")
        (self.root / "runtime/input.txt").write_text("input")
        generated = output / "generated.rs"
        generated.write_text("pub const VALUE: i32 = 1;")
        record = {"reason": "build-script-executed", "package_id": "runtime", "out_dir": str(output),
                  "linked_libs": [], "linked_paths": [], "cfgs": ["runtime_cfg"], "env": []}
        self.fixture.build_records = [record]
        before = self.fingerprint()
        generated.write_text("pub const VALUE: i32 = 2;")
        self.assertNotEqual(before, self.fingerprint())
        generated.write_text("pub const VALUE: i32 = 1;")
        (self.root / "runtime/input.txt").write_text("different input")
        self.assertNotEqual(before, self.fingerprint())
        self.assertTrue(any(env.get("CARGO_TARGET_DIR") == str(self.root / "target/regression-identity")
                            for env in self.fixture.environments))
        record["linked_libs"] = ["static=unbounded_native"]
        self.assertIsNone(self.identity.fingerprint(self.fixture.row))

    def test_package_links_metadata_is_not_native_linkage(self):
        self.fixture.packages[1]["links"] = "rayon-core"
        self.fingerprint()

    def test_run_graph_uses_build_without_program_arguments(self):
        self.fixture.units[0]["mode"] = "build"
        self.fixture.units[0]["target"]["kind"] = ["bin"]
        self.fingerprint(["app", "oracle", "golden", "app/Cargo.toml"])
        graphs = [call for call in self.fixture.calls if "--unit-graph" in call]
        self.assertEqual(graphs[0][1], "build")
        self.assertNotIn("golden", graphs[0])
        self.assertIn("--features", graphs[0])

    def test_harness_identifiers_and_opaque_prose_are_not_runtime_calls(self):
        text = '''
#[rustc_test_marker = "checks::compile_fail"]
fn example() {
    let fs = 1;
    let net = 2;
    let rustc = 3;
    let text = "std::fs::read(compile_fail())";
    let raw = r###"Command::new(\\"cargo\\")"###;
    // trybuild::TestCases, std::process::Command
    assert_eq!(fs + net, rustc);
}
'''
        self.assertIsNone(identity.runtime_observer(text))

    def test_dynamic_paths_imports_and_calls_still_fail_closed(self):
        for text in (
            'std::process::Command::new("cargo")',
            'Command :: new("rustc")',
            'use std::{fs, net}; fs::read("input")',
            'use std::fs as files;',
            'trybuild::TestCases::new()',
            'use trybuild as checker;',
            'compile_fail!("input.rs");',
            'check_compile_fail("input.rs");',
            'extern "C" { fn call(); }',
            '#[link(name = "native")] extern "C" {}',
        ):
            with self.subTest(text=text):
                self.assertIsNotNone(identity.runtime_observer(text))

    def test_dev_test_profile_names_do_not_hide_real_profile_changes(self):
        graph = {"units": copy.deepcopy(self.fixture.units), "roots": [0]}
        before = self.identity._closure(graph, 0)
        graph["units"][1]["profile"]["name"] = "test"
        self.assertEqual(before, self.identity._closure(graph, 0))
        graph["units"][1]["profile"]["opt_level"] = "3"
        self.assertNotEqual(before, self.identity._closure(graph, 0))

    def test_comparable_paths_are_tree_relative(self):
        self.assertEqual(self.identity._portable({"path": str(self.root / "src/lib.rs")}),
                         {"path": "<TREE>/src/lib.rs"})


if __name__ == "__main__":
    unittest.main()
