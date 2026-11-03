#!/usr/bin/env python3
"""Paired GPU benchmark on a disposable pod (2026-11-02 performance record).

Run from an unpacked remote-GPU bundle after every bundle's gate has mirrored
its worktree (tools/remote_gpu). Discovers sibling bundles (``bundle-*``),
labels them by manifest commit, and interleaves fresh-process samples:
one untimed warmup round, then PAIRS rounds whose order alternates (AB, BA).

Suites, in priority order (stopped between samples when --budget expires):
  gpu-tpch-sf1, gpu-tpch-sf10  cq-gpu-tpch --scale S --mode advised; every
                               query must report exact (5 measured iterations
                               checked against the DuckDB answers).
  doop-xalan, doop-batik (doop-cassandra optional)
                               doop-gpu: one untimed full run per bundle
                               (exact against doop-mini, all 26 outputs), then
                               --gpu-only timed runs whose 26 relation counts
                               must equal that exact run's counts.
Standard library only; writes RESULTS/pod-bench.json after every sample.
"""
import argparse, json, os, re, statistics, subprocess, sys, time
from datetime import datetime, timezone
from pathlib import Path


def now():
    return datetime.now(timezone.utc).isoformat()


def tenants():
    return subprocess.run(['nvidia-smi', '--query-compute-apps=pid,process_name,used_memory',
                           '--format=csv,noheader'], capture_output=True, text=True).stdout.strip()


def gpu_state():
    return subprocess.run(['nvidia-smi', '--query-gpu=name,clocks.sm,clocks.mem,clocks.max.sm,temperature.gpu,'
                           'power.draw,utilization.gpu,memory.used,clocks_throttle_reasons.active',
                           '--format=csv,noheader'], capture_output=True, text=True).stdout.strip()


class Bundle:
    def __init__(self, directory):
        self.dir = directory
        self.manifest = json.loads((directory / 'manifest.json').read_text())
        self.commit = self.manifest['commit']
        self.label = self.commit[:8]
        self.worktree = Path(self.manifest['worktree'])
        if not self.worktree.exists():
            self.worktree.parent.mkdir(parents=True, exist_ok=True)
            self.worktree.symlink_to(directory / 'source')
        self.env = os.environ.copy()
        self.env['LD_LIBRARY_PATH'] = f'{directory / "lib"}:/usr/local/nvidia/lib64:/usr/local/nvidia/lib'
        self.env.pop('CUDA_HOME', None)
        self.env['GPU_DATALOG_PHASES'] = '1'

    def binary(self, name):
        return str(self.dir / self.manifest['binaries'][name])


def run(bundle, argv):
    before = tenants()
    if before:
        raise RuntimeError('foreign GPU tenant: ' + before)
    began = time.monotonic()
    proc = subprocess.Popen(argv, cwd=str(bundle.worktree / 'experiments/gpu'), env=bundle.env,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    out, err = proc.communicate()
    wall = time.monotonic() - began
    if proc.returncode:
        raise RuntimeError(f'{argv} failed {proc.returncode}: {err[-3000:]}')
    return {'argv': argv, 'utc': now(), 'process_seconds': wall, 'gpu_state_after': gpu_state(),
            'tenants_after': tenants(), 'stdout': out, 'stderr': err}


def tpch(bundle, scale, out):
    rec = run(bundle, [bundle.binary('cq-gpu-tpch'), str(out), '--scale', scale, '--mode', 'advised'])
    report = json.loads(out.read_text())
    per = {}
    for entry in report['results']:
        if entry.get('exact') is not True:
            raise RuntimeError(f'{bundle.label} {scale} {entry.get("query")} not exact')
        per[entry['query']] = {'total_ms': entry['median_ms']['total'], 'median_ms': entry['median_ms'],
                               'answer_rows': entry.get('answer_rows'), 'samples': entry.get('samples')}
    rec.update(per_query=per, timing=report.get('timing'))
    rec['stdout'], rec['stderr'] = rec['stdout'][-2000:], rec['stderr'][-2000:]
    return rec


def counts(stdout):
    return {a: int(b) for a, b in re.findall(r'^(\w+)\t(\d+)$', stdout, re.M)}


def doop(bundle, dataset, full=False):
    rec = run(bundle, [bundle.binary('doop-gpu'), f'/data/doop-bench/doop-data/{dataset}'] +
              ([] if full else ['--gpu-only']))
    phases = {m[0]: float(m[1]) for m in re.findall(r'^phase\t(setup|fixpoint|output)\t([\d.]+)$', rec['stderr'], re.M)}
    rec.update(phases=phases, fixpoint_seconds=phases.get('fixpoint'),
               evaluation_seconds=float(re.search(r'^gpu_evaluation_seconds\t([\d.]+)', rec['stdout'], re.M)[1]),
               relation_counts=counts(rec['stdout']))
    if full and 'comparison\texact full-tuple sets, all 26 outputs' not in rec['stdout']:
        raise RuntimeError(f'{bundle.label} {dataset}: exact comparison line missing')
    rec['stdout'], rec['stderr'] = rec['stdout'][-3000:], rec['stderr'][-2000:]
    return rec


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--results', type=Path, required=True)
    parser.add_argument('--pairs', type=int, default=5)
    parser.add_argument('--budget', type=float, default=2100.0, help='seconds; no sample starts after it')
    parser.add_argument('--suites', nargs='+', default=['gpu-tpch-sf1', 'gpu-tpch-sf10', 'doop-xalan',
                                                        'doop-batik'])
    args = parser.parse_args()
    deadline = time.monotonic() + args.budget
    if 'BUNDLE_DIRS' in os.environ:
        dirs = [Path(p) for p in json.loads(os.environ['BUNDLE_DIRS'])]
    else:
        root = next(p for p in Path(__file__).resolve().parents if p.name.startswith('bundle-')).parent
        dirs = sorted(p for p in root.glob('bundle-*') if (p / 'manifest.json').exists())
    # One driver run interleaves every bundle; when the session invokes it per
    # bundle, only the last bundle's invocation measures.
    current = os.environ.get('BUNDLE_DIR')
    if current and Path(current).resolve() != dirs[-1].resolve():
        print('pod-bench: deferred to the last bundle invocation', flush=True)
        return 0
    for target, source in (('/data/tpch-gpu', '/workspace/data/tpch-gpu'),
                           ('/data/doop-bench/doop-data', '/workspace/data/doop-bench/doop-data')):
        if not Path(target).exists():
            Path(target).parent.mkdir(parents=True, exist_ok=True)
            Path(target).symlink_to(source)
    bundles = [Bundle(p) for p in dirs]
    if len(bundles) != 2:
        raise SystemExit(f'expected two bundles under {root}, found {len(bundles)}')
    args.results.mkdir(parents=True, exist_ok=True)
    path = args.results / 'pod-bench.json'
    data = {'started_utc': now(), 'bundles': {b.label: {'commit': b.commit, 'dir': str(b.dir),
                                                         'worktree': str(b.worktree)} for b in bundles},
            'nvidia_smi': subprocess.run(['nvidia-smi'], capture_output=True, text=True).stdout,
            'gpu_state_before': gpu_state(), 'pairs': args.pairs, 'budget_seconds': args.budget,
            'order': 'one warmup round, then rounds alternating AB / BA (A = first bundle)',
            'suites': {}, 'incomplete': []}

    def save():
        tmp = path.with_suffix('.tmp')
        tmp.write_text(json.dumps(data, indent=1, sort_keys=True))
        tmp.replace(path)

    save()
    for suite in args.suites:
        s = data['suites'].setdefault(suite, {'samples': {b.label: [] for b in bundles}, 'exact': {}})
        began = time.monotonic()
        if suite.startswith('doop-'):
            dataset = suite.removeprefix('doop-')
            for b in bundles:
                if time.monotonic() > deadline:
                    break
                s['exact'][b.label] = doop(b, dataset, full=True)
                print(f'{suite} {b.label} exact 26/26', flush=True)
                save()
            sample = lambda b, i: doop(b, dataset)
        else:
            scale = suite.removeprefix('gpu-tpch-')
            sample = lambda b, i: tpch(b, scale, args.results / f'{suite}-{i}-{b.label}.json')
        for i in range(-1, args.pairs):
            order = bundles if i % 2 == 0 else list(reversed(bundles))
            for b in order:
                if time.monotonic() > deadline:
                    data['incomplete'].append(f'{suite} stopped at pair {i} (budget)')
                    break
                rec = sample(b, i)
                if suite.startswith('doop-'):
                    rec['exact_counts_match'] = rec['relation_counts'] == s['exact'][b.label]['relation_counts']
                    if not rec['exact_counts_match']:
                        raise RuntimeError(f'{suite} {b.label} pair {i}: counts differ from exact run')
                rec.update(pair=i, warmup=i < 0)
                if i >= 0:
                    s['samples'][b.label].append(rec)
                print(f'{suite} {b.label} pair {i} {rec["process_seconds"]:.1f}s', flush=True)
                save()
            else:
                continue
            break
        s['gate_seconds'] = time.monotonic() - began
        save()
    data['finished_utc'] = now()
    data['gpu_state_after'] = gpu_state()
    save()
    return 1 if data['incomplete'] else 0


if __name__ == '__main__':
    sys.exit(main())
