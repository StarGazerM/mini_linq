#!/usr/bin/env python3
"""Build relocatable default-program sm_89 DOOP assets without GPU execution.

Run with the SRDatalog checkout's .venv available. Re-running uses its JIT cache;
--reuse-builds repackages validated existing libraries and optional ours binaries.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import tarfile
import time


DATASETS = ('bloat', 'xalan', 'batik', 'soot', 'javac', 'jruby')
REVISION = 'fab849f672d43af2a6a1e8a1727ec065201e3720'
SYSTEM = re.compile(r'^(?:linux-vdso|ld-linux|lib(?:c|m|pthread|dl|rt|resolv|util|anl|BrokenLocale|nss_[\w]+)\.so|libcuda\.so)')


def sha256(path):
    with Path(path).open('rb') as stream:
        digest = hashlib.sha256()
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(block)
        return digest.hexdigest()


def capture(argv, **kwargs):
    return subprocess.check_output(list(map(str, argv)), text=True, **kwargs).strip()


def save(path, value):
    temporary = path.with_suffix(path.suffix + '.tmp')
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True) + '\n')
    temporary.replace(path)


def dependencies(artifacts, destination):
    records = {}
    pending = list(artifacts)
    visited = set()
    while pending:
        artifact = Path(pending.pop()).resolve()
        if artifact in visited:
            continue
        visited.add(artifact)
        output = capture(['ldd', artifact])
        for line in output.splitlines():
            words = line.split()
            if not words or SYSTEM.match(words[0]):
                continue
            if 'not found' in line:
                raise RuntimeError(f'Unresolved dependency of {artifact}: {line}')
            if len(words) < 3 or words[1] != '=>':
                continue
            soname, source = words[0], Path(words[2])
            digest = sha256(source)
            if soname in records and records[soname]['sha256'] != digest:
                raise RuntimeError(f'Conflicting dependency {soname}')
            shutil.copy2(source, destination / soname)
            records[soname] = {'path': f'lib/{soname}', 'sha256': digest}
            pending.append(source)
    return records


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--srdatalog', type=Path, default=Path('/home/stargazermiao/workspace/srdatalog-python'))
    parser.add_argument('--prepared', type=Path, default=Path('/data/doop-bench/corpus-20261110/prepared'))
    parser.add_argument('--output', type=Path, default=Path('target/remote-gpu/doop'))
    parser.add_argument('--jobs', type=int, default=2)
    parser.add_argument('--command-timeout', type=int, default=100,
                        help='Per compiler command seconds; 0 for approved long artifact builds')
    parser.add_argument('--cxx', default=os.environ.get('CXX', 'clang++'))
    parser.add_argument('--reuse-builds', action='store_true')
    parser.add_argument('--ours-written', type=Path)
    parser.add_argument('--ours-advised', type=Path)
    args = parser.parse_args()
    root, prepared, output = (p.resolve() for p in (args.srdatalog, args.prepared, args.output))
    revision = capture(['git', '-C', root, 'rev-parse', 'HEAD'])
    if revision != REVISION:
        raise RuntimeError(f'Expected SRDatalog {REVISION}, found {revision}')
    source_status = capture(['git', '-C', root, 'status', '--porcelain', '--untracked-files=no'])
    for directory in ('srdatalog', 'lib', 'build'):
        (output / directory).mkdir(parents=True, exist_ok=True)
    compiler = shutil.which(args.cxx)
    if not compiler:
        raise RuntimeError(f'Compiler not found: {args.cxx}')
    wrapper = output / 'build' / 'locked-cxx'
    wrapper.write_text('#!/bin/sh\nexec flock -s /tmp/mini_linq-timing.lock timeout '
                       + str(args.command_timeout) + ' ' + shlex.quote(compiler) + ' "$@"\n')
    wrapper.chmod(0o755)
    source_hash = sha256(root / 'examples/doop.py')
    manifest = {'schema_version': 1, 'architecture': 'sm_89', 'program': 'default',
                'source': {'revision': revision, 'program_sha256': source_hash,
                           'working_tree_status': source_status,
                           'gpu_driver_sha256': sha256(root / 'examples/doop_suite/gpu.py')},
                'datasets': {}}
    artifacts = []
    for dataset in DATASETS:
        facts, work = prepared / dataset, output / 'build' / dataset
        work.mkdir(exist_ok=True)
        metadata = json.loads((facts / 'manifest.json').read_text())
        metadata_hash = sha256(facts / 'meta.json')
        manifest_hash = sha256(facts / 'manifest.json')
        if not args.reuse_builds:
            command = [str(root / '.venv/bin/python'), str(root / 'examples/doop_suite/gpu.py'),
                       '_build', str(facts), str(work), 'baseline', str(args.jobs)]
            print(json.dumps({'dataset': dataset, 'command': command}), flush=True)
            started = time.monotonic()
            with (work / 'build.log').open('w') as log:
                result = subprocess.run(command, env=dict(os.environ, CXX=str(wrapper)), stdout=log, stderr=subprocess.STDOUT)
            save(work / 'command.json', {'argv': command, 'returncode': result.returncode,
                                       'wall_seconds': time.monotonic() - started})
            if result.returncode:
                raise RuntimeError(f'{dataset} build exited {result.returncode}; see {work / "build.log"}')
        build = json.loads((work / 'build.json').read_text())
        for key, expected in [('source_sha256', source_hash), ('metadata_sha256', metadata_hash),
                              ('input_manifest_sha256', manifest_hash)]:
            if build[key] != expected:
                raise RuntimeError(f'{dataset}: stale build {key}')
        library = Path(build['library'])
        if sha256(library) != build['library_sha256']:
            raise RuntimeError(f'{dataset}: library checksum mismatch')
        inputs = build['input_rows']
        arities = {r['name']: r['arity'] for r in build['relations']}
        if len(arities.keys() - inputs.keys()) != 40:
            raise RuntimeError(f'{dataset}: expected 40 derived relations')
        destination = output / 'srdatalog' / f'{dataset}.so'
        shutil.copy2(library, destination)
        artifacts.append(destination)
        manifest['datasets'][dataset] = {
            'prepared_dir': f'prepared/{dataset}', 'library': f'srdatalog/{dataset}.so',
            'library_sha256': sha256(destination), 'input_cardinalities': inputs,
            'relation_arities': arities, 'prepared_metadata': {
                'manifest': metadata, 'manifest_sha256': manifest_hash, 'metadata_sha256': metadata_hash}}
        print(f'{dataset}: {destination}', flush=True)
    binaries = {}
    for name, source in [('ours-written', args.ours_written), ('ours-advised', args.ours_advised)]:
        if source is not None:
            destination = output.parent / 'bin' / name
            destination.parent.mkdir(exist_ok=True)
            if source.resolve() != destination.resolve():
                shutil.copy2(source, destination)
            artifacts.append(destination)
            binaries[name] = {'path': f'bin/{name}', 'sha256': sha256(destination)}
    manifest['binaries'] = binaries
    manifest['dependencies'] = dependencies(artifacts, output / 'lib')
    archive = output / 'prepared.tar.zst'
    # Relative archive members allow extraction at any mounted volume root.
    command = ['tar', '--sort=name', '--mtime=@0', '--owner=0', '--group=0', '--numeric-owner',
               '-I', 'zstd -T2', '-cf', str(archive), '-C', str(prepared.parent),
               *[f'{prepared.name}/{name}' for name in DATASETS]]
    if prepared.name != 'prepared':
        command[1:1] = ['--transform', f's,^{re.escape(prepared.name)}/,prepared/,']
    subprocess.run(['flock', '-s', '/tmp/mini_linq-timing.lock', 'timeout', '100', *command], check=True)
    manifest['prepared_archive'] = {'path': archive.name, 'sha256': sha256(archive),
                                    'size_bytes': archive.stat().st_size}
    source_archive = output / 'srdatalog-source.tar'
    tracked = capture(['git', '-C', root, 'ls-files', '-z']).split('\0')
    source_files = {}
    with tarfile.open(source_archive, 'w') as tar:
        for name in sorted(filter(None, tracked)):
            path = root / name
            if path.is_file():
                source_files[name] = sha256(path)
                tar.add(path, arcname=f'srdatalog/{name}', recursive=False)
    manifest['source']['files_sha256'] = source_files
    subprocess.run(['flock', '-s', '/tmp/mini_linq-timing.lock', 'timeout', '100',
                    'zstd', '-T2', '-f', '--rm', str(source_archive)], check=True)
    source_archive = source_archive.with_suffix('.tar.zst')
    manifest['source']['archive'] = {'path': source_archive.name, 'sha256': sha256(source_archive)}
    save(output / 'manifest.json', manifest)
    print(json.dumps({'manifest': str(output / 'manifest.json'), 'datasets': list(DATASETS)}), flush=True)


if __name__ == '__main__':
    main()
