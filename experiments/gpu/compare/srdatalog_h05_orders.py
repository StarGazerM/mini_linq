#!/usr/bin/env python3
"""Exact generic-join cost of every srdatalog variable order for H05.

srdatalog evaluates a rule variable-at-a-time: the root variable's distinct
values are a ParallelFor; every deeper variable is a sequential nested loop
inside its root binding. At depth k the bindings are
    P_k = { b over the first k variables | every atom A touching them has
            b|vars(A) in pi_{vars(A) cap prefix}(A) },
with a filter applied only once its variable is bound. This script counts
|P_k| for every prefix set (bag-free: tries enumerate distinct values) and,
per order, the work under the heaviest single root value.

Run: python3 experiments/gpu/compare/srdatalog_h05_orders.py DBGEN.duckdb OUT.json
"""
import itertools
import json
import sys

import duckdb

DB, OUT = sys.argv[1], sys.argv[2]
DATE = "o_orderdate >= DATE '1994-01-01' AND o_orderdate < DATE '1995-01-01'"

# atom -> (relation SQL, {variable: column}); filters keyed by the variable they need.
FORMS = {
    # --dimension-filters: AsiaNation(n) is derived inside the timed run.
    'derived_filters': {
        'atoms': {
            'L': ('lineitem', {'o': 'l_orderkey', 's': 'l_suppkey'}),
            'O': ('orders', {'o': 'o_orderkey', 'c': 'o_custkey', 'd': 'o_orderdate'}),
            'C': ('customer', {'c': 'c_custkey', 'n': 'c_nationkey'}),
            'AN': ("(SELECT n_nationkey FROM nation JOIN region ON r_regionkey = n_regionkey "
                   "WHERE r_name = 'ASIA')", {'n': 'n_nationkey'}),
            'S': ('supplier', {'s': 's_suppkey', 'n': 's_nationkey'}),
        },
        'filters': {'d': ('O', DATE)},
    },
    'plain': {
        'atoms': {
            'L': ('lineitem', {'o': 'l_orderkey', 's': 'l_suppkey'}),
            'O': ('orders', {'o': 'o_orderkey', 'c': 'o_custkey', 'd': 'o_orderdate'}),
            'C': ('customer', {'c': 'c_custkey', 'n': 'c_nationkey'}),
            'N': ('nation', {'n': 'n_nationkey', 'r': 'n_regionkey'}),
            'R': ("(SELECT r_regionkey FROM region WHERE r_name = 'ASIA')", {'r': 'r_regionkey'}),
            'S': ('supplier', {'s': 's_suppkey', 'n': 's_nationkey'}),
        },
        'filters': {'d': ('O', DATE)},
    },
}

connection = duckdb.connect()
connection.execute(f"ATTACH '{DB}' AS tpch (READ_ONLY)")
connection.execute('USE tpch')
connection.execute('SET threads=24')


def level_sql(form, prefix):
    """Distinct bindings of `prefix` consistent with every touched atom's projection."""
    parts, bound = [], {}
    for name, (relation, columns) in form['atoms'].items():
        shared = [v for v in prefix if v in columns]
        if not shared:
            continue
        where = [sql for v, (atom, sql) in form['filters'].items() if atom == name and v in prefix]
        select = ', '.join(f'{columns[v]} AS {v}' for v in shared)
        parts.append((name, shared,
                      f"(SELECT DISTINCT {select} FROM {relation}" + (f" WHERE {' AND '.join(where)}" if where else '') + ')'))
    sql, seen = None, []
    for index, (name, shared, sub) in enumerate(parts):
        alias = f'a{index}'
        if sql is None:
            sql = f'{sub} {alias}'
        else:
            join = [f'{alias}.{v} = {bound[v]}' for v in shared if v in bound]
            sql += f" JOIN {sub} {alias} ON {' AND '.join(join)}" if join else f' CROSS JOIN {sub} {alias}'
        for v in shared:
            bound.setdefault(v, f'{alias}.{v}')
    return sql, bound


def count(form, prefix):
    sql, bound = level_sql(form, prefix)
    distinct = ', '.join(bound[v] for v in prefix)
    return connection.execute(f'SELECT count(*) FROM (SELECT DISTINCT {distinct} FROM {sql})').fetchone()[0]


def root_work(form, order):
    """Work (summed level sizes) under the heaviest single root value."""
    root = order[0]
    totals = {}
    for depth in range(1, len(order) + 1):
        prefix = order[:depth]
        sql, bound = level_sql(form, prefix)
        distinct = ', '.join(bound[v] for v in prefix)
        rows = connection.execute(
            f'SELECT {root}, count(*) FROM (SELECT DISTINCT {distinct} FROM {sql}) t({", ".join(prefix)}) GROUP BY {root}'
        ).fetchall()
        for value, n in rows:
            totals[value] = totals.get(value, 0) + n
    return max(totals.values()) if totals else 0


report = {}
for form_name, form in FORMS.items():
    variables = sorted({v for _, columns in form['atoms'].values() for v in columns})
    sizes = {}
    for k in range(1, len(variables) + 1):
        for subset in itertools.combinations(variables, k):
            sizes[frozenset(subset)] = count(form, list(subset))
    orders = []
    for order in itertools.permutations(variables):
        levels = [sizes[frozenset(order[:k])] for k in range(1, len(order) + 1)]
        orders.append({'order': ','.join(order), 'levels': levels, 'work': sum(levels), 'roots': levels[0]})
    orders.sort(key=lambda o: o['work'])
    # Heaviest-root work only for the cheapest orders with real root parallelism.
    for candidate in [o for o in orders if o['roots'] >= 1000][:12]:
        candidate['max_root_work'] = root_work(form, candidate['order'].split(','))
    report[form_name] = {'variables': variables, 'orders': orders}
    print(f'== {form_name}: {len(orders)} orders')
    for o in orders[:8]:
        print(f"  {o['order']:14s} work={o['work']:>12,} roots={o['roots']:>10,} levels={o['levels']}")
    print('  best with >=1000 roots:')
    for o in [o for o in orders if 'max_root_work' in o][:6]:
        print(f"  {o['order']:14s} work={o['work']:>12,} roots={o['roots']:>10,} max_root_work={o['max_root_work']:>9,} levels={o['levels']}")
json.dump(report, open(OUT, 'w'), indent=1)
