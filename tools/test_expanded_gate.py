#!/usr/bin/env python3
"""Unit regressions for the content-addressed expanded-Rust gate."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("expanded_gate", Path(__file__).with_name("expanded_gate.py"))
gate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(gate)


class ExpandedGateTests(unittest.TestCase):
    def test_normalization_preserves_literals_and_comments(self):
        source = '''
macro_rules! unused { () => {{ let __cq_8_2 = 7; }}; }
fn kernel() {
    let __cq_5_0 = r###"macro_rules! fake { __cq_7_3 }"###;
    let __cq_5_1 = "__cq_5_0";
    // __cq_5_0 is documentation, not an identifier.
    let _ = (__cq_5_0, __cq_5_1, '{', b'}');
}
'''
        result = gate.normalize(source, gate.ROOT, gate.ROOT / "target/harness", "rustfmt")
        self.assertNotIn("macro_rules! unused", result)
        self.assertIn('r###"macro_rules! fake { __cq_7_3 }"###', result)
        self.assertIn('"__cq_5_0"', result)
        self.assertIn("// __cq_5_0", result)
        self.assertIn("let __cq_generated_0", result)
        self.assertIn("let __cq_generated_1", result)
        self.assertIn("(__cq_generated_0, __cq_generated_1", result)

    def test_nested_comments_and_raw_strings_do_not_close_module(self):
        text = 'pub mod query { /* } /* { */ } */ fn f() { let s=r#"}"#; } } mod other {}'
        result = gate.extract_module(text, "query")
        self.assertTrue(result.startswith("pub mod query"))
        self.assertNotIn("mod other", result)
        self.assertIn('r#"}"#', result)
        with self.assertRaises(ValueError):
            gate.extract_module(text, "absent")

    def test_hunks_require_exact_review_and_report_every_change(self):
        before = "fn f() { old(); }\n"
        after = "fn f() { new(); }\n"
        _, changes = gate.hunks(before, after, "query", [])
        self.assertEqual(len(changes), 1)
        self.assertTrue(changes[0]["unexplained"])
        rule = {"target": "quer*", "class": "e", "hunk_sha256": changes[0]["sha256"],
                "source": "runtime/source.rs:1", "reason": "Reviewed dispatch"}
        _, matched = gate.hunks(before, after, "query", [rule])
        self.assertEqual(matched[0]["classes"], ["e"])
        self.assertFalse(matched[0]["unexplained"])
        _, different = gate.hunks(before, "fn f() { other(); }\n", "query", [rule])
        self.assertTrue(different[0]["unexplained"])
        _, wrong_target = gate.hunks(before, after, "unrelated", [rule])
        self.assertTrue(wrong_target[0]["unexplained"])

    def test_class_labels_are_not_blanket_approvals(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "rules.json"
            path.write_text(json.dumps({"version": 1, "rules": [{"target": "*", "class": "e"}]}))
            with self.assertRaises(ValueError):
                gate.read_rules(path)

    def test_named_intended_classes_and_snapshot_only_classification(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _, changes = gate.hunks("fn old() {}\n", "fn new() {}\n", "query", [])
            rule = {"target": "query", "class": "m", "hunk_sha256": changes[0]["sha256"],
                    "reason": "Reviewed staging change", "source": "source.rs:1"}
            classes = root / "classes.json"
            classes.write_text(json.dumps({"version": 1,
                "definitions": {"m": {"kind": "intended", "description": "Deferred staging"}},
                "rules": [rule]}))
            rules = gate.read_rules(classes)
            self.assertEqual(rules[0]["kind"], "intended")
            path = root / "report.json"
            path.write_text(json.dumps({"version": 1, "targets": [
                {"target": "query", "status": "differs", "hunks": changes,
                 "unexplained": 1, "verification": "required"},
                {"target": "runtime_only", "status": "runtime-changed", "hunks": [],
                 "unexplained": 0, "verification": "required"},
            ]}))
            with patch.object(gate, "run", side_effect=AssertionError("no subprocesses")), patch("builtins.print"):
                self.assertEqual(gate.reclassify_report(path, rules, root), 2)
            report = json.loads(path.read_text())
            self.assertTrue(report["snapshot_only"])
            intended, runtime_only = report["targets"]
            self.assertEqual(intended["classification"], "intended")
            self.assertFalse(intended["passed"])
            self.assertEqual(runtime_only["classification"], "refactor-only")
            self.assertTrue(runtime_only["execution_evidence_pending"])
            classes.write_text(json.dumps({"version": 1, "rules": [],
                "definitions": {"a": {"kind": "intended", "description": "Invalid override"}}}))
            with self.assertRaises(ValueError):
                gate.read_rules(classes)

    def test_runtime_hash_ignores_build_artifacts_and_changes_on_source(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp)
            (path / "src").mkdir()
            source = path / "src/lib.rs"
            source.write_text("pub fn f() {}")
            before = gate.crate_digest(path)
            (path / "target").mkdir()
            (path / "target/cache").write_text("ignored")
            self.assertEqual(gate.crate_digest(path), before)
            source.write_text("pub fn f() { work(); }")
            self.assertNotEqual(gate.crate_digest(path), before)

    def test_each_identity_input_invalidates_key(self):
        values = {"expansion": "a", "runtimes": {"cq-storage": "b"}, "rustc": "v",
                  "flags": ["-Zunpretty=expanded"], "data_oracle": "c"}
        before = gate.cache_key(values)
        for field in values:
            changed = dict(values)
            changed[field] = "changed"
            self.assertNotEqual(gate.cache_key(changed), before, field)
        self.assertEqual(gate.cache_key(dict(reversed(list(values.items())))), before)

    def test_verified_cache_requires_outcomes_and_intact_evidence(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            evidence = root / "run.json"
            evidence.write_text('{"verified":true,"rows_tested":42}')
            outcome = {"exact": True, "counters": {"passed": True, "values": {"rows_tested": 42}},
                       "scope": "default", "evidence": ["run.json"]}
            cache = root / "cache"
            key = "a" * 64
            with self.assertRaises(ValueError):
                gate.record_verified(cache, "query", key, {"exact": True}, root)
            gate.record_verified(cache, "query", key, outcome, root)
            record = gate.verified_record(cache, key)
            self.assertEqual(record["outcome"]["scope"], "default")
            self.assertIsNone(gate.verified_record(cache, "b" * 64))
            (cache / record["evidence"][0]["file"]).write_text("tampered")
            self.assertIsNone(gate.verified_record(cache, key))

    def test_library_host_support_is_part_of_data_identity(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, baseline = Path(tmp) / "working", Path(tmp) / "baseline"
            for tree in (root, baseline):
                source = tree / "features/workloads/src/extended.rs"
                source.parent.mkdir(parents=True)
                source.write_text("fn inputs() {}")
            current, _ = gate.data_digest(root, "correlated_parameter", [], root)
            before, _ = gate.data_digest(root, "correlated_parameter", [], baseline)
            self.assertEqual(current, before)
            (baseline / "features/workloads/src/extended.rs").write_text("fn inputs() { changed(); }")
            self.assertNotEqual(current, gate.data_digest(root, "correlated_parameter", [], baseline)[0])

    def test_catalog_never_silently_drops_an_unrecognized_entry(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "features/workloads/src/tpch.rs"
            source.parent.mkdir(parents=True)
            source.write_text('struct Case {} const CASES: &[Case] = &[Case { name: "a", rust_path: "workloads/a.rs" }];')
            self.assertEqual(gate.catalog(root), {"a": "features/workloads/a.rs"})
            source.write_text('const CASES: &[Case] = &[Case { name: "a", rust_path: "workloads/a.rs" }, Case { name: "b", /* reviewed */ rust_path: "workloads/b.rs" }];')
            with self.assertRaises(ValueError):
                gate.catalog(root)

    def test_unrelated_standalone_edits_do_not_invalidate_selected_worker(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            paths = {"retail_h07": "features/workloads/src/queries/retail/tpch/retail_h07.rs",
                     "retail_h13": "features/workloads/src/queries/retail/tpch/retail_h13.rs"}
            helper = "features/workloads/src/tpch_runtime.rs"
            for name in [*paths.values(), helper]:
                file = root / name
                file.parent.mkdir(parents=True, exist_ok=True)
                file.write_text("before")
            listing = ("\0".join([*paths.values(), helper]) + "\0").encode()
            with patch.object(gate, "run", return_value=listing):
                before = gate.source_snapshot(root, root / "target", "retail_h07", paths)
                (root / paths["retail_h13"]).write_text("unrelated")
                self.assertEqual(before, gate.source_snapshot(root, root / "target", "retail_h07", paths))
                (root / helper).write_text("shared change")
                self.assertNotEqual(before, gate.source_snapshot(root, root / "target", "retail_h07", paths))

    def test_runtime_graph_excludes_unreachable_gpu_packages(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            names = ["query", "cq-storage", "cq-load", "cq-gpu-runtime"]
            packages = []
            for name in names:
                path = root / name
                path.mkdir()
                (path / "Cargo.toml").write_text(name)
                packages.append({"id": name, "name": name, "version": "1", "source": None,
                                 "manifest_path": str(path / "Cargo.toml")})
            nodes = [{"id": name, "features": [], "dependencies": ["cq-storage", "cq-load"] if name == "query" else []}
                     for name in names]
            meta = {"packages": packages, "resolve": {"root": "query", "nodes": nodes}}
            runtimes, _ = gate.runtime_sources(meta)
            self.assertEqual(set(runtimes), {"cq-storage", "cq-load"})
            nodes[0]["dependencies"].append("cq-gpu-runtime")
            self.assertIn("cq-gpu-runtime", gate.runtime_sources(meta)[0])
            packages[-1]["targets"] = [{"kind": ["proc-macro"]}]
            self.assertNotIn("cq-gpu-runtime", gate.runtime_sources(meta)[0])
            packages[-1]["targets"] = [{"kind": ["lib"]}]
            nodes[0]["deps"] = [
                {"pkg": "cq-storage", "dep_kinds": [{"kind": None}]},
                {"pkg": "cq-load", "dep_kinds": [{"kind": None}]},
                {"pkg": "cq-gpu-runtime", "dep_kinds": [{"kind": "build"}]},
            ]
            self.assertNotIn("cq-gpu-runtime", gate.runtime_sources(meta)[0])

    def test_worktree_removed_when_body_raises(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            calls = []
            def command(argv, cwd, env=None):
                calls.append(argv)
                if "add" in argv:
                    tree = Path(argv[-2])
                    tree.mkdir()
                    (tree / ".git").write_text("test")
                return b""
            with patch.object(gate, "run", command):
                with self.assertRaisesRegex(RuntimeError, "body failure"):
                    with gate.baseline_tree(root, "HEAD"):
                        raise RuntimeError("body failure")
            self.assertIn("remove", calls[-1])


if __name__ == "__main__":
    unittest.main()
