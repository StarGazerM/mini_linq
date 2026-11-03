#!/usr/bin/env python3
"""Offline tests: no RunPod calls, SSH connections, or GPU execution."""
import contextlib
import hashlib
import io
import importlib.util
import json
import os
from pathlib import Path
import shlex
import signal
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import bundle as bundle_builder
import data_manifest
import pod_gate
import seed_volume
import session


def compare_module(name):
    path = Path(__file__).resolve().parents[2] / 'experiments/gpu/compare' / (name + '.py')
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


p6c_driver = compare_module('pod_bench_p6c')
p6c_analysis = compare_module('p6c_analyze')


class MirrorTests(unittest.TestCase):
    def test_mirror_switches_only_owned_bundle(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in ('one', 'two'):
                (root / name / 'source').mkdir(parents=True)
                (root / name / 'manifest.json').write_text('{}')
            link = root / 'worktree'
            pod_gate.mirror(link, root / 'one/source')
            pod_gate.mirror(link, root / 'two/source')
            self.assertEqual(link.resolve(), root / 'two/source')
            ordinary = root / 'existing'
            ordinary.mkdir()
            with self.assertRaises(RuntimeError):
                pod_gate.mirror(ordinary, root / 'one/source')

    def test_data_manifest_covers_all_six_roots(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in data_manifest.ROOTS:
                (root / name).mkdir(parents=True)
                (root / name / 'file').write_bytes(name.encode())
            output = root / 'SHA256SUMS'
            with patch.object(sys, 'argv', ['make', str(output), str(root)]):
                data_manifest.main()
            lines = output.read_text().splitlines()
            self.assertEqual(len(lines), 6)
            self.assertTrue(all(line.split('  ')[1].endswith('/file') for line in lines))

    def test_verified_marker_never_redownloads(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = root / 'SHA256SUMS'
            manifest.write_text('trusted\n')
            revision = 'a' * 40
            identity = json.dumps(dict(repo='owner/data', revision=revision,
                manifest=hashlib.sha256(manifest.read_bytes()).hexdigest()), sort_keys=True)
            (root / ('.seeded-' + revision)).write_text(identity)
            with patch.object(sys, 'argv', ['seed', 'owner/data', revision, str(manifest), str(root)]), \
                 patch.object(subprocess, 'run', side_effect=AssertionError('must not download')):
                seed_volume.main()


class BundleTests(unittest.TestCase):
    def exercise_bundle(self, natives=(), absolute=False):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            workspace = root / 'experiments/gpu'
            target = workspace / 'target'
            target.mkdir(parents=True)
            (root / 'fixture.txt').write_text('runtime source')
            names = ['cq-gpu-tpch', 'doop-gpu', 'srd-doop-gpu']
            for name in names:
                (target / name).write_bytes(b'executable fixture')
            native_args = []
            for relative, mode in natives:
                path = root / relative
                if mode is not None:
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_bytes(b'native executable fixture')
                    path.chmod(mode)
                native_args.extend(['--native', str(path) if absolute else relative])
            metadata = dict(target_directory=str(target), packages=[
                dict(id=name, name=name, manifest_path=str(workspace / name / 'Cargo.toml'), targets=[])
                for name in names])
            archive_manifest = {}
            oxide_commands = []
            linked_executables = []

            def check_output(argv, **kwargs):
                if argv[0] == 'git':
                    return 'commit\n' if 'rev-parse' in argv else b'fixture.txt\0'
                if argv[0] == 'cargo':
                    return json.dumps(metadata)
                if argv[0] == 'ldd':
                    linked_executables.append(argv[1])
                    return 'libc.so.6 => /lib/libc.so.6 (0x0)\n'
                raise AssertionError(argv)

            def run(argv, **kwargs):
                if 'oxide' in argv:
                    oxide_commands.append(argv)
                    self.assertEqual(argv[argv.index('oxide') + 1], 'build')
                    for name in names:
                        kwargs['stdout'].write(json.dumps(dict(reason='compiler-artifact',
                            executable=str(target / name), target=dict(name=name))) + '\n')
                elif argv[0] == 'tar':
                    stage = Path(argv[argv.index('-C') + 1])
                    archive_manifest.update(json.loads((stage / 'manifest.json').read_text()))
                    self.assertEqual((stage / 'source/fixture.txt').read_text(), 'runtime source')
                    for relative, mode in natives:
                        packaged = stage / 'bin' / Path(relative).name
                        self.assertEqual(packaged.read_bytes(), b'native executable fixture')
                        self.assertTrue(os.access(packaged, os.X_OK))
                        self.assertEqual(archive_manifest['files'][str(packaged.relative_to(stage))],
                                         hashlib.sha256(packaged.read_bytes()).hexdigest())
                    Path(argv[argv.index('-cf') + 1]).write_bytes(b'archive fixture')
                elif argv[0] not in ('flock', 'python3'):
                    raise AssertionError(argv)
                return subprocess.CompletedProcess(argv, 0)

            with patch.object(sys, 'argv', ['bundle', str(root), str(root / 'bundle.tar.zst'),
                                           '--no-tests', *native_args]), \
                 patch.object(subprocess, 'check_output', side_effect=check_output), \
                 patch.object(subprocess, 'run', side_effect=run), \
                 patch.dict(os.environ, {'DUCKDB_CLI': str(target / names[0]),
                     'TPCH_GPU_ADVICE': 'candidate.json', 'CQ_GPU_TPCH_QUERIES': 'h06',
                     'RUSTFLAGS': '-Clink-arg=-Wl,--sort-section=name'}), \
                 contextlib.redirect_stdout(io.StringIO()):
                bundle_builder.main()
            self.assertEqual(len(oxide_commands), 1)
            self.assertEqual(archive_manifest['tests'], [])
            self.assertEqual(archive_manifest['steps'], [])
            self.assertEqual(set(archive_manifest['binaries']),
                             set(names) | {Path(relative).name for relative, mode in natives})
            self.assertEqual([build['name'] for build in archive_manifest['builds']], ['binaries-build'])
            for relative, mode in natives:
                self.assertIn(str(root / relative), linked_executables)
            environment = archive_manifest['builds'][0]['environment']
            self.assertEqual(environment['TPCH_GPU_ADVICE'], 'candidate.json')
            self.assertEqual(environment['CQ_GPU_TPCH_QUERIES'], 'h06')
            self.assertEqual(environment['RUSTFLAGS'], '-Clink-arg=-Wl,--sort-section=name')
            return archive_manifest

    def test_no_tests_builds_only_binaries_and_emits_no_gate(self):
        self.exercise_bundle()

    def test_native_repeatable_relative_and_absolute_paths(self):
        for absolute in (False, True):
            with self.subTest(absolute=absolute):
                manifest = self.exercise_bundle([('native/bench', 0o755), ('native/scan', 0o755)],
                                                absolute=absolute)
                self.assertEqual(pod_gate.benchmark_argv(Path('/bundle'), manifest, ['bench', 'q6', '1']),
                                 ['/bundle/bin/bench', 'q6', '1'])

    def test_native_rejects_missing_nonexecutable_and_collisions(self):
        for natives in ([('native/missing', None)], [('native/noexec', 0o644)],
                        [('native/cq-gpu-tpch', 0o755)], [('native/duckdb', 0o755)],
                        [('one/bench', 0o755), ('two/bench', 0o755)]):
            with self.subTest(natives=natives), contextlib.redirect_stderr(io.StringIO()), \
                 self.assertRaises(SystemExit) as error:
                self.exercise_bundle(natives)
            self.assertEqual(error.exception.code, 2)

    def test_pod_runner_rejects_full_gate_without_tests(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'manifest.json').write_text(json.dumps(dict(commit='test', steps=[])))
            with patch.object(sys, 'argv', ['pod-gate', '--bundle', str(root),
                    '--results', str(root / 'results')]), \
                 patch.object(subprocess, 'check_output', side_effect=AssertionError('must reject before GPU checks')), \
                 contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as error:
                pod_gate.main()
            self.assertEqual(error.exception.code, 2)


class P6cBenchmarkTests(unittest.TestCase):
    def test_rotation_and_warmup_schedule(self):
        self.assertEqual(p6c_driver.rotation_order(3, 0), [0, 1, 2])
        self.assertEqual(p6c_driver.rotation_order(3, 1), [1, 2, 0])
        self.assertEqual(p6c_driver.rotation_order(3, 2), [2, 0, 1])
        plan = p6c_driver.schedule(3, ['sf1', 'sf10'], 3)
        self.assertEqual(len(plan), 24)
        for r in range(-1, 3):
            for scale in ('sf1', 'sf10'):
                group = [item for item in plan if item['round'] == r and item['scale'] == scale]
                self.assertEqual([item['bundle'] for item in group],
                                 p6c_driver.rotation_order(3, max(r, 0)))
                self.assertEqual([item['warmup'] for item in group], [r < 0] * 3)

    def test_bootstrap_exact_and_seeded_sampling(self):
        result = p6c_analysis.bootstrap_ratios([1, 4])
        self.assertAlmostEqual(result['geometric_mean'], 2)
        self.assertEqual(result['resamples'], 4)
        self.assertEqual(result['method'], 'exhaustive')
        self.assertLess(result['ci95'][0], 2)
        self.assertGreater(result['ci95'][1], 2)
        constant = p6c_analysis.bootstrap_ratios([0.5] * 3)
        self.assertEqual(constant['ci95'], [0.5, 0.5])
        sampled = p6c_analysis.bootstrap_ratios(list(range(1, 9)), seed=7)
        self.assertEqual(sampled['resamples'], 20000)
        self.assertEqual(sampled['method'], 'monte_carlo')
        self.assertEqual(sampled, p6c_analysis.bootstrap_ratios(list(range(1, 9)), seed=7))
        self.assertIsNone(p6c_analysis.bootstrap_ratios([])['ci95'])

    def test_parsing_retains_exactness_samples_and_nested_transfer(self):
        row = dict(query='h06', exact=True, answer_rows=1, median_ms=dict(total=2, kernels=1, build=1),
                   samples=[dict(total_ms=2)], load_ms=3, inputs=[dict(transfer_bytes=64)])
        parsed = p6c_driver.parse_results(dict(results=[row]), ['h06'])['h06']
        self.assertTrue(parsed['exact'])
        self.assertEqual(parsed['samples'], row['samples'])
        self.assertEqual(parsed['median_ms'], row['median_ms'])
        self.assertEqual(parsed['load_transfer']['load_ms'], 3)
        self.assertEqual(parsed['load_transfer']['inputs']['0']['transfer_bytes'], 64)
        with self.assertRaises(ValueError):
            p6c_driver.parse_results(dict(results=[]), ['h06'])

    def test_analysis_pairs_within_sessions_and_reports_control(self):
        with tempfile.TemporaryDirectory() as directory:
            paths = []
            for session_index in range(2):
                records = []
                for r in range(-1, 2):
                    for index, factor in enumerate((1, 0.5, 1.02)):
                        value = (10 + session_index * 20 + r) * factor
                        records.append(dict(round=r, warmup=r < 0, bundle=index, scale='sf1',
                            process_seconds=value, exit_code=0, exact=True, failed=False,
                            rusage_delta=dict(minflt=100 + r, majflt=r + 1, maxrss=0),
                            per_query={'h06': dict(exact=True, median_ms=dict(total=value, kernels=value))}))
                path = Path(directory) / f'{session_index}.json'
                path.write_text(json.dumps(dict(bundles=[{}, {}, {}], mode='advised', processes=records)))
                paths.append(path)
            report = p6c_analysis.analyze(paths, control=2)
            rows = [row for row in report['rows'] if row['bundle'] == 1]
            self.assertEqual(len(rows), 3)
            for row in rows:
                self.assertEqual(row['pairs'], 4)
                self.assertEqual(row['geometric_mean'], 0.5)
                self.assertAlmostEqual(row['control_ci95'][0], 1.02)
                self.assertEqual(row['minflt_range'], [100, 101])
                self.assertEqual({pair['input'] for pair in row['paired_ratios']}, {0, 1})
            self.assertIn('layout-control', p6c_analysis.markdown(report))

    def test_driver_budget_failure_and_per_process_checkpoint(self):
        for budget, fail, expected_count in ((100, False, 18), (8, False, 2), (100, True, 18)):
            with self.subTest(budget=budget, fail=fail), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                directories = [root / f'bundle-{index}' for index in range(3)]
                for bundle in directories:
                    bundle.mkdir()
                    (bundle / 'manifest.json').write_text(json.dumps(dict(
                        commit='same-commit', worktree=str(root / 'shared-worktree'),
                        binaries={'cq-gpu-tpch': 'bin/cq-gpu-tpch'})))
                clock = [0]
                calls = []
                output = root / 'results/pod-bench.json'

                def run(bundle, result_path, queries, scale, mode, timeout):
                    checkpoint = json.loads(output.read_text())
                    self.assertEqual(len(checkpoint['processes']), len(calls))
                    self.assertEqual(queries, ['h06'])
                    self.assertEqual(mode, 'advised')
                    self.assertGreater(timeout, 0)
                    calls.append((directories.index(bundle.dir), scale))
                    clock[0] += 2
                    return dict(process_seconds=2, failed=fail, timed_out=False)

                with patch.object(sys, 'argv', ['driver', '--results', str(output.parent),
                        '--queries', 'h06', '--scales', 'sf1,sf10', '--pairs', '2',
                        '--budget', str(budget)]), \
                     patch.dict(os.environ, {'BUNDLE_DIR': str(directories[-1]),
                         'BUNDLE_DIRS': json.dumps(list(map(str, directories)))}), \
                     patch.object(p6c_driver.time, 'monotonic', side_effect=lambda: clock[0]), \
                     patch.object(p6c_driver, 'gpu_state', return_value={'gpu': 'test-state'}), \
                     patch.object(p6c_driver, 'run_process', side_effect=run), \
                     patch.object(pod_gate, 'mirror') as mirror:
                    self.assertEqual(p6c_driver.main(), int(fail))
                result = json.loads(output.read_text())
                self.assertEqual(len(result['processes']), expected_count)
                self.assertEqual(len(result['not_run']), 18 - expected_count)
                expected = p6c_driver.schedule(3, ['sf1', 'sf10'], 2)
                self.assertEqual(calls, [(item['bundle'], item['scale']) for item in expected[:expected_count]])
                self.assertEqual(mirror.call_count, expected_count + 1)
                self.assertTrue(all('before' in row and 'after' in row for row in result['rounds']))
                self.assertEqual(result['stop_reason'] is not None, budget == 8)

    def test_driver_defers_before_loading_nonlast_bundle(self):
        with patch.object(sys, 'argv', ['driver', '--results', '/unused', '--budget', '10']), \
             patch.dict(os.environ, {'BUNDLE_DIR': '/bundle-0',
                 'BUNDLE_DIRS': json.dumps(['/bundle-0', '/bundle-1'])}), \
             patch.object(p6c_driver, 'Bundle', side_effect=AssertionError('must defer')), \
             contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(p6c_driver.main(), 0)

    def test_process_records_failure_resources_and_results(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'manifest.json').write_text(json.dumps(dict(worktree=str(root),
                binaries={'cq-gpu-tpch': 'bin/cq-gpu-tpch'})))
            bundle = p6c_driver.Bundle(root)
            output = root / 'query.json'
            for code, exact in ((0, True), (1, True), (0, False)):
                output.write_text(json.dumps(dict(results=[dict(query='h06', exact=exact,
                    answer_rows=1, samples=[], median_ms=dict(total=1, build=0, kernels=1))])))
                with self.subTest(code=code, exact=exact), \
                     patch.object(p6c_driver.subprocess, 'Popen') as popen:
                    popen.return_value.communicate.return_value = ('stdout', 'stderr')
                    popen.return_value.returncode = code
                    record = p6c_driver.run_process(bundle, output, ['h06'], 'sf1', 'advised', 10)
                self.assertEqual(record['failed'], code != 0 or not exact)
                self.assertEqual(record['exit_code'], code)
                self.assertEqual(record['exact'], exact)
                self.assertEqual(record['results_json'], str(output))
                self.assertEqual(set(record['rusage_delta']), {'minflt', 'majflt', 'maxrss'})
                self.assertEqual(record['argv'][-6:], ['--query', 'h06', '--scale', 'sf1', '--mode', 'advised'])
                self.assertEqual(bundle.env['DUCKDB_CLI'], str(root / 'bin/duckdb'))
                self.assertEqual(bundle.env['LD_LIBRARY_PATH'].split(':')[0], str(root / 'lib'))


class BenchmarkTests(unittest.TestCase):
    def test_python_argv_resolves_only_bundled_source(self):
        manifest = dict(files={'source/driver.py': 'hash'}, binaries={'bench': 'bin/bench'})
        bundle = Path('/bundle')
        self.assertEqual(pod_gate.benchmark_argv(bundle, manifest,
                         ['python3', 'source/driver.py', 'literal ; $value']),
                         ['python3', '/bundle/source/driver.py', 'literal ; $value'])
        self.assertEqual(pod_gate.benchmark_argv(bundle, manifest, ['bench', 'arg']),
                         ['/bundle/bin/bench', 'arg'])
        for argv in [[], ['python3'], ['python3', '/source/driver.py'],
                     ['python3', 'source/../driver.py'], ['python3', 'source/missing.py']]:
            with self.subTest(argv=argv), self.assertRaises(ValueError):
                pod_gate.benchmark_argv(bundle, manifest, argv)

    def test_python_runner_preserves_bundle_environment_and_literal_argv(self):
        with tempfile.TemporaryDirectory() as directory:
            bundle = Path(directory)
            (bundle / 'source').mkdir()
            script = bundle / 'source/driver.py'
            script.write_text('import json,os,sys\n'
                'from pathlib import Path\n'
                'Path(sys.argv[1]).write_text(json.dumps(dict(argv=sys.argv[2:], '
                'bundle=os.environ["BUNDLE_DIR"], bundles=json.loads(os.environ["BUNDLE_DIRS"]), '
                'gpu=os.environ["CUDA_VISIBLE_DEVICES"])))\n')
            worktree = bundle / 'worktree'
            (worktree / 'experiments/gpu').mkdir(parents=True)
            (bundle / 'manifest.json').write_text(json.dumps(dict(
                commit='test', worktree=str(worktree), binaries={},
                files={'source/driver.py': hashlib.sha256(script.read_bytes()).hexdigest()})))
            results = bundle / 'results'
            siblings = [str(bundle), str(bundle.parent / 'bundle-1')]
            literal = '; touch NOT_A_SHELL_COMMAND'
            real_is_dir = Path.is_dir
            with patch.object(sys, 'argv', ['pod-gate', '--bundle', str(bundle),
                    '--results', str(results), '--existing-mirrors', '--bench', 'python3',
                    'source/driver.py', '{results}/driver.json', literal]), \
                 patch.dict(os.environ, {'BUNDLE_DIRS': json.dumps(siblings)}), \
                 patch.object(subprocess, 'check_output', side_effect=lambda argv, **kwargs:
                     'CUDA Version: 13.0' if argv == ['nvidia-smi'] else '0, 8.9\n'), \
                 patch.object(Path, 'is_dir', lambda path:
                     str(path) in ('/data/tpch-gpu', '/data/doop-bench/doop-data') or real_is_dir(path)):
                self.assertEqual(pod_gate.main(), 0)
            recorded = json.loads((results / 'driver.json').read_text())
            self.assertEqual(recorded, dict(argv=[literal], bundle=str(bundle), bundles=siblings, gpu='0'))

    def test_gate_indices_validate_before_provisioning(self):
        self.assertEqual(session.gate_bundle_indices('all', 2), {0, 1})
        self.assertEqual(session.gate_bundle_indices('none', 2), set())
        self.assertEqual(session.gate_bundle_indices('0,2', 3), {0, 2})
        for value in ('', '-1', '2', '0,x'):
            with self.subTest(value=value), self.assertRaises(ValueError):
                session.gate_bundle_indices(value, 2)


class SessionTests(unittest.TestCase):
    def exercise(self, mode, options=(), bundle_count=1, benchmark_only=False, reject_gate=False):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            key, token, bundle, manifest = [root / name for name in ['key', 'token', 'bundle.tar.zst', 'SHA256SUMS']]
            for path in [key, Path(str(key) + '.pub'), token, bundle, manifest]:
                path.write_text('private-test-token' if path == token else 'fixture')
            bundles = [bundle]
            for index in range(1, bundle_count):
                extra = root / f'bundle-{index}.tar.zst'
                extra.write_text('fixture')
                bundles.append(extra)
            executed = []
            deletes, handlers, timer = [], {}, {}

            def check_output(argv, **kwargs):
                if argv[0] == 'hf':
                    return '{"user":"tester"}'
                if argv[0] == 'tar':
                    return json.dumps(dict(binaries={'cq-gpu-tpch': 'bin/cq-gpu-tpch'},
                        files={'source/driver.py': 'hash'}, steps=[] if benchmark_only else [{'name': 'crates'}]))
                if argv[:3] == ['runpodctl', 'pod', 'create'] and '--help' in argv:
                    return '--min-cuda-version --network-volume-id --wait-timeout --env'
                raise AssertionError(argv)

            def run(argv, **kwargs):
                if argv[:3] == ['runpodctl', 'pod', 'delete']:
                    deletes.append(argv[-1])
                    return subprocess.CompletedProcess(argv, 0, '{}', '')
                raise AssertionError(argv)

            class Timer:
                def __init__(self, seconds, callback):
                    self.assert_seconds = seconds
                    timer['callback'] = callback
                    timer['seconds'] = seconds
                def start(self):
                    pass
                def cancel(self):
                    timer['cancelled'] = True

            class Process:
                pid = 12345
                returncode = 0
                def __init__(self, argv, **kwargs):
                    self.argv = argv
                    executed.append(argv)
                def poll(self):
                    return None
                def communicate(self, **kwargs):
                    argv = self.argv
                    if argv[:3] == ['runpodctl', 'pod', 'create']:
                        # Mimic a pod response containing its environment; the
                        # persisted create log must discard the secret.
                        return '{"id":"test-pod","env":{"HF_TOKEN":"private-test-token"}}', ''
                    if argv[:3] == ['runpodctl', 'ssh', 'info']:
                        return '{"ip":"127.0.0.1","port":2222}', ''
                    if argv[0] == 'ssh' and 'seed_volume.sh' in argv[-1]:
                        if mode == 'failure':
                            self.returncode = 1
                        elif mode == 'interrupt':
                            handlers[signal.SIGINT](signal.SIGINT, None)
                            self.returncode = -15
                        elif mode == 'cap':
                            timer['callback']()
                            self.returncode = -15
                    return '', ''

            argv = ['session', '--volume', 'volume', '--data-center', 'DC', '--gpu-ids', 'GPU',
                    '--revision', 'a' * 40, '--data-manifest', str(manifest), '--ssh-key', str(key),
                    *options, *map(str, bundles)]
            with patch.object(sys, 'argv', argv), \
                 patch.dict(os.environ, {'HF_TOKEN_PATH': str(token)}), \
                 patch.object(session, '__file__', str(root / 'tools/remote_gpu/session.py')), \
                 patch.object(subprocess, 'check_output', side_effect=check_output), \
                 patch.object(subprocess, 'run', side_effect=run), \
                 patch.object(subprocess, 'Popen', Process), \
                 patch.object(session.threading, 'Timer', Timer), \
                 patch.object(signal, 'signal', side_effect=lambda sig, handler: handlers.update({sig: handler})), \
                 patch.object(os, 'killpg'), contextlib.redirect_stdout(io.StringIO()), \
                 contextlib.redirect_stderr(io.StringIO()):
                if reject_gate:
                    with self.assertRaises(SystemExit) as error:
                        session.main()
                    self.assertEqual(error.exception.code, 2)
                    self.assertEqual(executed, [])
                    self.assertEqual(timer, {})
                    return None, None
                code = session.main()
            self.assertEqual(deletes, ['test-pod'])
            self.assertEqual(timer['seconds'], 3600)
            self.assertTrue(timer['cancelled'])
            logs = list((root / 'target/remote-gpu').glob('*/session.json'))
            state = json.loads(logs[0].read_text())
            self.assertTrue(state['deleted'])
            for log in logs[0].parent.iterdir():
                if log.is_file():
                    self.assertNotIn('private-test-token', log.read_text())
            self.executed = executed
            return code, state

    def test_success_deletes(self):
        code, state = self.exercise('success')
        self.assertEqual(code, 0)
        self.assertEqual(state['not_run'], [])

    def test_failed_seed_deletes_and_reports_unrun_gate(self):
        code, state = self.exercise('failure')
        self.assertEqual(code, 1)
        self.assertEqual(state['not_run'], ['0/gate'])
        self.assertEqual(state['running'], 'seed')

    def test_ctrl_c_deletes(self):
        code, state = self.exercise('interrupt')
        self.assertEqual(code, 1)

    def test_hard_cap_deletes_mid_step(self):
        code, state = self.exercise('cap')
        self.assertEqual(code, 1)
        self.assertTrue(state['capped'])

    def test_selective_gate_keeps_python_benches_and_sibling_paths(self):
        code, state = self.exercise('success', options=[
            '--gate-bundles', '0', '--bench', 'python3 source/driver.py --results {results}'],
            bundle_count=2)
        self.assertEqual(code, 0)
        self.assertEqual(state['completed'], ['seed', '0/gate', '0/bench-0', '1/bench-0'])
        self.assertEqual(state['gate_bundles'], [0])
        self.assertEqual([Path(path).name for path in state['bundle_dirs']], ['bundle-0', 'bundle-1'])
        calls = [shlex.split(argv[-1]) for argv in self.executed
                 if argv[0] == 'ssh' and '/tools/pod_gate.sh' in argv[-1]]
        self.assertEqual(len(calls), 3)
        for argv in calls:
            self.assertEqual(argv[:2], ['env', 'BUNDLE_DIRS=' + json.dumps(state['bundle_dirs'])])
        benches = [argv for argv in calls if '--bench' in argv]
        self.assertEqual(len(benches), 2)
        for argv in benches:
            position = argv.index('--bench')
            self.assertEqual(argv[position + 1:position + 4], ['python3', 'source/driver.py', '--results'])
            self.assertNotIn('{results}', argv[-1])

    def test_none_skips_every_gate_but_runs_named_benchmarks(self):
        code, state = self.exercise('success', options=[
            '--gate-bundles', 'none', '--bench', 'cq-gpu-tpch output.json'], bundle_count=2)
        self.assertEqual(code, 0)
        self.assertEqual(state['completed'], ['seed', '0/bench-0', '1/bench-0'])
        self.assertEqual(state['gate_bundles'], [])

    def test_default_still_gates_all_bundles(self):
        code, state = self.exercise('success', bundle_count=2)
        self.assertEqual(code, 0)
        self.assertEqual(state['completed'], ['seed', '0/gate', '1/gate'])

    def test_benchmark_only_archive_is_rejected_before_provisioning(self):
        self.exercise('success', benchmark_only=True, reject_gate=True)

    def test_benchmark_only_archive_runs_when_gate_is_excluded(self):
        code, state = self.exercise('success', benchmark_only=True, options=[
            '--gate-bundles', 'none', '--bench', 'cq-gpu-tpch output.json'])
        self.assertEqual(code, 0)
        self.assertEqual(state['completed'], ['seed', '0/bench-0'])


if __name__ == '__main__':
    unittest.main()
