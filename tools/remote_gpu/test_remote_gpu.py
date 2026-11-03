#!/usr/bin/env python3
"""Offline tests: no RunPod calls, SSH connections, or GPU execution."""
import contextlib
import hashlib
import io
import json
import os
from pathlib import Path
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


class SessionTests(unittest.TestCase):
    def exercise(self, mode):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            key, token, bundle, manifest = [root / name for name in ['key', 'token', 'bundle.tar.zst', 'SHA256SUMS']]
            for path in [key, Path(str(key) + '.pub'), token, bundle, manifest]:
                path.write_text('private-test-token' if path == token else 'fixture')
            deletes, handlers, timer = [], {}, {}

            def check_output(argv, **kwargs):
                if argv[0] == 'hf':
                    return '{"user":"tester"}'
                if argv[0] == 'tar':
                    return b'{"binaries":{"cq-gpu-tpch":"bin/cq-gpu-tpch"}}'
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
                    '--revision', 'a' * 40, '--data-manifest', str(manifest), '--ssh-key', str(key), str(bundle)]
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


if __name__ == '__main__':
    unittest.main()
