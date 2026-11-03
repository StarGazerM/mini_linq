#!/usr/bin/env python3
"""Build on the workstation; package only execution inputs, never a Cargo cache."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import tempfile
import time

PACKAGES = ['gpu-datalog-runtime', 'cq-gpu-datalog-core', 'cq-gpu-datalog',
            'cq-gpu-sequence-core', 'cq-gpu-sequence', 'cq-gpu-tpch']
HPC = '/opt/nvidia/hpc_sdk/Linux_x86_64/25.9'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('worktree', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--bench', nargs='+', action='extend', default=[])
    parser.add_argument('--no-tests', action='store_true',
                        help='Build a benchmark-only bundle without test harnesses or doctests')
    args = parser.parse_args()
    root, output = args.worktree.resolve(), args.output.absolute()
    workspace = root / 'experiments/gpu'
    output.parent.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env.update(CARGO_INCREMENTAL='0', CARGO_BUILD_JOBS='6', CUDA_HOME=HPC + '/cuda/13.0',
               BINDGEN_EXTRA_CLANG_ARGS='-I' + HPC + '/math_libs/13.0/targets/x86_64-linux/include',
               LIBRARY_PATH=HPC + '/math_libs/13.0/targets/x86_64-linux/lib',
               LD_LIBRARY_PATH=HPC + '/math_libs/13.0/targets/x86_64-linux/lib')
    start = time.monotonic()
    with tempfile.TemporaryDirectory(prefix='remote-gpu-bundle-') as temporary:
        stage = Path(temporary)
        for directory in ['bin', 'lib', 'logs', 'source', 'tools']:
            (stage / directory).mkdir()
        manifest = dict(version=1, worktree=str(root), arch='sm_89',
                        commit=subprocess.check_output(['git', '-C', str(root), 'rev-parse', 'HEAD'], text=True).strip(),
                        builds=[], tests=[], binaries={}, libraries=[],
                        cubins='Embedded native cubins in ELF artifacts; no external cubin or compiler required.')
        metadata = json.loads(subprocess.check_output(
            ['cargo', '+1.99.1', 'metadata', '--format-version=1', '--no-deps'], cwd=workspace, env=env))
        if not Path(metadata['target_directory']).resolve().is_relative_to(root):
            raise RuntimeError('Cargo target directory escapes the worktree; unset CARGO_TARGET_DIR')
        packages = {p['id']: p for p in metadata['packages']}
        runtime_output_dirs = set()
        selected_packages = set(PACKAGES + args.bench + ['doop-gpu', 'srd-doop-gpu'])

        def build(name, command, require_executables=True):
            # Gate acquisition is separate, matching tools/README.md exactly.
            waited = time.monotonic()
            subprocess.run(['flock', '/tmp/mini_linq-timing-gate.lock', 'true'], check=True)
            actual = ['nice', '-n', '10', *command]
            began = time.monotonic()
            with (stage / 'logs' / (name + '.jsonl')).open('w') as stdout, \
                 (stage / 'logs' / (name + '.stderr')).open('w') as stderr:
                result = subprocess.run(['flock', '-s', '/tmp/mini_linq-timing.lock', *actual],
                                        cwd=workspace, env=env, stdout=stdout, stderr=stderr)
            elapsed = time.monotonic() - began
            manifest['builds'].append(dict(name=name, argv=actual, cwd=str(workspace),
                environment={key: env[key] for key in ('CARGO_INCREMENTAL', 'CARGO_BUILD_JOBS',
                    'CUDA_HOME', 'BINDGEN_EXTRA_CLANG_ARGS', 'LIBRARY_PATH', 'LD_LIBRARY_PATH',
                    'RUSTDOCFLAGS') if key in env},
                seconds=elapsed, gate_wait_seconds=began - waited))
            print(f'Gate time {name}: {elapsed:.3f}s (gate wait {began - waited:.3f}s)', flush=True)
            if result.returncode:
                print((stage / 'logs' / (name + '.stderr')).read_text())
                # Keep failed build evidence beside the intended output.
                shutil.copytree(stage / 'logs', Path(str(output) + '.failed-logs'), dirs_exist_ok=True)
                raise SystemExit(result.returncode)
            artifacts = []
            for line in (stage / 'logs' / (name + '.jsonl')).read_text().splitlines():
                if line.startswith('{'):
                    item = json.loads(line)
                    if item.get('reason') == 'compiler-artifact' and item.get('executable'):
                        artifacts.append(item)
                    if item.get('reason') == 'build-script-executed':
                        package = packages.get(item['package_id'])
                        if package and package['name'] in selected_packages:
                            runtime_output_dirs.add(Path(item['out_dir']).resolve())
            if require_executables and not artifacts:
                raise RuntimeError(f'{name}: oxide did not report any executables')
            return artifacts

        oxide = ['cargo', '+1.99.1', 'oxide']
        common = ['--arch', 'sm_89', '--materialize-cubin', '--', '--release', '--message-format=json']
        tests = [] if args.no_tests else build(
            'crates-build', oxide + ['test'] + common + ['--no-run'] +
            [arg for package in PACKAGES for arg in ['-p', package]])
        binaries = sorted(set(['cq-gpu-tpch', 'doop-gpu', 'srd-doop-gpu'] + args.bench))
        built = build('binaries-build', oxide + ['build'] + common +
                      [arg for binary in binaries for arg in ['--bin', binary]])
        executables = []
        for index, artifact in enumerate(tests):
            if not artifact['profile']['test']:
                continue
            package = packages[artifact['package_id']]
            destination = 'bin/test-' + str(index) + '-' + artifact['target']['name']
            shutil.copy2(artifact['executable'], stage / destination)
            manifest['tests'].append(dict(argv=[destination],
                cwd=str(Path(package['manifest_path']).parent), package=package['name']))
            executables.append(Path(artifact['executable']))
        if not args.no_tests and not manifest['tests']:
            raise RuntimeError('No test harnesses were enumerated')
        for artifact in built:
            name = artifact['target']['name']
            if name not in binaries:
                continue
            destination = 'bin/' + name
            shutil.copy2(artifact['executable'], stage / destination)
            manifest['binaries'][name] = destination
            executables.append(Path(artifact['executable']))
        if set(manifest['binaries']) != set(binaries):
            raise RuntimeError('Missing requested binary artifacts')

        # Cargo --no-run omits doctests. rustdoc has its own compile-only mode;
        # persist those executables locally rather than invoking a test runner.
        # Keep package directories separate because rustdoc names can collide.
        for package_name in ([] if args.no_tests else PACKAGES):
            package = next(p for p in packages.values() if p['name'] == package_name)
            if not any(t.get('doctest') for t in package['targets']):
                continue
            persisted = stage / 'doctests' / package_name
            env['RUSTDOCFLAGS'] = '-Z unstable-options --no-run --persist-doctests ' + str(persisted)
            build('doctests-' + package_name,
                  oxide + ['test'] + common + ['--doc', '-p', package_name],
                  require_executables=False)
            for path in sorted(persisted.rglob('*')):
                if path.is_file() and os.access(path, os.X_OK):
                    manifest['tests'].append(dict(argv=[str(path.relative_to(stage))],
                        cwd=str(Path(package['manifest_path']).parent), package=package_name))
                    executables.append(path)
        env.pop('RUSTDOCFLAGS', None)

        # ldd reports transitive ELF dependencies. Keep the loader, libc and driver
        # from the destination OS, but carry all non-system dependencies with us.
        libraries = {}
        for executable in executables:
            linked = subprocess.check_output(['ldd', str(executable)], env=env, text=True)
            if 'not found' in linked:
                raise RuntimeError(f'Unresolved library for {executable}:\n{linked}')
            for soname, source in re.findall(r'^\s*(\S+) => (/\S+)', linked, re.M):
                if soname.startswith('libcuda.so'):
                    continue
                if source.startswith(('/lib/', '/lib64/', '/usr/lib/', '/usr/lib64/')):
                    continue
                resolved = Path(source).resolve()
                if soname in libraries and libraries[soname] != resolved:
                    raise RuntimeError(f'Conflicting library {soname}')
                libraries[soname] = resolved
        for soname, source in sorted(libraries.items()):
            shutil.copy2(source, stage / 'lib' / soname)
            manifest['libraries'].append(dict(soname=soname, source=str(source)))

        # Preserve versioned/nonignored sources and fixtures, including dirty
        # edits. Cargo caches and .git are absent; selected OUT_DIR fixtures follow.
        files = subprocess.check_output(['git', '-C', str(root), 'ls-files', '-z',
                                         '--cached', '--others', '--exclude-standard']).split(b'\0')
        for raw in sorted(set(files)):
            if not raw:
                continue
            relative = Path(os.fsdecode(raw))
            if 'target' in relative.parts or '.git' in relative.parts:
                continue
            source = root / relative
            if not source.is_file():
                continue
            destination = stage / 'source' / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination)
        # Build scripts can generate runtime fixtures with absolute OUT_DIR
        # paths baked into tests (TPC-H's string-guard-fixture is one). Cargo's
        # build-script-executed records identify exact directories, not a glob
        # over stale build outputs. Preserve their data/source, never compiler
        # objects, libraries, or Cargo dependency caches.
        manifest['runtime_output_dirs'] = []
        for directory in sorted(runtime_output_dirs):
            if not directory.is_relative_to(root):
                raise RuntimeError(f'Runtime OUT_DIR escapes worktree: {directory}')
            manifest['runtime_output_dirs'].append(str(directory))
            for source in sorted(directory.rglob('*')):
                if not source.is_file() or source.suffix in ('.o', '.a', '.rlib', '.rmeta', '.so'):
                    continue
                destination = stage / 'source' / source.relative_to(root)
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(source, destination)
        tools = Path(__file__).resolve().parent
        for source in tools.iterdir():
            if source.suffix in ('.py', '.sh'):
                shutil.copy2(source, stage / 'tools' / source.name)
        # Exactness oracles run the prebuilt DuckDB CLI (cq-testkit duckdb.rs).
        duckdb = os.environ.get('DUCKDB_CLI') or shutil.which('duckdb')
        if not duckdb:
            raise RuntimeError('DuckDB CLI not found: set DUCKDB_CLI or put duckdb on PATH')
        shutil.copy2(Path(duckdb).resolve(), stage / 'bin' / 'duckdb')
        # Prepare the downloader locally as well; the pod never pip-installs.
        # typer-slim does not pull click, which the hf CLI imports.
        subprocess.run(['python3', '-m', 'pip', 'install', '--disable-pip-version-check',
                        '--only-binary=:all:', '--target', str(stage / 'hf'),
                        'huggingface_hub==1.1.5', 'click'], check=True)
        manifest['steps'] = [] if args.no_tests else [
            dict(name='crates', commands=manifest['tests']),
            dict(name='tpch-h05-sf10-ceiling', argv=['bin/cq-gpu-tpch', '{results}/gpu-gate-h05-sf10.json',
                 '--query', 'h05', '--scale', 'sf10', '--mode', 'advised']),
            dict(name='doop-cassandra-exact', argv=['bin/doop-gpu', '/data/doop-bench/doop-data/cassandra'])]
        manifest['build_seconds'] = time.monotonic() - start
        manifest['files'] = {}
        for path in sorted(stage.rglob('*')):
            if path.is_file():
                with path.open('rb') as stream:
                    digest = hashlib.sha256()
                    for chunk in iter(lambda: stream.read(1024 * 1024), b''):
                        digest.update(chunk)
                manifest['files'][str(path.relative_to(stage))] = digest.hexdigest()
        (stage / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
        subprocess.run(['tar', '--zstd', '-cf', str(output), '-C', str(stage), '.'], check=True,
                       env={**os.environ, 'ZSTD_CLEVEL': '3'})
        print(f'Bundle: {output} ({output.stat().st_size} bytes); total {time.monotonic() - start:.3f}s')
        print('Libraries:', ', '.join(libraries) or '(none)')


if __name__ == '__main__':
    main()
