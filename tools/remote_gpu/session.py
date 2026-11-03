#!/usr/bin/env python3
"""One local owner for a disposable pod, including a hard 60-minute watchdog."""
import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import shlex
import signal
import subprocess
import sys
import threading
import time
from pod_gate import benchmark_argv

# Verified Docker Hub amd64 manifest, 653 MB compressed. CUDA 13 runtime
# dependencies come from the bundle, not a multi-GB CUDA development image.
IMAGE = 'runpod/base:1.6.0-ubuntu2204@sha256:5d921d4125918744ea3f9c5ba63e401f1cfd43ed3fdc582d37db8142fea76adc'
CAP_SECONDS = 3600


def gate_bundle_indices(value, count):
    if value == 'all':
        return set(range(count))
    if value == 'none':
        return set()
    try:
        indices = {int(index) for index in value.split(',')}
    except ValueError:
        raise ValueError('--gate-bundles requires all, none, or comma-separated zero-based indices') from None
    if any(index < 0 or index >= count for index in indices):
        raise ValueError('--gate-bundles index is outside the supplied bundle list')
    return indices


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--volume', required=True)
    parser.add_argument('--data-center', required=True)
    parser.add_argument('--gpu-ids', required=True, help='Comma-separated GPU ids in preference order')
    parser.add_argument('--revision', required=True, help='Immutable 40-character HF dataset commit')
    parser.add_argument('--data-manifest', required=True, type=Path)
    parser.add_argument('--ssh-key', type=Path, default=Path.home() / '.ssh/id_ed25519')
    parser.add_argument('--repo', help='Defaults to <hf auth whoami username>/mini-linq-gpu-data')
    parser.add_argument('--bench', action='append', default=[], help='Shell-quoted argv, NOT a shell command; repeatable, {results} is expanded')
    parser.add_argument('--gate-bundles', default='all', help='Gate all bundles (default), none, or comma-separated zero-based indices')
    parser.add_argument('bundles', nargs='+', type=Path)
    args = parser.parse_args()
    try:
        gate_indices = gate_bundle_indices(args.gate_bundles, len(args.bundles))
    except ValueError as error:
        parser.error(str(error))
    # All prerequisites are checked before the first billable API call.
    for path in [args.data_manifest, args.ssh_key, Path(str(args.ssh_key) + '.pub'), *args.bundles]:
        if not path.is_file():
            parser.error(f'Missing local prerequisite: {path}')
    import re
    if not re.fullmatch(r'[0-9a-fA-F]{40}', args.revision):
        parser.error('--revision must be an immutable HF commit id')
    hf_home = Path(os.environ.get('HF_HOME', str(Path(os.environ.get('XDG_CACHE_HOME', str(Path.home() / '.cache'))) / 'huggingface')))
    token_path = Path(os.environ.get('HF_TOKEN_PATH', str(hf_home / 'token')))
    token = token_path.read_text().strip()  # Explicitly use the local hf token file.
    if not token:
        parser.error(f'Empty HF token file: {token_path}')
    auth_env = {**os.environ, 'HF_TOKEN': token}
    identity = json.loads(subprocess.check_output(['hf', 'auth', 'whoami', '--json'], env=auth_env, text=True))
    username = identity.get('name') or identity.get('username') or identity.get('user')
    if not isinstance(username, str) or not username:
        raise RuntimeError('hf auth whoami returned no username')
    repo = args.repo or username + '/mini-linq-gpu-data'
    public_key = Path(str(args.ssh_key) + '.pub').read_text().strip()
    benches = [shlex.split(command) for command in args.bench]
    if any(not command for command in benches):
        parser.error('Empty --bench command')
    for bundle in args.bundles:
        packaged = json.loads(subprocess.check_output(
            ['tar', '--zstd', '-xOf', str(bundle.resolve()), './manifest.json']))
        for argv in benches:
            try:
                benchmark_argv(Path('/bundle'), packaged, argv)
            except ValueError as error:
                parser.error(f'{bundle}: {error}')
    # Check the available resource command surface without creating anything.
    help_text = subprocess.check_output(['runpodctl', 'pod', 'create', '--help'], text=True)
    for flag in ['--min-cuda-version', '--network-volume-id', '--wait-timeout', '--env']:
        if flag not in help_text:
            raise RuntimeError(f'Installed runpodctl lacks {flag}')
    stamp = datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S.%fZ')
    root = Path(__file__).resolve().parents[2]
    results = root / 'target/remote-gpu' / stamp
    results.mkdir(parents=True)
    remote = '/workspace/remote-gpu/' + stamp
    bundle_dirs = [remote + '/bundle-' + str(i) for i in range(len(args.bundles))]
    pod_name = 'mini-linq-gpu-' + stamp
    bundle_steps = [
        ([('gate', [])] if i in gate_indices else []) +
        [('bench-' + str(j), ['--bench', *argv]) for j, argv in enumerate(benches)]
        for i in range(len(args.bundles))]
    planned = ['seed'] + [f'{i}/{step}' for i, steps in enumerate(bundle_steps) for step, _ in steps]
    state = dict(image=IMAGE, repo=repo, revision=args.revision, pod_id=None, completed=[],
                 gate_bundles=sorted(gate_indices), bundle_dirs=bundle_dirs,
                 not_run=planned.copy(), running=None, errors=[], capped=False, deleted=False)
    pod_id = None
    active = None
    creating = False
    cancelled = False
    connection = None
    began = time.monotonic()
    deadline = began + CAP_SECONDS
    deletion_lock = threading.Lock()

    def save():
        (results / 'session.json').write_text(json.dumps(state, indent=2) + '\n')

    def remaining():
        return max(0.01, deadline - time.monotonic())

    def stop_active():
        if active is not None and active.poll() is None and not creating:
            os.killpg(active.pid, signal.SIGTERM)

    def interrupt(signum, frame):
        nonlocal cancelled
        cancelled = True
        stop_active()
        if creating:
            # --wait can be waiting for SSH after scheduling a billable pod.
            # Recover by our unique name and delete without waiting for SSH.
            threading.Thread(target=delete, daemon=True).start()

    def recover_id():
        nonlocal pod_id
        if pod_id:
            return
        # --wait failures carry an id on stderr, including startup timeouts.
        for path in sorted(results.glob('create-*.*')):
            for line in path.read_text().splitlines():
                try:
                    item = json.loads(line)
                except ValueError:
                    continue
                if isinstance(item, dict) and item.get('id'):
                    pod_id = item['id']
                    return
        # A broken transport can lose create's response. Recover only our unique
        # session name; never delete another user's pod.
        response = subprocess.run(['runpodctl', 'pod', 'list', '--all', '--name', pod_name],
                                  capture_output=True, text=True, timeout=30)
        if response.returncode == 0:
            items = json.loads(response.stdout)
            if isinstance(items, dict):
                items = items.get('pods', [])
            matches = [item for item in items if item.get('name') == pod_name]
            if len(matches) == 1:
                pod_id = matches[0]['id']

    def delete():
        with deletion_lock:
            if state['deleted']:
                return
            try:
                recover_id()
            except (subprocess.SubprocessError, OSError, ValueError) as error:
                state['errors'].append('Pod id recovery failed: ' + str(error))
            if not pod_id:
                state['errors'].append('Check for a surviving pod named ' + pod_name)
                save()
                return
            state['pod_id'] = pod_id
            # Persist deletion evidence without pod env (which contains a token).
            for attempt in range(3):
                try:
                    response = subprocess.run(['runpodctl', 'pod', 'delete', pod_id],
                                              capture_output=True, text=True, timeout=30)
                    deleted = response.returncode == 0 or '"not_found"' in response.stderr
                except (subprocess.SubprocessError, OSError):
                    deleted = False
                if deleted:
                    state['deleted'] = True
                    state['billed_minutes_upper_bound'] = (time.monotonic() - began) / 60
                    print(f"Pod {pod_id} deleted; billed minutes <= {state['billed_minutes_upper_bound']:.2f} "
                          '(create-to-delete wall clock; not an invoice)', flush=True)
                    save()
                    return
                time.sleep(2 ** attempt)
            state['errors'].append('DELETE FAILED: manually delete pod ' + pod_id)
            save()
            print('CRITICAL: failed to delete pod ' + pod_id, file=sys.stderr, flush=True)

    def cap():
        nonlocal cancelled
        cancelled = True
        state['capped'] = True
        stop_active()
        delete()  # Independent of the SSH process, even if its remote step hangs.

    def command(argv, name, *, check=True, timeout=None, secret=False):
        nonlocal active
        if cancelled:
            raise RuntimeError('Session cancelled or reached its 60-minute cap')
        start = time.monotonic()
        # Never log create argv/stdout: pod records can contain HF_TOKEN.
        active = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                  text=True, start_new_session=True)
        try:
            stdout, stderr = active.communicate(timeout=min(timeout or remaining(), remaining()))
        except subprocess.TimeoutExpired:
            os.killpg(active.pid, signal.SIGKILL)
            stdout, stderr = active.communicate()
            raise RuntimeError(f'{name} timed out')
        finally:
            print(f'Gate time {name}: {time.monotonic() - start:.3f}s', flush=True)
        code = active.returncode
        active = None
        if secret:
            # Preserve only id/error code, never the environment or token.
            for suffix, text in [('stdout', stdout), ('stderr', stderr)]:
                try:
                    parsed = json.loads(text)
                    clean = {key: parsed[key] for key in ('id', 'code') if key in parsed}
                except ValueError:
                    clean = {}
                (results / (name + '.' + suffix)).write_text(json.dumps(clean) + '\n')
        else:
            (results / (name + '.stdout')).write_text(stdout)
            (results / (name + '.stderr')).write_text(stderr)
        if check and code:
            raise RuntimeError(f'{name} failed ({code}); see {results}')
        return code, stdout, stderr

    def ssh(argv, name, **kwargs):
        return command([*connection, shlex.join(argv)], name, **kwargs)

    signal.signal(signal.SIGINT, interrupt)
    signal.signal(signal.SIGTERM, interrupt)
    timer = threading.Timer(CAP_SECONDS, cap)
    timer.daemon = True
    timer.start()
    save()
    exit_code = 0
    try:
        for index, gpu in enumerate(args.gpu_ids.split(',')):
            creating = True
            try:
                code, stdout, stderr = command(['runpodctl', 'pod', 'create', '--name', pod_name,
                    '--image', IMAGE, '--gpu-id', gpu.strip(), '--gpu-count', '1',
                    '--data-center-ids', args.data_center, '--network-volume-id', args.volume,
                    '--volume-mount-path', '/workspace', '--min-cuda-version', '13.0',
                    '--ports', '22/tcp', '--ssh', '--container-disk-in-gb', '10',
                    '--env', json.dumps(dict(HF_TOKEN=token, PUBLIC_KEY=public_key,
                        NVIDIA_VISIBLE_DEVICES='all', NVIDIA_DRIVER_CAPABILITIES='compute,utility')),
                    '--wait', '--wait-timeout', '10m'], f'create-{index}', check=False, secret=True, timeout=660)
            finally:
                creating = False
            recover_id()
            if pod_id:
                state['pod_id'] = pod_id
                save()
                if code:
                    raise RuntimeError('Created pod did not become SSH-ready')
                break
            if code == 0:
                raise RuntimeError('Create succeeded without an identifiable pod')
            try:
                failure_code = json.loads(stderr).get('code')
            except ValueError:
                failure_code = None
            if failure_code not in ('bad_request', 'not_found', 'cli_error'):
                # Never create a second pod after an ambiguous transport or
                # server failure: the first request may already have succeeded.
                raise RuntimeError('Ambiguous/unauthorized create failure; refusing another GPU attempt')
        if not pod_id:
            raise RuntimeError('None of the requested GPUs could be provisioned')
        _, info, _ = command(['runpodctl', 'ssh', 'info', pod_id], 'ssh-info', timeout=30)
        info = json.loads(info)
        host, port = 'root@' + info['ip'], str(info['port'])
        options = ['-i', str(args.ssh_key.resolve()), '-o', 'BatchMode=yes', '-o', 'ConnectTimeout=10',
                   '-o', 'StrictHostKeyChecking=accept-new', '-o', 'UserKnownHostsFile=' + str(results / 'known_hosts')]
        connection = ['ssh', *options, '-p', port, host]
        ssh(['mkdir', '-p', remote + '/results'], 'remote-mkdir')
        command(['scp', *options, '-P', port, str(args.data_manifest.resolve()),
                 host + ':' + remote + '/SHA256SUMS'], 'copy-data-manifest')
        for index, bundle in enumerate(args.bundles):
            command(['scp', *options, '-P', port, str(bundle.resolve()),
                     host + ':' + remote + '/' + str(index) + '.tar.zst'], f'copy-bundle-{index}')
            directory = bundle_dirs[index]
            ssh(['mkdir', '-p', directory], f'mkdir-bundle-{index}')
            ssh(['tar', '--zstd', '-xf', remote + '/' + str(index) + '.tar.zst', '-C', directory], f'unpack-{index}')
        first_tools = remote + '/bundle-0/tools'
        # sshd does not guarantee propagation of the container's environment.
        # Read HF_TOKEN from PID 1's environment remotely, without printing it.
        seed = ('import os,subprocess; '
                'env=dict(os.environ); '
                'env.update(x.decode().split("=",1) for x in open("/proc/1/environ","rb").read().split(b"\\0") if x.startswith(b"HF_TOKEN=")); '
                'subprocess.run(' + repr(['bash', first_tools + '/seed_volume.sh', repo, args.revision,
                                           remote + '/SHA256SUMS']) + ',env=env,check=True)')
        state['running'] = 'seed'
        state['not_run'].remove('seed')
        save()
        ssh(['python3', '-c', seed], 'seed')
        state['completed'].append('seed')
        state['running'] = None
        save()
        for index in range(len(args.bundles)):
            directory = bundle_dirs[index]
            for step, extra in bundle_steps[index]:
                label = str(index) + '/' + step
                out = remote + '/results/' + str(index) + '-' + step
                argv = ['env', 'BUNDLE_DIRS=' + json.dumps(bundle_dirs),
                        'bash', directory + '/tools/pod_gate.sh', '--results', out,
                        *[arg.replace('{results}', out) for arg in extra]]
                state['running'] = label
                state['not_run'].remove(label)
                save()
                ssh(argv, label.replace('/', '-'))
                state['completed'].append(label)
                state['running'] = None
                save()
                # Pull each finished step immediately; a hard cap need not lose it.
                command(['scp', *options, '-P', port, '-r', host + ':' + remote + '/results/.',
                         str(results / 'pod-results')], 'pull-' + label.replace('/', '-'))
    except Exception as error:
        state['errors'].append(str(error))
        print(str(error), file=sys.stderr)
        exit_code = 1
    finally:
        # On an ordinary failure fetch partial logs before deletion. At the cap,
        # deletion wins; partial logs remain on the persistent network volume.
        if connection and not state['capped'] and not cancelled:
            try:
                command(['scp', *options, '-P', port, '-r', host + ':' + remote + '/results/.',
                         str(results / 'pod-results')], 'pull-final', check=False, timeout=30)
            except Exception as error:
                state['errors'].append(str(error))
        delete()
        timer.cancel()
        state['remote_results'] = remote + '/results'
        save()
        print('Did not run: ' + (', '.join(state['not_run']) or '(none)'))
        if state['running']:
            print('Did not finish successfully: ' + state['running'])
        print('Results: ' + str(results))
    return exit_code or int(cancelled or bool(state['errors']))


if __name__ == '__main__':
    raise SystemExit(main())
