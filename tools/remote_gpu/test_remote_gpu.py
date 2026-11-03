#!/usr/bin/env python3
"""Offline tests: no RunPod calls, SSH connections, or GPU execution."""
import contextlib
import hashlib
import io
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

import data_manifest
import pod_gate
import seed_volume
import session


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
    def exercise(self, mode, options=(), bundle_count=1):
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
                    return b'{"binaries":{"cq-gpu-tpch":"bin/cq-gpu-tpch"},"files":{"source/driver.py":"hash"}}'
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


if __name__ == '__main__':
    unittest.main()
