#!/usr/bin/env python3
"""Bounded Access-refinement laws, independent of compiler emission and Rust.

A finite universe exercises current left and right views separately from their
immutable stores. Block bounds describe original payload. An independent pairwise
oracle checks candidate pruning plus exact residual membership. This is neither
production runtime code nor an interpreter for a proposed compiler IR.
"""
from itertools import product


PHYSICAL_MODEL_RULES = {
    "AF-Keys": "exact_keys",
    "AF-Zones": "zone_summary",
    "AF-Scan": "scan_candidates",
    "AF-Prune": "prune_block_candidates",
    "AF-Semi": "reduce",
    "AF-Hash": "hash_witnesses",
    "AF-Nested": "nested_witnesses",
    "AF-Join": "join",
    "AF-Stream": "support_stream",
}


def exact_keys(rows, columns):
    return {tuple(row[c] for c in columns) for row in rows}


def zone_summary(keys):
    if not keys:
        return None
    return tuple((min(xs), max(xs)) for xs in zip(*keys))


def block_bounds(rows):
    assert rows
    return tuple((min(xs), max(xs)) for xs in zip(*rows))


def may_match(summary, bounds, columns):
    return summary is not None and all(
        lo <= bounds[c][1] and bounds[c][0] <= hi
        for (lo, hi), c in zip(summary, columns)
    )


def scan_candidates(store, current):
    return [i for i in range(len(store)) if i in current]


def prune_block_candidates(store, current, block_size, summary, columns):
    handles = []
    for start in range(0, len(store), block_size):
        block = store[start:start + block_size]
        selected = [i for i in range(start, start + len(block)) if i in current]
        if selected and may_match(summary, block_bounds(block), columns):
            handles.extend(selected)
    return handles


def reduce(store, candidates, keys, columns):
    return {i for i in candidates if tuple(store[i][c] for c in columns) in keys}


def oracle(store, current, right, left_columns, right_columns):
    # Deliberately does not construct keys, summaries or block candidates.
    return {i for i in current if any(
        all(store[i][lc] == row[rc] for lc, rc in zip(left_columns, right_columns))
        for row in right
    )}


def hash_witnesses(rows, columns):
    buckets = {}
    for row in rows:
        key = tuple(row[c] for c in columns)
        buckets.setdefault(key, []).append(row)
    return lambda key: buckets.get(key, [])


def nested_witnesses(rows, columns):
    # Construction does not enumerate rows; each probe repeats the source.
    return lambda key: [row for row in rows if tuple(row[c] for c in columns) == key]


def join(left, right, left_columns, right_columns, build):
    witnesses = build(right, right_columns)
    return {(a, b) for a in left for b in witnesses(tuple(a[c] for c in left_columns))}


def support_stream(values):
    return set(values)


def states(universe):
    # 0: absent from immutable store; 1: stored, unselected; 2: selected.
    for status in product(range(3), repeat=len(universe)):
        stored = [(row, flag) for row, flag in zip(universe, status) if flag]
        yield tuple(row for row, _ in stored), frozenset(
            i for i, (_, flag) in enumerate(stored) if flag == 2)


def main():
    universe = tuple(product((-1, 1), repeat=2))
    views = tuple(states(universe))
    layouts = (((), ()), ((0,), (0,)), ((1,), (0,)),
               ((0, 1), (0, 1)), ((1, 0), (0, 1)))
    cases = 0
    for left_store, left_mask in views:
        for right_store, right_mask in views:
            right = [right_store[i] for i in sorted(right_mask)]
            for left_columns, right_columns in layouts:
                keys = exact_keys(right, right_columns)
                summary = zone_summary(keys)
                expected = oracle(left_store, left_mask, right, left_columns, right_columns)
                all_handles = scan_candidates(left_store, left_mask)
                assert reduce(left_store, all_handles, keys, left_columns) == expected
                for size in (1, 2, 3):
                    candidates = prune_block_candidates(left_store, left_mask, size, summary, left_columns)
                    assert len(candidates) == len(set(candidates))
                    assert expected <= set(candidates) <= left_mask
                    assert reduce(left_store, candidates, keys, left_columns) == expected
                    cases += 1
    subsets = [tuple(row for row, bit in zip(universe, bits) if bit)
               for bits in product((False, True), repeat=len(universe))]
    joins = 0
    for left in subsets:
        for right in subsets:
            for lc, rc in layouts:
                expected = {(a, b) for a in left for b in right
                            if all(a[x] == b[y] for x, y in zip(lc, rc))}
                assert join(left, right, lc, rc, hash_witnesses) == expected
                assert join(left, right, lc, rc, nested_witnesses) == expected
                joins += 1
    # Correlated coordinates and i64 extrema are targeted beyond the tiny universe.
    keys = {(0, 1), (1, 0)}
    assert may_match(zone_summary(keys), ((0, 0), (0, 0)), (0, 1))
    assert (0, 0) not in keys
    edge_keys = {(-(2**63),), (2**63 - 1,)}
    assert may_match(zone_summary(edge_keys), ((0, 0),), (0,))
    assert support_stream([1, 1, 2]) == support_stream([1, 2])
    assert zone_summary(set()) is None and zone_summary({()}) == ()
    # A stale RHS superset would change exact semijoin semantics.
    assert oracle(((1, 1),), {0}, [], (0,), (0,)) == set()
    assert reduce(((1, 1),), [0], {(1,)}, (0,)) == {0}
    print(f"Checked {cases:,} current-view candidate cases and {joins:,} witness cases.")
    print("Plus correlated-key, i64-boundary, empty-key, stream-support and stale-version witnesses.")
    print("Bounded denotational evidence only; no Rust, macro-expansion or performance proof.")


if __name__ == "__main__":
    main()
