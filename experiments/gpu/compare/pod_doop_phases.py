#!/usr/bin/env python3
"""Rotated fresh-process DOOP phase timings across any number of bundles.

Each bundle first runs once in full mode (GPU against the mini reference;
the exact-comparison line is required). Then `--rounds` rounds run every
bundle once in `--gpu-only` mode, rotating the start position each round.
A sample is refused if a GPU compute process is present before or after it.
Writes `doop-phases.json` with setup/fixpoint/output seconds per sample.
"""
import argparse, json, os, re, statistics, subprocess, time
from pathlib import Path

PHASE = re.compile(r'^phase\t(setup|fixpoint|output)\t([\d.]+)$', re.M)
EXACT = 'comparison\texact full-tuple sets, all 26 outputs'


def tenants():
    return subprocess.run(['nvidia-smi', '--query-compute-apps=pid,process_name,used_memory',
                           '--format=csv,noheader'], capture_output=True, text=True).stdout.strip()


class Bundle:
    def __init__(self, directory):
        self.dir = directory
        self.manifest = json.loads((directory / 'manifest.json').read_text())
        self.label = self.manifest['commit'][:8]
        worktree = Path(self.manifest['worktree'])
        if not worktree.exists():
            worktree.parent.mkdir(parents=True, exist_ok=True)
            worktree.symlink_to(directory / 'source')
        self.cwd = worktree / 'experiments/gpu'
        self.env = {k: v for k, v in os.environ.items() if not k.startswith('BENCH_CHECK_')}
        self.env['LD_LIBRARY_PATH'] = f'{directory / "lib"}:/usr/local/nvidia/lib64:/usr/local/nvidia/lib'
        self.env.pop('CUDA_HOME', None)
        self.env['GPU_DATALOG_PHASES'] = '1'
        self.binary = str(directory / self.manifest['binaries']['doop-gpu'])

    def run(self, dataset, gpu_only):
        before = tenants()
        if before:
            raise RuntimeError('foreign GPU tenant: ' + before)
        argv = [self.binary, f'/data/doop-bench/doop-data/{dataset}'] + (['--gpu-only'] if gpu_only else [])
        began = time.monotonic()
        proc = subprocess.run(argv, cwd=self.cwd, env=self.env, capture_output=True, text=True)
        wall = time.monotonic() - began
        after = tenants()
        if after:
            raise RuntimeError('foreign GPU tenant: ' + after)
        if proc.returncode != 0:
            raise RuntimeError(f'{self.label}: exit {proc.returncode}: {proc.stderr[-2000:]}')
        phases = {name: float(value) for name, value in PHASE.findall(proc.stderr)}
        if set(phases) != {'setup', 'fixpoint', 'output'}:
            raise RuntimeError(f'{self.label}: phases missing: {proc.stderr[-1000:]}')
        if not gpu_only and EXACT not in proc.stdout:
            raise RuntimeError(f'{self.label}: exact comparison line missing')
        return {'phases': phases, 'process_seconds': wall}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--results', type=Path, required=True)
    parser.add_argument('--dataset', default='xalan')
    parser.add_argument('--rounds', type=int, default=7)
    parser.add_argument('--budget', type=float, default=1800.0)
    args = parser.parse_args()
    dirs = [Path(p) for p in json.loads(os.environ['BUNDLE_DIRS'])]
    current = os.environ.get('BUNDLE_DIR')
    if current and Path(current).resolve() != dirs[-1].resolve():
        print('doop-phases: deferred to the last bundle invocation', flush=True)
        return 0
    for target, source in (('/data/doop-bench/doop-data', '/workspace/data/doop-bench/doop-data'),):
        if not Path(target).exists():
            Path(target).parent.mkdir(parents=True, exist_ok=True)
            Path(target).symlink_to(source)
    bundles = [Bundle(p) for p in dirs]
    deadline = time.monotonic() + args.budget
    args.results.mkdir(parents=True, exist_ok=True)
    out = args.results / 'doop-phases.json'
    data = {'dataset': args.dataset, 'bundles': [b.label for b in bundles], 'exact': {}, 'samples': [],
            'incomplete': []}
    for b in bundles:
        data['exact'][b.label] = b.run(args.dataset, gpu_only=False)
        out.write_text(json.dumps(data, indent=1))
    for round_index in range(args.rounds):
        shift = round_index % len(bundles)
        for b in bundles[shift:] + bundles[:shift]:
            if time.monotonic() > deadline:
                data['incomplete'].append(f'stopped in round {round_index} (budget)')
                out.write_text(json.dumps(data, indent=1))
                return 1
            sample = b.run(args.dataset, gpu_only=True)
            sample.update(bundle=b.label, round=round_index)
            data['samples'].append(sample)
            print(f'{b.label} r{round_index} fixpoint {sample["phases"]["fixpoint"]:.4f}', flush=True)
            out.write_text(json.dumps(data, indent=1))
    for b in bundles:
        values = [s['phases']['fixpoint'] for s in data['samples'] if s['bundle'] == b.label]
        print(f'{b.label} fixpoint median {statistics.median(values):.4f} s (n={len(values)})', flush=True)
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
