#!/usr/bin/env python3
"""Fresh-process paired GPU timings with persistent native tuple validation.

One untimed warmup round precedes alternating AB/BA measured rounds. Each
binary/configuration's first measured result is checked in full; subsequent
results require exact relation cardinalities and 1000 seeded actual tuples
(or all tuples for small relations) against a reference indexed once. Warmups
do not discharge the first measured full check. Ordinary DOOP gates stay full.
"""
import argparse, json, os, re, subprocess, sys, tempfile, time
from datetime import datetime, timezone
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / 'tools'))
from bench_check import Checker


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
        # A timing parent owns policy; inherited producer destinations cannot leak.
        for name in tuple(self.env):
            if name.startswith('BENCH_CHECK_'):
                self.env.pop(name)
        self.env['GPU_DATALOG_PHASES'] = '1'

    def binary(self, name):
        return str(self.dir / self.manifest['binaries'][name])

    def key(self, name, configuration):
        binary = self.manifest['binaries'][name]
        return f'{self.dir.resolve()}:{self.manifest["files"][binary]}:{configuration}'


def run(bundle, argv, allowed=(0,), env=None):
    before = tenants()
    if before:
        raise RuntimeError('foreign GPU tenant: ' + before)
    began = time.monotonic()
    # File-backed capture cannot fill a pipe while wait4 waits for the child.
    # Reap this PID directly: RUSAGE_CHILDREN would mix earlier samples and
    # cannot provide a per-process peak RSS by subtracting cumulative values.
    with tempfile.TemporaryFile(mode='w+t') as stdout, tempfile.TemporaryFile(mode='w+t') as stderr:
        with subprocess.Popen(argv, cwd=str(bundle.worktree / 'experiments/gpu'),
                              env={**bundle.env, **(env or {})},
                              stdout=stdout, stderr=stderr, text=True) as proc:
            _, status, usage = os.wait4(proc.pid, 0)
            proc.returncode = os.waitstatus_to_exitcode(status)
            wall = time.monotonic() - began
            stdout.seek(0)
            stderr.seek(0)
            out, err = stdout.read(), stderr.read()
    if proc.returncode not in allowed:
        raise RuntimeError(f'{argv} failed {proc.returncode}: {err[-3000:]}')
    return {'argv': argv, 'utc': now(), 'process_seconds': wall, 'gpu_state_after': gpu_state(),
            'tenants_after': tenants(), 'stdout': out, 'stderr': err, 'exit_code': proc.returncode,
            'resources': {'scope': 'benchmark child process (wait4)',
                          'minor_page_faults': usage.ru_minflt,
                          'major_page_faults': usage.ru_majflt,
                          'peak_rss_kib': usage.ru_maxrss,
                          'peak_rss_units': 'KiB (Linux ru_maxrss)'}}


def tpch(bundle, scale, out, checker, registered, pair):
    witness_dir = out.with_suffix('.witnesses')
    witness_dir.mkdir(parents=True, exist_ok=True)
    env = {'BENCH_CHECK_MODE': 'full' if pair <= 0 else 'sampled',
           'BENCH_CHECK_SEED': str(20261103 + max(pair, 0)),
           'BENCH_CHECK_SAMPLE': str(max(pair, 0)),
           'BENCH_CHECK_WITNESS_DIR': str(witness_dir.resolve())}
    # Exit 1 can mean a recorded workstation ceiling failed. Missing/failed
    # tuple evidence below still rejects it, independently of that ceiling.
    rec = run(bundle, [bundle.binary('cq-gpu-tpch'), str(out), '--scale', scale, '--mode', 'advised'],
              allowed=(0, 1), env=env)
    report = json.loads(out.read_text())
    per = {}
    validation_started = time.monotonic()
    if not report['results']:
        raise RuntimeError('TPC-H report has no query results')
    for entry in report['results']:
        query = entry['query']
        samples = entry.get('samples', [])
        if entry.get('validated') is not True or len(samples) != report['measured_runs'] or not samples:
            raise RuntimeError(f'{bundle.label} {scale} {query}: missing validation evidence')
        reference = f'tpch:{report["data_root"]}:{scale}:{query}:tsv-v1'
        if reference not in registered:
            oracle = Path(report['data_root']) / scale / f'answer.retail_{query}.tsv'
            checker.register_files(reference, {query: oracle})
            registered.add(reference)
        key = bundle.key('cq-gpu-tpch', f'{scale}:{query}:advised') + (':warmup' if pair < 0 else '')
        checks = []
        for local, sample in enumerate(samples):
            ordinal = max(pair, 0) * report['measured_runs'] + local
            evidence = sample['check']
            plan = checker.plan(key, reference, ordinal, seed=evidence['seed'])
            if plan['mode'] != evidence['mode']:
                raise RuntimeError(f'{key}: native check mode differs from persistent checker plan')
            witness = witness_dir / scale / entry['mode'] / f'{query}-{local}.json'
            checks.append(checker.check(key, reference, ordinal, witness))
            witness.unlink()
        per[query] = {'total_ms': entry['median_ms']['total'], 'median_ms': entry['median_ms'],
                      'answer_rows': entry['answer_rows'], 'samples': samples,
                      'validation': entry['validation'], 'checks': checks}
    (witness_dir / scale / 'advised').rmdir()
    (witness_dir / scale).rmdir()
    witness_dir.rmdir()
    rec.update(per_query=per, timing=report.get('timing'),
               validation_seconds=time.monotonic() - validation_started +
               sum(entry['validation']['seconds'] for entry in report['results']))
    rec['stdout'], rec['stderr'] = rec['stdout'][-2000:], rec['stderr'][-2000:]
    return rec


def counts(stdout):
    return {a: int(b) for a, b in re.findall(r'^(\w+)\t(\d+)$', stdout, re.M)}


def doop(bundle, dataset, env, full=False):
    rec = run(bundle, [bundle.binary('doop-gpu'), f'/data/doop-bench/doop-data/{dataset}'] +
              ([] if full else ['--gpu-only']), env=env)
    phases = {m[0]: float(m[1]) for m in re.findall(r'^phase\t(setup|fixpoint|output)\t([\d.]+)$', rec['stderr'], re.M)}
    witness_time = re.search(r'^phase\tbench_check_witness\tdone\t([\d.]+)$', rec['stderr'], re.M)
    rec.update(phases=phases, fixpoint_seconds=phases.get('fixpoint'),
               evaluation_seconds=float(re.search(r'^gpu_evaluation_seconds\t([\d.]+)', rec['stdout'], re.M)[1]),
               witness_seconds=float(witness_time[1]) if witness_time else 0.0,
               relation_counts=counts(rec['stdout']))
    if full and 'comparison\texact full-tuple sets, all 26 outputs' not in rec['stdout']:
        raise RuntimeError(f'{bundle.label} {dataset}: exact comparison line missing')
    rec['stdout'], rec['stderr'] = rec['stdout'][-3000:], rec['stderr'][-2000:]
    return rec


def doop_sample(bundle, dataset, checker, reference, pair, witness):
    key = bundle.key('doop-gpu', f'{dataset}:gpu-only') + (':warmup' if pair < 0 else '')
    ordinal = max(pair, 0)
    plan = checker.plan(key, reference, ordinal)
    rec = doop(bundle, dataset, {**checker.env(plan), 'BENCH_CHECK_WITNESS': str(witness.resolve())})
    started = time.monotonic()
    rec['check'] = checker.check(key, reference, ordinal, witness)
    rec['validation_seconds'] = rec['witness_seconds'] + time.monotonic() - started
    witness.unlink()
    return rec


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--results', type=Path, required=True)
    parser.add_argument('--pairs', type=int, default=5)
    parser.add_argument('--budget', type=float, default=2100.0, help='seconds; no sample starts after it')
    parser.add_argument('--suites', nargs='+', default=['gpu-tpch-sf1', 'gpu-tpch-sf10', 'doop-xalan', 'doop-batik'])
    args = parser.parse_args()
    args.results = args.results.resolve()
    deadline = time.monotonic() + args.budget
    if 'BUNDLE_DIRS' in os.environ:
        dirs = [Path(p) for p in json.loads(os.environ['BUNDLE_DIRS'])]
    else:
        root = next(p for p in Path(__file__).resolve().parents if p.name.startswith('bundle-')).parent
        dirs = sorted(p for p in root.glob('bundle-*') if (p / 'manifest.json').exists())
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
        raise SystemExit(f'expected two bundles, found {len(bundles)}')
    args.results.mkdir(parents=True, exist_ok=True)
    path = args.results / 'pod-bench.json'
    data = {'started_utc': now(), 'bundles': {b.label: {'commit': b.commit, 'dir': str(b.dir),
                                                         'worktree': str(b.worktree)} for b in bundles},
            'nvidia_smi': subprocess.run(['nvidia-smi'], capture_output=True, text=True).stdout,
            'gpu_state_before': gpu_state(), 'pairs': args.pairs, 'budget_seconds': args.budget,
            'order': 'one warmup round, then rounds alternating AB / BA (A = first bundle)',
            'validation': 'first measured full per binary/config, then exact counts + seeded actual tuple membership',
            'suites': {}, 'incomplete': []}

    def save():
        tmp = path.with_suffix('.tmp')
        tmp.write_text(json.dumps(data, indent=1, sort_keys=True))
        tmp.replace(path)

    save()
    registered = set()
    with Checker(binary=bundles[-1].binary('bench-check')) as checker:
        for suite in args.suites:
            s = data['suites'].setdefault(suite, {'samples': {b.label: [] for b in bundles}})
            began = time.monotonic()
            if suite.startswith('doop-'):
                dataset = suite.removeprefix('doop-')
                if time.monotonic() > deadline:
                    data['incomplete'].append(f'{suite} stopped before reference (budget)')
                    save()
                    continue
                reference = f'doop:{dataset}:semantic-26-v1'
                if reference not in registered:
                    reference_path = args.results / f'{suite}-reference.json'
                    s['reference'] = doop(bundles[0], dataset,
                                          {'BENCH_CHECK_REFERENCE': str(reference_path)}, full=True)
                    started = time.monotonic()
                    checker.register_witness(reference, reference_path)
                    s['reference']['index_seconds'] = time.monotonic() - started
                    reference_path.unlink()
                    registered.add(reference)
                    save()
                sample = lambda b, i: doop_sample(b, dataset, checker, reference, i,
                                                   args.results / f'{suite}-{i}-{b.label}.witness.json')
            else:
                scale = suite.removeprefix('gpu-tpch-')
                sample = lambda b, i: tpch(b, scale, args.results / f'{suite}-{i}-{b.label}.json',
                                           checker, registered, i)
            for i in range(-1, args.pairs):
                order = bundles if i % 2 == 0 else list(reversed(bundles))
                for b in order:
                    if time.monotonic() > deadline:
                        data['incomplete'].append(f'{suite} stopped at pair {i} (budget)')
                        break
                    rec = sample(b, i)
                    rec.update(pair=i, warmup=i < 0)
                    if i >= 0:
                        s['samples'][b.label].append(rec)
                    else:
                        s.setdefault('warmup', {})[b.label] = rec
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
