#!/usr/bin/env python3
"""Build the two default DOOP executables and assemble an execution-only bundle."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parents[3]
HPC = Path('/opt/nvidia/hpc_sdk/Linux_x86_64/25.9')


def sha256(path):
    digest = hashlib.sha256()
    with path.open('rb') as source:
        for block in iter(lambda: source.read(1024 * 1024), b''):
            digest.update(block)
    return digest.hexdigest()


def environment():
    env = dict(os.environ, CARGO_INCREMENTAL='0', CARGO_BUILD_JOBS='12',
               CARGO_PROFILE_DEV_DEBUG='line-tables-only', CARGO_PROFILE_TEST_DEBUG='line-tables-only',
               CUDA_HOME=str(HPC / 'cuda/13.0'),
               BINDGEN_EXTRA_CLANG_ARGS='-I' + str(HPC / 'math_libs/13.0/targets/x86_64-linux/include'),
               LIBRARY_PATH=str(HPC / 'math_libs/13.0/targets/x86_64-linux/lib'),
               LD_LIBRARY_PATH=str(HPC / 'math_libs/13.0/targets/x86_64-linux/lib') + ':' + str(HPC / 'cuda/13.0/lib64'))
    env.pop('CQ_EXPANSION_COUNTERS', None)
    return env


def build(stage, variant):
    features = 'default-query' + (',advice-cost' if variant == 'advised' else '')
    dump = stage / 'plans' / variant
    dump.mkdir(parents=True, exist_ok=True)
    command = ['cargo', '+1.99.1', 'oxide', 'build', '--arch', 'sm_89', '--materialize-cubin',
               '--', '--release', '-p', 'srd-doop-gpu', '--features', features]
    started = time.monotonic()
    env = dict(environment(), CQ_GPU_SEQUENCE_DUMP=str(dump))
    log = stage / 'logs' / (variant + '.log')
    with log.open('w') as stream, open('/tmp/mini_linq-timing.lock', 'a') as lock:
        fcntl.flock(lock, fcntl.LOCK_SH)
        acquired = time.monotonic()
        result = subprocess.run(command, cwd=ROOT / 'experiments/gpu', env=env,
                                stdout=stream, stderr=subprocess.STDOUT)
    record = dict(argv=command, features=features, seconds=time.monotonic() - acquired,
                  lock_wait_seconds=acquired - started, exit_status=result.returncode, log=str(log))
    print(json.dumps(record), flush=True)
    if result.returncode:
        print(log.read_text())
        raise SystemExit(result.returncode)
    target = ROOT / 'experiments/gpu/target/release/srd-doop-gpu'
    shutil.copy2(target, stage / 'bin' / ('ours-' + variant))
    record['sha256'] = sha256(stage / 'bin' / ('ours-' + variant))
    record['advice_sha256'] = sha256(ROOT / 'experiments/gpu/srd-doop/src/advice_cost.dl') if variant == 'advised' else None
    (stage / 'logs' / (variant + '.json')).write_text(json.dumps(record, indent=2) + '\n')


def assemble(stage, assets, output, omit_prepared):
    for name in ('ours-written', 'ours-advised'):
        if not (stage / 'bin' / name).is_file():
            raise RuntimeError('Build first: ' + name)
    if assets.resolve() == (stage / 'doop').resolve():
        raise ValueError('Keep the local compiler asset tree separate from the execution bundle')
    excluded = ['build'] + (['prepared.tar.zst'] if omit_prepared else [])
    shutil.copytree(assets, stage / 'doop', dirs_exist_ok=True,
                    ignore=shutil.ignore_patterns(*excluded))
    for work in sorted((assets / 'build').iterdir()):
        if work.is_dir():
            destination = stage / 'logs' / 'srdatalog' / work.name
            destination.mkdir(parents=True, exist_ok=True)
            for name in ('build.json', 'command.json', 'build.log'):
                if (work / name).is_file():
                    shutil.copy2(work / name, destination / name)
    if omit_prepared and (stage / 'doop/prepared.tar.zst').exists():
        (stage / 'doop/prepared.tar.zst').unlink()
    libraries = {}
    for name in ('ours-written', 'ours-advised'):
        linked = subprocess.check_output(['ldd', str(stage / 'bin' / name)], env=environment(), text=True)
        if 'not found' in linked:
            raise RuntimeError(linked)
        for soname, source in re.findall(r'^\s*(\S+) => (/\S+)', linked, re.M):
            if soname.startswith('libcuda.so') or source.startswith(('/lib/', '/lib64/', '/usr/lib/', '/usr/lib64/')):
                continue
            resolved = Path(source).resolve()
            if soname in libraries and libraries[soname] != resolved:
                raise RuntimeError('Conflicting dependency: ' + soname)
            libraries[soname] = resolved
    for soname, source in libraries.items():
        shutil.copy2(source, stage / 'lib' / soname)
    files = subprocess.check_output(['git', 'ls-files', '-z', '--cached', '--others', '--exclude-standard'], cwd=ROOT).split(b'\0')
    for raw in sorted(set(files)):
        if not raw:
            continue
        relative = Path(os.fsdecode(raw))
        if 'target' in relative.parts or '.git' in relative.parts or not (ROOT / relative).is_file():
            continue
        destination = stage / 'source' / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / relative, destination)
    for source in (ROOT / 'tools/remote_gpu').iterdir():
        if source.suffix in ('.py', '.sh'):
            shutil.copy2(source, stage / 'tools' / source.name)
    if not (stage / 'hf/huggingface_hub').is_dir():
        subprocess.run(['python3', '-m', 'pip', 'install', '--disable-pip-version-check', '--only-binary=:all:',
                        '--target', str(stage / 'hf'), 'huggingface_hub==1.1.5', 'click'], check=True)
    manifest = dict(version=1, worktree=str(ROOT), arch='sm_89',
                    commit=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                    tests=[], steps=[], binaries={name: 'bin/' + name for name in ('ours-written', 'ours-advised')},
                    libraries=[dict(soname=k, source=str(v)) for k, v in libraries.items()],
                    builds=[json.loads((stage / 'logs' / (name + '.json')).read_text()) for name in ('written', 'advised')],
                    cubins='Embedded sm_89 native cubins; execution-only bundle', files={})
    for path in sorted(stage.rglob('*')):
        if path.is_file() and path != stage / 'manifest.json':
            manifest['files'][str(path.relative_to(stage))] = sha256(path)
    (stage / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    output.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(['tar', '--zstd', '-cf', str(output), '-C', str(stage), '.'],
                   env=dict(os.environ, ZSTD_CLEVEL='3'), check=True)
    print(json.dumps(dict(bundle=str(output), bytes=output.stat().st_size, sha256=sha256(output))))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--stage', type=Path, default=ROOT / 'target/remote-gpu/doop-bundle')
    parser.add_argument('--build', choices=['written', 'advised'])
    parser.add_argument('--assets', type=Path, default=ROOT / 'target/remote-gpu/doop')
    parser.add_argument('--output', type=Path)
    parser.add_argument('--omit-prepared', action='store_true', help='Use driver --prepared-archive pointing at a seeded volume archive')
    args = parser.parse_args()
    args.stage = args.stage.resolve()
    for directory in ('bin', 'lib', 'logs', 'source', 'tools'):
        (args.stage / directory).mkdir(parents=True, exist_ok=True)
    if args.build:
        build(args.stage, args.build)
    if args.output:
        assemble(args.stage, args.assets, args.output.resolve(), args.omit_prepared)
    if not args.build and not args.output:
        parser.error('supply --build or --output')


if __name__ == '__main__':
    main()
