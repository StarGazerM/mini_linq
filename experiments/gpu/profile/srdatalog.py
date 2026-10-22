#!/usr/bin/env python3
"""Profile the exact cached artifact/data recorded by measure_srdatalog.py.

Run via the SRDatalog uv project. No external sources or baseline JSON are
modified. cudaProfilerStart/Stop bound only run+synchronize, excluding prepare.
"""
import argparse
import ctypes
import json
import os
from pathlib import Path
import time


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('baseline', type=Path)
    a = p.parse_args()
    baseline = json.loads(a.baseline.read_text())
    lib = ctypes.CDLL(baseline['artifact'], mode=ctypes.RTLD_GLOBAL)
    for name in ('init', 'load_all', 'prepare', 'run', 'synchronize', 'shutdown'):
        getattr(lib, 'srdatalog_' + name).restype = ctypes.c_int
    lib.srdatalog_load_all.argtypes = [ctypes.c_char_p]
    lib.srdatalog_run.argtypes = [ctypes.c_ulonglong]
    def checked(name, *args):
        result = getattr(lib, 'srdatalog_' + name)(*args)
        if result:
            raise RuntimeError(f'{name}: {result}')
    cuda = Path(os.environ['CUDA_HOME'])
    cudart = ctypes.CDLL(str(cuda / 'lib64/libcudart.so'))
    checked('init')
    try:
        checked('load_all', baseline['data_path'].encode())
        checked('prepare')
        checked('synchronize')
        if cudart.cudaProfilerStart():
            raise RuntimeError('cudaProfilerStart failed')
        start = time.perf_counter()
        checked('run', 0)
        checked('synchronize')
        elapsed = time.perf_counter() - start
        if cudart.cudaProfilerStop():
            raise RuntimeError('cudaProfilerStop failed')
        print(json.dumps({'dataset': baseline['dataset'], 'run_synchronized_seconds': elapsed,
                          'artifact': baseline['artifact'], 'data_path': baseline['data_path']}))
    finally:
        checked('shutdown')


if __name__ == '__main__':
    main()
