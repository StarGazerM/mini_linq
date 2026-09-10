#!/usr/bin/env python3
"""Bounded semantic prototype of the proposed YA physical design.

This is not the production backend, a complete administrative trace interpreter,
or a performance proof.  It checks runtime representation laws against an
independent nested-loop relational oracle.  Integers stand in for lawful typed
keys; deterministic toy Bloom hashing supplies the no-false-negative property.
Certificate key sets are specification witnesses, not proposed Bloom storage.
The manuscript's proposed baseline reuses filters only on identical versions.
The model additionally checks its separately justified inclusion-witness reuse
extension; attach() checks that mathematical witness at runtime in this oracle,
not a set-inclusion test that the production compiler may perform at expansion.

Function correspondence: need/(build_exact, build_filter)/reuse/attach model
PA-Need/Build/Reuse/Attach; reduce/transfer model PA-Reduce/Transfer; lookup/expand/collect model
PA-Lookup/Expand/Collect.  block_may_match/row_may_match/probe/keep implement the
RX-Block/Row/Probe/Keep cases.  Scheduling is the fixed, certified tree sweep.
These are denotational function correspondences, not validation of administrative
phases, evidence transitions, temporary linearity, or emitted-command dominance.
"""

from dataclasses import dataclass
from itertools import count, product


SERIAL = count()

# Only the rules with direct semantic witnesses here are listed.  Other PA rules
# (including Choose and evidence/region boundaries) require the future checker.
PHYSICAL_MODEL_RULES = {
    "PA-Need": "need",
    "PA-Build": "build_exact",  # build_filter also supplies this rule's laws.
    "PA-Reuse": "reuse",
    "PA-Attach": "attach",
    "PA-Reduce": "reduce",
    "PA-Transfer": "transfer",
    "PA-Lookup": "lookup",
    "PA-Expand": "expand",
    "PA-Collect": "collect",
    "RX-Block": "block_may_match",
    "RX-Row": "row_may_match",
    "RX-Probe": "probe",
    "RX-Keep": "keep",
}


@dataclass(frozen=True)
class Arena:
    identity: int
    heading: tuple
    rows: tuple


@dataclass(frozen=True)
class Version:
    identity: int
    arena: Arena
    live: frozenset

    @property
    def heading(self):
        return self.arena.heading


def collect(heading, rows):
    """PA-Collect: set normalization and fresh arena, including projection."""
    assert tuple(sorted(set(heading))) == tuple(heading)
    normalized = tuple(sorted(set(rows)))
    assert all(len(row) == len(heading) for row in normalized)
    arena = Arena(next(SERIAL), tuple(heading), normalized)
    return Version(next(SERIAL), arena, frozenset(range(len(normalized))))


def represented(version):
    return frozenset(version.arena.rows[i] for i in version.live)


def key_of(version, row_id, key):
    row = version.arena.rows[row_id]
    return tuple(row[version.heading.index(column)] for column in key)


def keys_of(version, key):
    return frozenset(key_of(version, i, key) for i in version.live)


def separator(left, right):
    return tuple(column for column in right.heading if column in left.heading)


def need(source, key, groups=False):
    """PA-Need: exact source identity, ordered key layout and consumer role."""
    assert tuple(c for c in source.heading if c in key) == tuple(key)
    return source.identity, tuple(key), groups


@dataclass(frozen=True)
class Exact:
    source: int
    key: tuple
    members: frozenset
    groups: tuple | None


def build_exact(source, key, groups=False):
    """PA-Build: HashSet semantics or immutable key-to-row-ID buckets."""
    identity, key, _ = need(source, key, groups)
    buckets = {}
    for row_id in sorted(source.live):
        buckets.setdefault(key_of(source, row_id, key), []).append(row_id)
    return Exact(identity, key, frozenset(buckets),
                 tuple((k, tuple(v)) for k, v in sorted(buckets.items()))
                 if groups else None)


def reuse(resource, source, key, groups=False):
    """PA-Reuse: groups can answer membership on the same exact version."""
    assert (resource.source, resource.key) == need(source, key)[:2]
    assert not groups or resource.groups is not None
    return resource


def probe(resource, source, key, value):
    reuse(resource, source, key)
    return value in resource.members


def lookup(resource, source, key, value):
    reuse(resource, source, key, groups=True)
    return next((ids for stored, ids in resource.groups if stored == value), ())


def bloom_positions(value, bits):
    # Whole-tuple hashing: position and every coordinate affect both hashes.
    h = 17
    for coordinate in value:
        h = (h * 1000003 + coordinate + 31) & 0xFFFFFFFFFFFFFFFF
    return h % bits, ((h ^ (h >> 13)) * 97 + 11) % bits


@dataclass(frozen=True)
class Prefilter:
    source: int
    key: tuple
    source_keys: frozenset  # Proof witness only; not part of physical storage.
    empty: bool
    bits: int | None
    bloom: int
    ranges: tuple | None


def build_filter(source, key, bits=32, ranges=True):
    need(source, key)
    assert bits is None or bits > 0
    values = keys_of(source, key)
    bloom = 0
    if bits is not None:
        for value in values:
            for position in bloom_positions(value, bits):
                bloom |= 1 << position
    intervals = None
    if ranges:
        intervals = tuple((min(v[j] for v in values), max(v[j] for v in values))
                          for j in range(len(key))) if values else ()
    return Prefilter(source.identity, tuple(key), values, not values,
                     bits, bloom, intervals)


def attach(filter_, source, key):
    """PA-Attach: a stale superset is safe only with an inclusion witness."""
    assert filter_.key == tuple(key)
    assert keys_of(source, key) <= filter_.source_keys
    return filter_


def row_may_match(filter_, value):
    if filter_.empty:
        return False
    if filter_.bits is not None:
        if any(not (filter_.bloom & (1 << p))
               for p in bloom_positions(value, filter_.bits)):
            return False
    if filter_.ranges is not None:
        if any(not lo <= item <= hi
               for item, (lo, hi) in zip(value, filter_.ranges)):
            return False
    return True


def block_may_match(filter_, left, row_ids):
    """Only coordinate min/max disjointness or certified emptiness skips."""
    if filter_.empty:
        return False
    if filter_.ranges is None:
        return True
    values = [key_of(left, i, filter_.key) for i in row_ids]
    return not any(max(v[j] for v in values) < lo
                   or min(v[j] for v in values) > hi
                   for j, (lo, hi) in enumerate(filter_.ranges))


def keep(left, row_ids):
    """RX-Keep creates a persistent selection; it never edits an older mask."""
    live = frozenset(row_ids)
    assert live <= left.live
    return Version(next(SERIAL), left.arena, live)


def scan_candidates(left, right, filter_=None, exact=None, counters=None):
    key = separator(left, right)
    if filter_ is not None:
        attach(filter_, right, key)
    if exact is not None:
        reuse(exact, right, key)
    stats = counters if counters is not None else {}
    bump = lambda name: stats.__setitem__(name, stats.get(name, 0) + 1)
    survivors = []
    # Block metadata includes all arena rows, not merely the active selection.
    for start in range(0, len(left.arena.rows), 2):
        block = range(start, min(start + 2, len(left.arena.rows)))
        bump("blocks")
        if filter_ is not None and not block_may_match(filter_, left, block):
            bump("blocks_skipped")
            continue
        for row_id in block:
            if row_id not in left.live:
                continue
            bump("rows")
            value = key_of(left, row_id, key)
            if filter_ is not None and not row_may_match(filter_, value):
                bump("rows_rejected")
                continue
            if exact is not None:
                bump("exact_probes")
                if not probe(exact, right, key, value):
                    continue
            survivors.append(row_id)
    return keep(left, survivors)


def reduce(left, right, resource=None, filter_=None, counters=None):
    key = separator(left, right)
    if resource is None:
        resource = build_exact(right, key)
    return scan_candidates(left, right, filter_, resource, counters)


def transfer(left, right, filter_):
    return scan_candidates(left, right, filter_)


def expand(left, right, resource):
    """Exact lookup followed by row-ID expansion; collect supplies set meaning."""
    key = separator(left, right)
    heading = tuple(sorted(set(left.heading) | set(right.heading)))
    rows = []
    for left_id in sorted(left.live):
        value = key_of(left, left_id, key)
        for right_id in lookup(resource, right, key, value):
            binding = dict(zip(left.heading, left.arena.rows[left_id]))
            binding.update(zip(right.heading, right.arena.rows[right_id]))
            rows.append(tuple(binding[c] for c in heading))
    return collect(heading, rows)


def assemble(versions):
    result = versions[0]
    for right in versions[1:]:
        result = expand(result, right,
                        build_exact(right, separator(result, right), groups=True))
    return result


def project(version, heading):
    return collect(heading, (tuple(row[version.heading.index(c)] for c in heading)
                             for row in represented(version)))


def oracle_join(relations):
    """Independent product of whole rows, checked by valuation agreement."""
    heading = tuple(sorted({c for columns, _ in relations for c in columns}))
    output = set()
    for choice in product(*(rows for _, rows in relations)):
        binding = {}
        consistent = True
        for (columns, _), row in zip(relations, choice):
            for column, value in zip(columns, row):
                if column in binding and binding[column] != value:
                    consistent = False
                binding[column] = value
        if consistent:
            output.add(tuple(binding[c] for c in heading))
    return heading, frozenset(output)


def oracle_project(relation, heading):
    columns, rows = relation
    return frozenset(tuple(dict(zip(columns, row))[c] for c in heading)
                     for row in rows)


def sweeps(original, parents, advisory=False, bits=32, ranges=True, filters=True):
    roots = [i for i, parent in enumerate(parents) if parent is None]
    assert len(roots) == 1 and len(parents) == len(original)
    order = roots[:]
    for parent in order:
        order.extend(i for i, p in enumerate(parents) if p == parent)
    assert sorted(order) == list(range(len(original)))
    # Independently check running intersection on every original column.
    for column in {c for version in original for c in version.heading}:
        containing = {i for i, v in enumerate(original) if column in v.heading}
        internal_edges = sum(parents[i] in containing for i in containing)
        assert internal_edges == len(containing) - 1
    full = oracle_join([(v.heading, represented(v)) for v in original])
    current = list(original)
    for upward, children in [(True, reversed(order[1:])), (False, order[1:])]:
        for child in children:
            parent = parents[child]
            target, source = (parent, child) if upward else (child, parent)
            left, right = current[target], current[source]
            filter_ = build_filter(right, separator(left, right), bits, ranges) if filters else None
            current[target] = transfer(left, right, filter_) if advisory else reduce(
                left, right, filter_=filter_)
            for initial, candidate in zip(original, current):
                assert oracle_project(full, candidate.heading) <= represented(candidate)
                assert represented(candidate) <= represented(initial)
    return tuple(current)


def rejects(call):
    try:
        call()
    except AssertionError:
        return
    raise AssertionError("invalid resource/certificate was accepted")


def exhaustive_chain():
    pairs = tuple(product(range(2), repeat=2))
    subsets = [tuple(row for i, row in enumerate(pairs) if mask & (1 << i))
               for mask in range(16)]
    cases = 0
    for rows in product(subsets, repeat=3):
        original = tuple(collect(h, r) for h, r in
                         zip([("a", "b"), ("b", "c"), ("c", "d")], rows))
        full = oracle_join([(v.heading, represented(v)) for v in original])
        masks = tuple(v.live for v in original)
        for advisory in (False, True):
            reduced = sweeps(original, (None, 0, 1), advisory)
            if not advisory:
                for v in reduced:
                    assert represented(v) == oracle_project(full, v.heading)
            result = assemble(reduced)
            assert (result.heading, represented(result)) == full
            assert represented(project(result, ("a", "d"))) == oracle_project(
                full, ("a", "d"))
        assert tuple(v.live for v in original) == masks
        cases += 1
    return cases


def targeted_checks():
    # Coordinate ranges lose correlation; exact whole-key probes repair it.
    left = collect(("k", "tag"), [(0, 0), (1, 1)])
    right = collect(("k", "tag"), [(0, 1), (1, 0)])
    filter_ = build_filter(right, right.heading, bits=1)
    assert all(row_may_match(filter_, row) for row in represented(left))
    assert not represented(reduce(left, right, filter_=filter_))
    rejects(lambda: reduce(left, right, build_exact(right, ("k",))))

    # Saturation is a valid Bloom state: final answers stay exact, reduction does not.
    candidates = sweeps((left, right), (None, 0), advisory=True, bits=1)
    assert all(represented(v) for v in candidates)
    assert not represented(assemble(candidates))
    assert not oracle_join([(v.heading, represented(v)) for v in (left, right)])[1]

    # A nonempty answer survives pure saturated Bloom and entirely absent filters.
    original = (collect(("a", "b"), [(0, 0), (1, 1)]),
                collect(("b", "c"), [(0, 0), (2, 2)]),
                collect(("c", "d"), [(0, 0), (3, 3)]))
    full = oracle_join([(v.heading, represented(v)) for v in original])
    assert full[1] == {(0, 0, 0, 0)}
    for advisory, filters in product((False, True), repeat=2):
        reduced = sweeps(original, (None, 0, 1), advisory,
                         bits=1, ranges=False, filters=filters)
        assert represented(assemble(reduced)) == full[1]
        if advisory:
            assert all(represented(v) == represented(initial)
                       for v, initial in zip(reduced, original))
        else:
            assert all(represented(v) == oracle_project(full, v.heading)
                       for v in reduced)

    # Every inserted composite key survives filters over small deterministic domains.
    for bits in (1, 2, 8, 32):
        filter_ = build_filter(right, right.heading, bits)
        assert all(row_may_match(filter_, row) for row in represented(right))

    # The overview's paired z positions differ: left field 1, right field 2.
    probe_side = collect(("y", "z"), [(7, 11), (8, 12)])
    build_side = collect(("k", "tag", "z"), [(99, 55, 11)])
    assert separator(probe_side, build_side) == ("z",)
    assert probe_side.heading.index("z") == 1 and build_side.heading.index("z") == 2
    assert represented(reduce(probe_side, build_side)) == {(7, 11)}
    assert represented(assemble((probe_side, build_side))) == {(99, 55, 7, 11)}

    # Explicit emptiness is usable even with neither ranges nor a Bloom array.
    for source_heading in (("y", "z"), ("q",)):
        empty = collect(source_heading, [])
        key = separator(probe_side, empty)
        for bits in (None, 1, 32):
            filter_ = build_filter(empty, key, bits=bits, ranges=False)
            assert not row_may_match(filter_, tuple(0 for _ in key))
            stats = {}
            assert not represented(reduce(probe_side, empty, filter_=filter_, counters=stats))
            assert stats == {"blocks": 1, "blocks_skipped": 1}
            assert not represented(transfer(probe_side, empty, filter_))

    # Two disconnected components have an empty separator; emptiness propagates.
    disconnected_cases = 0
    for first, second in product([[], [(0,)], [(0,), (0,), (1,)]], repeat=2):
        original = (collect(("a",), first), collect(("b",), second))
        full = oracle_join([(v.heading, represented(v)) for v in original])
        for advisory in (False, True):
            reduced = sweeps(original, (None, 0), advisory)
            assert represented(assemble(reduced)) == full[1]
            if not advisory:
                assert all(represented(v) == oracle_project(full, v.heading)
                           for v in reduced)
        disconnected_cases += 1

    # Exact resources cannot cross versions. Conservative superset filters can.
    old = collect(("k",), [(0,), (1,)])
    newer = keep(old, [1])
    left = collect(("k",), [(0,), (1,)])
    masks = old.live, newer.live
    rejects(lambda: reduce(left, newer, build_exact(old, ("k",))))
    broad = build_filter(old, ("k",), bits=32)
    assert represented(reduce(left, newer, filter_=broad)) == {(1,)}
    narrow = build_filter(newer, ("k",), bits=32)
    rejects(lambda: attach(narrow, old, ("k",)))
    assert not row_may_match(narrow, (0,))  # Unsafe if the inclusion check vanished.
    assert (old.live, newer.live) == masks

    # Same-version group buckets subsume membership, but key sets cannot expand.
    group = build_exact(old, ("k",), groups=True)
    assert represented(reduce(left, old, group)) == represented(left)
    rejects(lambda: lookup(build_exact(old, ("k",)), old, ("k",), (0,)))

    # Zone maps skip physical blocks only on a sound range/emptiness certificate.
    blocks = collect(("k",), [(0,), (1,), (10,), (11,)])
    wanted = collect(("k",), [(10,)])
    stats = {}
    selected = reduce(blocks, wanted, filter_=build_filter(wanted, ("k",)),
                      counters=stats)
    assert represented(selected) == {(10,)} and stats["blocks_skipped"] == 1
    assert stats["rows"] == 2 and stats["exact_probes"] == 1

    # Projection merges witnesses, establishes a new arena, and retains unique IDs.
    duplicated = collect(("a", "b"), [(1, 0), (1, 1), (1, 1)])
    projected = project(duplicated, ("a",))
    assert len(duplicated.live) == 2 and represented(projected) == {(1,)}
    assert projected.arena.identity != duplicated.arena.identity
    assert projected.live == frozenset([0])
    assert represented(project(duplicated, ())) == {()}
    assert not represented(project(collect(("a",), []), ()))
    # One-node YA is the identity; a non-tree with running-intersection failure rejects.
    assert sweeps((duplicated,), (None,)) == (duplicated,)
    rejects(lambda: sweeps((collect(("a", "b"), []),
                            collect(("b", "c"), []),
                            collect(("a", "c"), [])), (None, 0, 1)))
    return disconnected_cases


def main():
    assert all(callable(globals()[function]) for function in PHYSICAL_MODEL_RULES.values())
    count_ = exhaustive_chain()
    disconnected = targeted_checks()
    print(f"Checked {count_} exhaustive chain instances, {count_ * 2} YA sweeps, "
          f"{count_ * 4} exact reductions and {count_ * 4} advisory transfers.")
    print(f"Checked {disconnected} disconnected/empty input presentations and "
          "targeted composite-key, saturation, version, reuse, block, set, and mask laws.")
    print("Semantic prototype only; no production-backend or performance claim.")


if __name__ == "__main__":
    main()
