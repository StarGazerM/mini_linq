#!/usr/bin/env python3
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import time
from data_manifest import data_files


def main():
    parser = argparse.ArgumentParser(description='Download and checksum a pinned private HF dataset exactly once')
    parser.add_argument('repo')
    parser.add_argument('revision')
    parser.add_argument('manifest', type=Path)
    parser.add_argument('data', type=Path, nargs='?', default=Path('/workspace/data'))
    args = parser.parse_args()
    if not re.fullmatch(r'[0-9a-fA-F]{40}', args.revision):
        parser.error('revision must be an immutable 40-character HF commit id')
    expected = args.manifest.read_bytes()
    identity = json.dumps(dict(repo=args.repo, revision=args.revision,
                               manifest=hashlib.sha256(expected).hexdigest()), sort_keys=True)
    args.data.mkdir(parents=True, exist_ok=True)
    marker = args.data / ('.seeded-' + args.revision)
    began = time.monotonic()
    with (args.data / '.seed.lock').open('w') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if marker.exists() and marker.read_text() == identity:
            print(f'Dataset already verified: {args.revision}')
            return
        env = os.environ.copy()
        helper = Path(__file__).resolve().parent.parent / 'hf'
        if helper.is_dir():
            env['PYTHONPATH'] = str(helper)
            hf = ['python3', '-c', 'from huggingface_hub.cli.hf import main; main()']
        else:
            hf = ['hf']
        if not env.get('HF_TOKEN'):
            raise RuntimeError('HF_TOKEN is required for a first seed')
        subprocess.run([*hf, 'download', args.repo, '--repo-type', 'dataset',
                        '--revision', args.revision, '--local-dir', str(args.data)], env=env, check=True)
        if (args.data / 'SHA256SUMS').read_bytes() != expected:
            raise RuntimeError('Downloaded SHA256SUMS differs from the trusted local manifest')
        listed = set()
        for line in expected.decode().splitlines():
            match = re.fullmatch(r'[0-9a-f]{64}  (.+)', line)
            if not match:
                raise RuntimeError('Invalid SHA256SUMS line')
            relative = Path(match[1])
            if relative.is_absolute() or '..' in relative.parts or str(relative) in listed:
                raise RuntimeError('Unsafe or duplicate checksum path')
            listed.add(str(relative))
        actual = {str(path.relative_to(args.data)) for path in data_files(args.data)}
        if actual != listed:
            raise RuntimeError(f'Dataset inventory differs: missing={listed - actual}, extra={actual - listed}')
        subprocess.run(['sha256sum', '--check', '--strict', str(args.manifest.resolve())],
                       cwd=args.data, check=True)
        temporary = marker.with_suffix('.tmp')
        temporary.write_text(identity)
        temporary.replace(marker)
    print(f'Gate time seed-volume: {time.monotonic() - began:.3f}s')


if __name__ == '__main__':
    main()
