#!/usr/bin/env python3
"""Export actual dbgen-order TPC-H string columns, without modifying the database."""
import pathlib
import duckdb
import array
import sys
root = pathlib.Path('/data/mini_linq')
out = root / 'target/storage-bench/tpch-sf1'
con = duckdb.connect(str(root / 'features/workloads/target/bench/data/tpch-1/dbgen.duckdb'), read_only=True)
con.execute('SET threads=1')
# DuckDB's insertion-order-preserving scan matches the existing binary export.
for col in ['l_returnflag', 'l_linestatus']:
    rows = con.execute(f'SELECT {col} FROM lineitem').fetchall()
    data = bytes(ord(row[0]) for row in rows)
    (out / (col + '.bin')).write_bytes(data)
    print(col, len(data), sorted(set(data)))
numbers = array.array('q', (r[0] for r in con.execute('SELECT l_linenumber FROM lineitem').fetchall()))
if sys.byteorder != 'little':
    numbers.byteswap()
(out / 'l_linenumber.bin').write_bytes(numbers.tobytes())
print('l_linenumber', len(numbers), min(numbers), max(numbers))
