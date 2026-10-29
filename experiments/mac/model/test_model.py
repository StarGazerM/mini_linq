import json
from pathlib import Path
import tempfile
import unittest

import analyze
import model


def hardware():
    # A cache boundary at 512 B: 1 ns for tiny probes, 9 ns outside it.
    records = []
    for proc, ops in model.OPS.items():
        for kind, (op, filters) in ops.items():
            for footprint in (8, 512):
                records.append(dict(op=op, processor=proc, threads=1, bytes=footprint,
                                    params=dict(filters, footprint_bytes=footprint),
                                    ns_per_item=9 if kind == "probe" and footprint == 512 else 1,
                                    gbps=1, median_ms=0.001))
    for dispatches in (1, 100):
        records.append(dict(op="dispatch_empty", processor="gpu", threads=1, bytes=1,
                            params=dict(dispatches=dispatches), ns_per_item=1,
                            gbps=1, median_ms=0.001))
    records.append(dict(op="sync_roundtrip", processor="gpu", threads=1, bytes=1,
                        params={}, ns_per_item=1, gbps=1, median_ms=0.001))
    return model.Hardware(records)


class WorkloadOrderingTests(unittest.TestCase):
    def setUp(self):
        self.hw = hardware()
        self.structure = model.Structure("sorted", 1, probe_proc=model.Proc("cpu"),
                                         maint_proc=model.Proc("cpu"))

    def simulate(self, rounds, *, overlap=False, base_rows=64):
        return model.simulate(self.hw, self.structure, model.Row(), rounds,
                              fanout=1, hit=0, overlap=overlap, base_rows=base_rows)

    def test_bulk_probes_populated_footprint(self):
        result = self.simulate(analyze.rounds_for(dict(bulk=(64, 1000))))
        self.assertAlmostEqual(result.probe_ms, 0.009)

    def test_seminaive_probes_previous_snapshot(self):
        result = self.simulate([model.Round(64, 1000), model.Round(64, 1000)])
        self.assertAlmostEqual(result.probe_ms, 0.010)

    def test_build_first_probes_new_live_runs(self):
        # 16 rows promote to level 1; the next 4 form a second live run at level 0.
        result = self.simulate([model.Round(16, 0, build_first=True),
                                model.Round(4, 1000, build_first=True)], base_rows=4)
        cost_per_line = self.hw.at("gather_line", "cpu", 1, "ns_per_item", 20 * 8)
        self.assertAlmostEqual(result.probe_ms, 2000 * cost_per_line / 1e6)

    def test_join_chain_resets_index_between_builds(self):
        chain = self.simulate(model.join_chain([(64, 1000), (1, 1000)]))
        first = self.simulate(model.join_chain([(64, 1000)]))
        second = self.simulate(model.join_chain([(1, 1000)]))
        self.assertAlmostEqual(chain.probe_ms, 0.010)
        self.assertAlmostEqual(chain.total, first.total + second.total)
        self.assertEqual(chain.bytes, first.bytes + second.bytes)

    def test_hybrid_cannot_overlap_dependent_build_and_probe(self):
        self.structure.maint_proc = model.Proc("gpu")
        dependent = model.join_chain([(64, 1000)])
        self.assertEqual(self.simulate(dependent, overlap=True), self.simulate(dependent))
        independent = [model.Round(64, 1000)]
        self.assertLess(self.simulate(independent, overlap=True).total,
                        self.simulate(independent).total)

    def test_trace_preserves_explicit_dependencies_and_legacy_order(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "trace.json"
            path.write_text(json.dumps([
                dict(delta=64, probes=1000, reset=True, build_first=True),
                dict(delta=1, probes=1000, reset=True, build_first=True),
            ]))
            self.assertAlmostEqual(self.simulate(model.datalog_trace(path, 2)).probe_ms, 0.010)
            path.write_text(json.dumps([dict(delta=64, probes=1000)]))
            self.assertAlmostEqual(self.simulate(model.datalog_trace(path, 2)).probe_ms, 0.001)


if __name__ == "__main__":
    unittest.main()
