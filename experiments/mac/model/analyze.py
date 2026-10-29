#!/usr/bin/env python3
"""Run the cost model over a scenario grid and report the best structure per placement.

    python3 analyze.py ../calib/results/{cpu,gpu,hybrid}-2026-10-23.json
"""
import json
import sys
from pathlib import Path

import model

SCENARIOS = {
    # name: (workload kwargs, row)
    "bulk build 15M + 60M probes": (dict(bulk=(15_000_000, 60_000_000)), model.Row(1, 1, 4)),
    "bulk-ish datalog 37M rows, 33 rounds": (dict(n=50_000_000, delta0=15_000_000, decay=0.6, rules=2), model.Row(1, 1, 4)),
    "  + 800-round tail x1000": (dict(n=50_000_000, delta0=15_000_000, decay=0.6, rules=2, tail=(800, 1000)), model.Row(1, 1, 4)),
    "pure tail 10M + 2000x2000, 3 probes": (dict(n=10_000_000, delta0=10_000_000, decay=0.1, rules=3, tail=(2000, 2000)), model.Row(1, 1, 4)),
    "wide 56B rows, bulk-ish": (dict(n=50_000_000, delta0=15_000_000, decay=0.6, rules=2), model.Row(3, 4, 8)),
    "wide 56B rows + tail": (dict(n=50_000_000, delta0=15_000_000, decay=0.6, rules=2, tail=(800, 1000)), model.Row(3, 4, 8)),
    "wide 56B, positional only, bulk-ish, 5 probes": (dict(n=50_000_000, delta0=15_000_000, decay=0.6, rules=5), model.Row(3, 4, 8)),
    "TC: 1M nodes, 200 reach/node, diameter 30 (200M pairs)": (dict(tc=(1_000_000, 200, 30, 10)), model.Row(1, 1, 4)),
    "TC: 100k nodes, 2000 reach, diameter 12 (200M pairs)": (dict(tc=(100_000, 2000, 12, 20)), model.Row(1, 1, 4)),
    "SG-like: 20M pairs, diameter 60, fanout 10": (dict(tc=(200_000, 100, 60, 10)), model.Row(1, 1, 4)),
    "doop-like: 30M rows, delta0 3M, t^-1.2, 6 probes, 24B rows": (dict(power=(30_000_000, 3_000_000, 1.2, 6)), model.Row(2, 1, 8)),
    "doop-like long: 10M rows, delta0 200k, t^-0.8, 6 probes, 24B": (dict(power=(10_000_000, 200_000, 0.8, 6)), model.Row(2, 1, 8)),
    "TPC-H SF10 chain: 5 builds (1.5M..60M) probed by 60M": (dict(chain=[(1_500_000, 15_000_000), (15_000_000, 60_000_000), (200_000, 60_000_000), (60_000_000, 60_000_000), (25, 60_000_000)]), model.Row(1, 1, 4)),
}


def rounds_for(kw):
    if "bulk" in kw:
        n, p = kw["bulk"]
        return [model.Round(n, p, build_first=True)]
    if "tc" in kw:
        n, reach, diam, fan = kw["tc"]
        return model.transitive_closure(n, reach, diam, fan)
    if "power" in kw:
        n, d0, alpha, rules = kw["power"]
        return model.power_tail(n, d0, alpha, rules)
    if "chain" in kw:
        return model.join_chain(kw["chain"])
    tail = kw.get("tail", (0, 1000))
    return model.datalog_synthetic(kw["n"], kw["delta0"], kw["decay"], kw["rules"], tail[0], tail[1])


def main():
    recs = []
    for f in sys.argv[1:]:
        recs += json.loads(Path(f).read_text())
    hw = model.Hardware(recs)
    placements = ["@gpu", "@cpu", "@cpu-probe/gpu-merge"]
    print("| scenario | rounds | best @gpu | best @cpu | best hybrid | GPU-merge payoff vs CPU |")
    print("|---|---:|---|---|---|---:|")
    for name, (kw, row) in SCENARIOS.items():
        rounds = rounds_for(kw)
        res = []
        for s in model.structures(15):
            try:
                res.append(model.simulate(hw, s, row, rounds, 1.0, 0.5, False))
            except KeyError:
                pass
        best = {}
        for pl in placements:
            cand = [r for r in res if r.structure.endswith(pl)]
            best[pl] = min(cand, key=lambda r: r.total)
        cells = []
        for pl in placements:
            b = best[pl]
            short = b.structure.replace(" " + pl, "").replace("sorted+dir(1 line) ", "s+d1 ").replace("sorted+dir(2 lines) ", "s+d2 ")
            cells.append(f"{short}: **{b.total:,.0f} ms** (probe {b.probe_ms:,.0f} / absorb {b.absorb_ms:,.0f} / ovh {b.overhead_ms:,.0f})")
        payoff = best["@cpu"].total / min(best["@gpu"].total, best["@cpu-probe/gpu-merge"].total)
        print(f"| {name} | {len(rounds)} | {cells[0]} | {cells[1]} | {cells[2]} | {payoff:.2f}x |")


if __name__ == "__main__":
    main()
