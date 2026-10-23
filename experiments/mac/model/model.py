#!/usr/bin/env python3
"""Trace-driven cost model: measured hardware primitives x data structure x workload.

    python3 model.py --hardware ../calib/results/*.json --workload datalog --rules 2

Every structure operation is a composition of primitives measured by `maccal`
(experiments/mac/calib). Costs are milliseconds. If a primitive is missing the
structure is reported as skipped rather than guessed.

Nominal-byte conventions follow maccal: merge = 3 bytes moved per element byte,
partition pass = 4 (read 2 cols, write 2 cols), probe line = 128 B.
"""
from __future__ import annotations

import argparse
import bisect
import json
import math
from dataclasses import dataclass, field
from pathlib import Path

LINE = 128
MiB = 1 << 20
GiB = 1 << 30


# --------------------------------------------------------------------------- hardware
class Hardware:
    """Measured primitives. For each (op, processor, threads, filters) the footprint sweep is
    turned into a monotone curve (cost per item non-decreasing in footprint, GB/s non-increasing)
    and log-linearly interpolated: a smaller footprint can never cost more per item than a larger
    one, which removes fixed-overhead artifacts at tiny footprints (those are modeled separately)."""

    def __init__(self, records: list[dict]):
        self.recs = [r for r in records if not r.get("params", {}).get("partner")]
        self.hybrid = [r for r in records if r.get("params", {}).get("partner")]
        self._curves: dict = {}

    def _cands(self, op, proc, threads, filters):
        cands = [r for r in self.recs if r["op"] == op and r["processor"] == proc
                 and all(str(r.get("params", {}).get(k)) == str(v) for k, v in filters.items())]
        if not cands:
            raise KeyError(f"not measured: {op}/{proc} {filters}")
        ths = sorted({r["threads"] for r in cands})
        t = max([x for x in ths if x <= threads] or [ths[0]])
        return [r for r in cands if r["threads"] == t]

    def curve(self, op, proc, threads, key, **filters):
        """-> (log footprints, values) monotone in the physically meaningful direction."""
        ck = (op, proc, threads, key, tuple(sorted(filters.items())))
        if ck in self._curves:
            return self._curves[ck]
        pts: dict[float, float] = {}
        for r in self._cands(op, proc, threads, filters):
            fp = float(r.get("params", {}).get("footprint_bytes") or r["bytes"] or 1.0)
            v = float(r[key])
            best = (min if key == "ns_per_item" or key == "median_ms" else max)
            pts[fp] = best(pts.get(fp, v), v)
        fps = sorted(pts)
        vals = [pts[f] for f in fps]
        # enforce monotonicity from the large-footprint end
        if key == "gbps":
            for i in range(len(vals) - 2, -1, -1):
                vals[i] = max(vals[i], vals[i + 1])
        else:
            for i in range(len(vals) - 2, -1, -1):
                vals[i] = min(vals[i], vals[i + 1])
        cv = ([math.log(max(f, 1.0)) for f in fps], vals)
        self._curves[ck] = cv
        return cv

    def at(self, op, proc, threads, key, footprint, **filters) -> float:
        xs, ys = self.curve(op, proc, threads, key, **filters)
        x = math.log(max(footprint, 1.0))
        if x <= xs[0]:
            return ys[0]
        if x >= xs[-1]:
            return ys[-1]
        i = bisect.bisect_left(xs, x)
        w = (x - xs[i - 1]) / (xs[i] - xs[i - 1])
        return ys[i - 1] + w * (ys[i] - ys[i - 1])

    def rec(self, op, proc, threads, footprint=0, **filters) -> dict:
        """Compatibility: a synthetic record at `footprint` (ns_per_item, gbps, median_ms)."""
        return {"ns_per_item": self.at(op, proc, threads, "ns_per_item", footprint, **filters),
                "gbps": self.at(op, proc, threads, "gbps", footprint, **filters),
                "median_ms": self.at(op, proc, threads, "median_ms", footprint, **filters)}

    def slowdown(self, op: str, proc: str, threads: int, partner_op: str) -> float:
        best = None
        for r in self.hybrid:
            p = r["params"]
            if r["op"] == op and r["processor"] == proc and p["partner"].split(":")[1] == partner_op:
                if best is None or abs(r["threads"] - threads) < abs(best["threads"] - threads):
                    best = r
        return float(best["params"]["slowdown"]) if best else 1.0


# per-processor primitive names and parameter filters
OPS = {
    "gpu": {"merge": ("merge_path", {}), "sort": ("partition_pass", {}), "hist": ("hist_scan", {"bins": 65536}),
            "probe": ("gather_line", {}), "rmw": ("rmw_atomic", {}), "write": ("seq_write", {}),
            "read": ("seq_read", {})},
    "cpu": {"merge": ("merge_u32", {}), "sort": ("sort_u32", {}), "hist": ("hist_scan", {}),
            "probe": ("gather_line", {}), "rmw": ("rmw_atomic", {}), "write": ("seq_write", {}),
            "read": ("seq_read", {}), "hash_probe": ("hashbrown_probe", {}), "hash_build": ("hashbrown_build", {})},
}


@dataclass
class Proc:
    name: str
    threads: int = 1


class Cost:
    """Primitive costs in ms on a processor."""

    def __init__(self, hw: Hardware, p: Proc):
        self.hw, self.p = hw, p

    def _r(self, kind: str, footprint: float):
        op, filt = OPS[self.p.name][kind]
        return self.hw.rec(op, self.p.name, self.p.threads, footprint, **filt)

    def stream(self, kind: str, nbytes: float, footprint: float) -> float:
        return nbytes / self._r(kind, footprint)["gbps"] / 1e6

    def items(self, kind: str, n: float, footprint: float) -> float:
        return n * self._r(kind, footprint)["ns_per_item"] / 1e6

    def merge_rows(self, rows: float, row_bytes: float) -> float:
        return self.stream("merge", 3 * rows * row_bytes, rows * row_bytes)

    def sort_rows(self, rows: float, row_bytes: float, key_bytes: float) -> float:
        if self.p.name == "gpu":
            passes = max(1, math.ceil(key_bytes))  # one 8-bit pass per key byte
            return passes * self.stream("sort", 4 * rows * row_bytes / 2, rows * row_bytes)
        return self.items("sort", rows, rows * row_bytes) * (row_bytes / 4)

    def overhead(self, cmdbufs: int, dispatches: int) -> float:
        if self.p.name == "gpu":
            cb = self.hw.rec("dispatch_empty", "gpu", 1, dispatches=1)["median_ms"]
            per = self.hw.rec("dispatch_empty", "gpu", 1, dispatches=100)["ns_per_item"] / 1e6
            return cmdbufs * cb + dispatches * per
        # a pool that keeps workers spinning pays the best measured wake cost at <= threads
        cands = [r["median_ms"] for r in self.hw.recs
                 if r["op"] == "cpu_pool_dispatch" and r["threads"] <= self.p.threads]
        return dispatches * (min(cands) if cands else 0.0)

    def sync(self, n: int) -> float:
        return n * self.hw.rec("sync_roundtrip", "gpu", 1)["ns_per_item"] / 1e6


# --------------------------------------------------------------------------- structures
@dataclass
class Row:
    key_cols: int = 1
    payload_cols: int = 1
    width: int = 4

    @property
    def key_bytes(self):
        return self.key_cols * self.width

    @property
    def row_bytes(self):
        return (self.key_cols + self.payload_cols) * self.width


@dataclass
class Structure:
    name: str
    probe_lines: float                  # random lines per probe per live level
    residual_lines: float = 0.0         # extra lines on hit for composite-key verification
    policy: str = "leveled"             # inplace | full | leveled | lazy | blocked
    T: int = 4
    clustered: bool = True              # payload moves with keys in merges
    dir_bytes_per_row: float = 4.0
    probe_proc: Proc = field(default_factory=lambda: Proc("gpu"))
    maint_proc: Proc = field(default_factory=lambda: Proc("gpu"))
    hash_probe_measured: bool = False   # CPU: use hashbrown_probe directly as the fastest-join baseline

    def moved_row_bytes(self, row: Row) -> float:
        return row.row_bytes if self.clustered else row.key_bytes + 4


class Levels:
    """LSM level simulator.

    leveled: one run per level, level i holds <= base*T^i rows; overflow merges into i+1.
    lazy:    levels 0..L-2 are tiered (up to T runs each, merged together into the next
             level when full); the last level is a single run.
    absorb() returns the row counts of the merges performed (rows read == rows written).
    """

    def __init__(self, T: int, policy: str, base_rows: int):
        self.T, self.policy, self.base = T, policy, max(base_rows, 1)
        self.runs: list[list[int]] = []  # runs per level, level 0 newest

    def _cap(self, i: int) -> int:
        return self.base * self.T ** i

    def absorb(self, delta: int) -> list[int]:
        if self.policy in ("inplace", "blocked"):
            n = sum(sum(l) for l in self.runs)
            self.runs = [[n + delta]]
            return []
        if self.policy == "full":
            n = sum(sum(l) for l in self.runs)
            self.runs = [[n + delta]]
            return [n + delta]
        merges = []
        if not self.runs:
            self.runs.append([])
        if self.policy == "leveled":
            old = self.runs[0][0] if self.runs[0] else 0
            self.runs[0] = [old + delta]
            if old:
                merges.append(old + delta)
        else:
            self.runs[0].append(delta)
        i = 0
        while i < len(self.runs):
            level_rows = sum(self.runs[i])
            last = i == len(self.runs) - 1
            over = level_rows > self._cap(i) if self.policy == "leveled" else len(self.runs[i]) > self.T
            if not over:
                break
            if last:
                self.runs.append([])
            nxt = sum(self.runs[i + 1])
            merged = level_rows + nxt
            if nxt == 0 and len(self.runs[i]) == 1:
                self.runs[i + 1] = [merged]      # promote a single run through an empty level: no copy
            else:
                merges.append(merged)
                self.runs[i + 1] = [merged]
            self.runs[i] = []
            i += 1
        return merges

    def live(self) -> int:
        return max(sum(len(l) for l in self.runs), 1)


def absorb(hw: Hardware, s: Structure, row: Row, delta: int, n_total: int, lv: Levels,
           touched: float) -> tuple[float, float, int, int]:
    """-> (ms, bytes, cmdbufs, dispatches) to absorb delta rows."""
    c = Cost(hw, s.maint_proc)
    rb = s.moved_row_bytes(row)
    tb = max(n_total, 1) * rb
    if s.policy == "inplace":
        stages = 2 if s.maint_proc.name == "gpu" else 1      # claim + publish without 64-bit CAS
        lines = delta * (stages + 1)                           # + amortized doubling rebuild
        return c.items("rmw", lines, tb), lines * 2 * LINE, 1, stages
    if s.policy == "blocked":
        rows = n_total * touched + delta
        ms = c.sort_rows(delta, rb, row.key_bytes) + c.merge_rows(rows, rb) + c.items("hist", rows, rows * 4)
        return ms, 3 * rows * rb + 4 * delta * rb, 1, 4
    ms = c.sort_rows(delta, rb, row.key_bytes)
    nbytes = 4.0 * delta * rb
    disp = 2 * max(1, math.ceil(row.key_bytes)) if s.maint_proc.name == "gpu" else 1
    for rows in lv.absorb(delta):
        ms += c.merge_rows(rows, rb) + c.items("hist", rows, rows * s.dir_bytes_per_row)
        nbytes += 3 * rows * rb + rows * s.dir_bytes_per_row
        disp += 3
    return ms, nbytes, 1, disp


def probe(hw: Hardware, s: Structure, row: Row, probes: float, n_total: int, lv: Levels,
          fanout: float, hit: float) -> tuple[float, float]:
    c = Cost(hw, s.probe_proc)
    tb = max(n_total, 1) * (s.dir_bytes_per_row + row.key_bytes)
    levels = lv.live() if s.policy in ("leveled", "lazy") else 1
    if s.hash_probe_measured:
        ms = c.items("hash_probe", probes, tb)
        lines = probes
    else:
        lines = probes * levels * s.probe_lines + probes * hit * s.residual_lines
        ms = c.items("probe", lines, tb)
    out = probes * hit * fanout * row.row_bytes
    if out > 0:
        ms += c.stream("write", out, out)
    return ms, lines * LINE + out


# --------------------------------------------------------------------------- workloads
@dataclass
class Round:
    delta: int
    probes: float
    touched: float = 1.0  # fraction of body blocks touched (blocked policy)
    reset: bool = False   # start a new relation (fresh index) before this round


def datalog_synthetic(n_final: int, delta0: int, decay: float, rules: float, tail_rounds: int = 0,
                      tail_delta: int = 1000) -> list[Round]:
    """Geometric decay until the delta would drop below 1 row or n_final is reached,
    then `tail_rounds` rounds of `tail_delta` rows (the cvc5-style long tail)."""
    rounds, total, d = [], 0, float(delta0)
    while total < n_final and d >= 1.0:
        dd = min(int(d), n_final - total)
        rounds.append(Round(dd, rules * dd, min(1.0, 32 * dd / max(total, 1))))
        total += dd
        d *= decay
    for _ in range(tail_rounds):
        rounds.append(Round(tail_delta, rules * tail_delta, min(1.0, 32 * tail_delta / max(total, 1))))
        total += tail_delta
    return rounds


def transitive_closure(n_nodes: int, reach_per_node: float, diameter: int, fanout: float) -> list[Round]:
    """BFS-shaped frontier: delta rows per round follow a bell over `diameter` rounds whose
    total is n_nodes*reach_per_node pairs; one rule, each delta row probes Edge once."""
    total = n_nodes * reach_per_node
    weights = [math.exp(-((t - diameter / 2) / (diameter / 4)) ** 2) for t in range(diameter)]
    z = sum(weights)
    rounds = []
    for t, w in enumerate(weights):
        d = max(int(total * w / z), 1)
        rounds.append(Round(d, d * 1.0, min(1.0, 32 * d / max(total, 1))))
    return rounds


def power_tail(n_final: int, delta0: int, alpha: float, rules: float, max_rounds: int = 400) -> list[Round]:
    """doop-style: delta_t = delta0 * t^-alpha, many rules per delta row."""
    rounds, total = [], 0
    for t in range(1, max_rounds + 1):
        d = max(int(delta0 * t ** -alpha), 1)
        d = min(d, n_final - total)
        if d <= 0:
            break
        rounds.append(Round(d, rules * d, min(1.0, 32 * d / max(total, 1))))
        total += d
    return rounds


def join_chain(tables: list[tuple[int, float]]) -> list[Round]:
    """TPC-H style: independent build/probe pairs (build rows, probes), each a fresh index."""
    return [Round(n, p, reset=True) for n, p in tables]


def datalog_trace(path: Path, rules: float) -> list[Round]:
    return [Round(int(r["delta"]), float(r.get("probes", rules * r["delta"])), float(r.get("touched", 1.0)))
            for r in json.loads(path.read_text())]


# --------------------------------------------------------------------------- simulation
@dataclass
class Result:
    structure: str
    probe_ms: float = 0.0
    absorb_ms: float = 0.0
    overhead_ms: float = 0.0
    bytes: float = 0.0
    rounds: int = 0
    note: str = ""

    @property
    def total(self):
        return self.probe_ms + self.absorb_ms + self.overhead_ms


def simulate(hw: Hardware, s: Structure, row: Row, rounds: list[Round], fanout: float, hit: float,
             overlap: bool, base_rows: int = 1 << 16) -> Result:
    res = Result(s.name)
    lv = Levels(s.T, s.policy, base_rows)
    n = 0
    cp = Cost(hw, s.probe_proc)
    cm = Cost(hw, s.maint_proc)
    for r in rounds:
        if r.reset:
            lv = Levels(s.T, s.policy, base_rows)
            n = 0
        pm, pb = probe(hw, s, row, r.probes, n, lv, fanout, hit)
        am, ab, cbs, disp = absorb(hw, s, row, r.delta, n, lv, r.touched)
        ov = cm.overhead(cbs, disp) + cp.overhead(1, 2)
        split = s.probe_proc.name != s.maint_proc.name
        if split:
            ov += cm.sync(2)
        if overlap and split:
            pop = OPS[s.probe_proc.name]["hash_probe" if s.hash_probe_measured else "probe"][0]
            mop = OPS[s.maint_proc.name]["merge"][0]
            pm *= hw.slowdown(pop, s.probe_proc.name, s.probe_proc.threads, mop)
            am *= hw.slowdown(mop, s.maint_proc.name, s.maint_proc.threads, pop)
            step = max(pm, am)
            res.probe_ms += pm
            res.absorb_ms += step - pm
        else:
            res.probe_ms += pm
            res.absorb_ms += am
        res.overhead_ms += ov
        res.bytes += pb + ab
        res.rounds += 1
        n += r.delta
    return res


def structures(cpu_threads: int) -> list[Structure]:
    gpu, cpu = Proc("gpu"), Proc("cpu", cpu_threads)
    out = []
    for place, pp, mp in (("gpu", gpu, gpu), ("cpu", cpu, cpu), ("cpu-probe/gpu-merge", cpu, gpu)):
        out.append(Structure(f"hash-inplace @{place}", 1.5, policy="inplace", probe_proc=pp, maint_proc=mp,
                             hash_probe_measured=(pp.name == "cpu")))
        out += [
            Structure(f"sorted+dir(1 line) full-merge @{place}", 1, policy="full", probe_proc=pp, maint_proc=mp),
            Structure(f"sorted+dir(1 line) leveled T=4 @{place}", 1, policy="leveled", probe_proc=pp, maint_proc=mp),
            Structure(f"sorted+dir(2 lines) leveled T=4 @{place}", 2, policy="leveled", probe_proc=pp, maint_proc=mp),
            Structure(f"sorted+dir(1 line) lazy-leveled T=4 @{place}", 1, policy="lazy", probe_proc=pp, maint_proc=mp),
            Structure(f"sorted+dir(1 line) blocked-partial @{place}", 1, policy="blocked", probe_proc=pp, maint_proc=mp),
            Structure(f"sorted+dir(1 line) leveled positional-payload @{place}", 1, residual_lines=1,
                      policy="leveled", clustered=False, probe_proc=pp, maint_proc=mp),
        ]
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--hardware", nargs="+", required=True)
    ap.add_argument("--workload", choices=["datalog", "bulk"], default="datalog")
    ap.add_argument("--trace", type=Path)
    ap.add_argument("--n", type=int, default=50_000_000, help="final rows (datalog) / build rows (bulk)")
    ap.add_argument("--delta0", type=int, default=5_000_000)
    ap.add_argument("--decay", type=float, default=0.7)
    ap.add_argument("--tail-rounds", type=int, default=0, help="extra rounds of --tail-delta rows")
    ap.add_argument("--tail-delta", type=int, default=1000)
    ap.add_argument("--rules", type=float, default=2.0, help="probes per delta row")
    ap.add_argument("--probes", type=float, default=200_000_000, help="bulk: probe count")
    ap.add_argument("--fanout", type=float, default=1.0)
    ap.add_argument("--hit", type=float, default=0.5)
    ap.add_argument("--key-cols", type=int, default=1)
    ap.add_argument("--payload-cols", type=int, default=1)
    ap.add_argument("--width", type=int, default=4)
    ap.add_argument("--cpu-threads", type=int, default=15)
    ap.add_argument("--overlap", action="store_true")
    ap.add_argument("--base-rows", type=int, default=1 << 16, help="level-0 capacity (rows)")
    ap.add_argument("--top", type=int, default=12)
    a = ap.parse_args()

    recs = []
    for f in a.hardware:
        recs += json.loads(Path(f).read_text())
    hw = Hardware(recs)
    row = Row(a.key_cols, a.payload_cols, a.width)
    if a.workload == "bulk":
        rounds = [Round(a.n, a.probes)]
    elif a.trace:
        rounds = datalog_trace(a.trace, a.rules)
    else:
        rounds = datalog_synthetic(a.n, a.delta0, a.decay, a.rules, a.tail_rounds, a.tail_delta)

    results = []
    for s in structures(a.cpu_threads):
        try:
            results.append(simulate(hw, s, row, rounds, a.fanout, a.hit, a.overlap, a.base_rows))
        except KeyError as e:
            results.append(Result(s.name, note=str(e)))
    results.sort(key=lambda r: r.total if r.rounds else float("inf"))
    print(f"workload={a.workload} rounds={len(rounds)} rows={sum(r.delta for r in rounds):,} "
          f"probes={sum(r.probes for r in rounds):,.0f} row={row.row_bytes}B key={row.key_bytes}B "
          f"overlap={a.overlap}")
    print("| structure | total ms | probe | absorb | launch/sync | GB moved |")
    print("|---|---:|---:|---:|---:|---:|")
    for r in results[: a.top]:
        if not r.rounds:
            print(f"| {r.structure} | skipped: {r.note} | | | | |")
        else:
            print(f"| {r.structure} | {r.total:,.1f} | {r.probe_ms:,.1f} | {r.absorb_ms:,.1f} | "
                  f"{r.overhead_ms:,.1f} | {r.bytes / 1e9:,.2f} |")


if __name__ == "__main__":
    main()
