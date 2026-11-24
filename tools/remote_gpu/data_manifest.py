#!/usr/bin/env python3
import argparse
import hashlib
from pathlib import Path
import time

ROOTS = ['tpch-gpu/' + scale for scale in ('sf1', 'sf10', 'synthetic')] + [
    'doop-bench/doop-data/' + name for name in ('cassandra', 'xalan', 'batik')]


def data_files(root):
    for relative in ROOTS:
        directory = root / relative
        if not directory.is_dir():
            raise RuntimeError(f'Missing dataset root: {directory}')
        for path in sorted(directory.rglob('*')):
            if path.is_file():
                yield path


def main():
    parser = argparse.ArgumentParser(description='SHA256SUMS for the six GPU dataset roots')
    parser.add_argument('output', type=Path)
    parser.add_argument('root', type=Path, nargs='?', default=Path('/data'))
    args = parser.parse_args()
    start = time.monotonic()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    count = 0
    with args.output.open('w') as output:
        for path in data_files(args.root):
            relative = str(path.relative_to(args.root))
            if '\n' in relative or '\\' in relative:
                raise RuntimeError(f'Unsupported checksum filename: {relative!r}')
            digest = hashlib.sha256()
            with path.open('rb') as source:
                for block in iter(lambda: source.read(1024 * 1024), b''):
                    digest.update(block)
            output.write(f'{digest.hexdigest()}  {relative}\n')
            count += 1
    print(f'Gate time data-manifest: {time.monotonic() - start:.3f}s; {count} files')


if __name__ == '__main__':
    main()
