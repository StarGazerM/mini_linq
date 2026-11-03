#!/usr/bin/env python3
"""Execute a built bundle; no Cargo, CUDA toolkit, profiler or package installation."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import subprocess
import time


def mirror(path, destination):
    path, destination = Path(path), Path(destination).resolve(strict=True)
    if path.is_symlink():
        if path.resolve() == destination:
            return
        # Only replace a prior bundle mirror, not an unrelated administrator link.
        if not (path.resolve().parent / 'manifest.json').is_file():
            raise RuntimeError(f'Refusing to replace unrelated mirror {path}')
        path.unlink()
    elif path.exists():
        if path.resolve() == destination:
            return
        raise RuntimeError(f'Refusing to overwrite {path}; use a fresh pod or --existing-mirrors locally')
    path.parent.mkdir(parents=True, exist_ok=True)
    path.symlink_to(destination, target_is_directory=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bundle', type=Path, default=Path(__file__).resolve().parent.parent)
    parser.add_argument('--results', type=Path, required=True)
    parser.add_argument('--data', type=Path, default=Path('/workspace/data'))
    parser.add_argument('--existing-mirrors', action='store_true', help='Local verification only; never replace existing paths')
    parser.add_argument('--allow-sf10-oom', action='store_true', help='Local verification only; record SF10 OOM as skipped, not PASS')
    parser.add_argument('--bench', nargs=argparse.REMAINDER, help='Run one bundled named binary with literal argv instead of the gate')
    args = parser.parse_args()
    bundle, results = args.bundle.resolve(), args.results.resolve()
    results.mkdir(parents=True, exist_ok=False)
    manifest = json.loads((bundle / 'manifest.json').read_text())
    status = dict(commit=manifest['commit'], steps=[], running=None, complete=False)
    status_path = results / 'status.json'

    def save():
        status_path.write_text(json.dumps(status, indent=2) + '\n')

    if args.bench is not None and not args.bench:
        parser.error('--bench requires a binary name')
    steps = manifest['steps'] if args.bench is None else [dict(name='bench-' + args.bench[0],
        argv=[manifest['binaries'][args.bench[0]], *args.bench[1:]])]
    status['not_run'] = [step['name'] for step in steps]
    save()
    # Hashes bind the source snapshot, ELF cubins and libraries to this manifest.
    for relative, expected in manifest['files'].items():
        digest = hashlib.sha256()
        with (bundle / relative).open('rb') as stream:
            for chunk in iter(lambda: stream.read(1024 * 1024), b''):
                digest.update(chunk)
        if digest.hexdigest() != expected:
            raise RuntimeError(f'Bundle checksum mismatch: {relative}')
    smi = subprocess.check_output(['nvidia-smi'], text=True)
    (results / 'nvidia-smi.txt').write_text(smi)
    version = re.search(r'CUDA Version:\s*(\d+)\.(\d+)', smi)
    if not version or tuple(map(int, version.groups())) < (13, 0):
        raise RuntimeError('Requires NVIDIA driver CUDA >= 13.0')
    devices = subprocess.check_output(['nvidia-smi', '--query-gpu=index,compute_cap', '--format=csv,noheader'], text=True)
    selected = next((line.split(',')[0].strip() for line in devices.splitlines()
                     if line.split(',')[-1].strip() == '8.9'), None)
    if selected is None:
        raise RuntimeError('Requires an sm_89 GPU (RTX 6000 Ada, L40/L40S or RTX 4090)')
    if not args.existing_mirrors:
        mirror(manifest['worktree'], bundle / 'source')
        mirror('/data/tpch-gpu', args.data / 'tpch-gpu')
        mirror('/data/doop-bench/doop-data', args.data / 'doop-bench/doop-data')
    for path in [manifest['worktree'], '/data/tpch-gpu', '/data/doop-bench/doop-data']:
        if not Path(path).is_dir():
            raise RuntimeError(f'Missing runtime mirror: {path}')
    env = os.environ.copy()
    # Do not inherit workstation CUDA SDK paths; use only bundled dependencies
    # plus container driver locations (provided by nvidia-container-runtime).
    env['LD_LIBRARY_PATH'] = str(bundle / 'lib') + ':/usr/local/nvidia/lib64:/usr/local/nvidia/lib'
    env['CUDA_VISIBLE_DEVICES'] = selected
    env.pop('CUDA_HOME', None)
    env.pop('CUDA_OXIDE_PTX_DIR', None)
    failed = False
    for step in steps:
        name = step['name']
        status['running'] = name
        status['not_run'].remove(name)
        save()
        began = time.monotonic()
        commands = step.get('commands', [step])
        code = 0
        logfile = results / (name + '.log')
        with logfile.open('w') as log:
            for command in commands:
                argv = [str(bundle / command['argv'][0]), *[
                    arg.replace('{results}', str(results)) for arg in command['argv'][1:]]]
                cwd = command.get('cwd', manifest['worktree'] + '/experiments/gpu')
                print('$ ' + shlex.join(argv), file=log, flush=True)
                code = subprocess.run(['nice', '-n', '10', *argv], cwd=cwd, env=env,
                                      stdout=log, stderr=subprocess.STDOUT).returncode
                if code:
                    break
        elapsed = time.monotonic() - began
        outcome = 'PASS' if code == 0 else 'FAIL'
        print(f'gpu-gate {outcome} {name} {int(elapsed)}s', flush=True)
        skipped = bool(code and name == 'tpch-h05-sf10-ceiling' and args.allow_sf10_oom and
                       re.search(r'CUDA_ERROR_OUT_OF_MEMORY|out of memory', logfile.read_text(), re.I))
        if skipped:
            print('gpu-gate SKIP tpch-h05-sf10-ceiling (local GPU out of memory)', flush=True)
        status['steps'].append(dict(name=name, argv=commands, seconds=elapsed,
                                    exit_code=code, outcome='SKIP-OOM' if skipped else outcome))
        status['running'] = None
        save()
        if code and not skipped:
            failed = True
            break
    status['complete'] = not status['not_run'] and all(s['outcome'] == 'PASS' for s in status['steps'])
    save()
    return int(failed)


if __name__ == '__main__':
    raise SystemExit(main())
