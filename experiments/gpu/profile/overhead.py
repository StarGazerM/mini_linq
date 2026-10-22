#!/usr/bin/env python3
"""Alternate feature-disabled/enabled oracle binaries without a profiler."""
import argparse
import json
from pathlib import Path
import re
import subprocess
import time


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('disabled')
    p.add_argument('enabled')
    p.add_argument('output', type=Path)
    p.add_argument('--pairs', type=int, default=7)
    a = p.parse_args()
    records = []
    a.output.mkdir(parents=True, exist_ok=True)
    for pair in range(a.pairs + 1):
        for mode in (('disabled', 'enabled') if pair % 2 == 0 else ('enabled', 'disabled')):
            while subprocess.check_output(['nvidia-smi', '--query-compute-apps=pid', '--format=csv,noheader'], text=True).strip():
                time.sleep(2)
            run = subprocess.run([getattr(a, mode), '--bench'], check=True, text=True, stdout=subprocess.PIPE)
            (a.output / f'{pair}-{mode}.log').write_text(run.stdout)
            for line in run.stdout.splitlines():
                if 'moderate-' not in line:
                    continue
                fields = dict(re.findall(r'(\w+)=([^ ]+)', line))
                records.append({'pair': pair, 'mode': mode, 'warmup': pair == 0, 'fields': fields})
            (a.output / 'runs.json').write_text(json.dumps(records, indent=2) + '\n')
    print(json.dumps(records, indent=2))


if __name__ == '__main__':
    main()
