"""Measure the pinned original ddisasm module without modifying external sources."""
import ctypes
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import time

from srdatalog import CompilerConfig, build_project, compile_jit_project
from srdatalog.runtime import cuda_compile_flags, cuda_include_paths, cuda_libs, cuda_link_flags, runtime_defines, runtime_include_paths

ROOT = Path(__file__).resolve().parent
SOURCE = Path('/data/doop-next-round/source/examples/ddisasm.py')
UPSTREAM = Path('/home/stargazermiao/workspace/SRDatalog')
name = sys.argv[1]
meta_path = UPSTREAM / f'integration_tests/examples/ddisasm/{name}_consts.json'
data_path = UPSTREAM / f'benchmarks/data/ddisasm/{name}'
meta = json.loads(meta_path.read_text())
spec = importlib.util.spec_from_file_location('original_ddisasm', SOURCE)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
program = module.build_ddisasmdb_program(meta)
result = {'dataset': name, 'source': str(SOURCE), 'source_sha256': hashlib.sha256(SOURCE.read_bytes()).hexdigest(), 'metadata_path': str(meta_path), 'constants': meta, 'data_path': str(data_path), 'rule_count': len(program.rules), 'relation_count': len(program.relations), 'input_mapping': {r.name: r.input_file for r in program.relations if r.input_file}, 'timers_seconds': {}}
timers = result['timers_seconds']
t0 = time.perf_counter()
project = build_project(program, project_name='Ddisasm' + name.capitalize(), cache_base=str(ROOT / 'srdatalog-build'))
timers['emit'] = time.perf_counter() - t0
cfg = CompilerConfig(include_paths=runtime_include_paths() + cuda_include_paths(), defines=runtime_defines(), cxx_flags=cuda_compile_flags() + ['-fPIC'], link_flags=cuda_link_flags(), libs=cuda_libs() + ['boost_container'], shared=True, jobs=4)
t0 = time.perf_counter()
build = compile_jit_project(project, cfg)
timers['compile'] = time.perf_counter() - t0
if not build.ok():
    for entry in build.compile_results:
        if entry.returncode:
            print(entry.stderr or entry.stdout, flush=True)
    raise RuntimeError('Compilation failed')
result['artifact'] = str(build.artifact)
lib = ctypes.CDLL(str(build.artifact), mode=ctypes.RTLD_GLOBAL)
for fn in ['init', 'load_all', 'prepare', 'run', 'synchronize', 'shutdown']:
    getattr(lib, 'srdatalog_' + fn).restype = ctypes.c_int
lib.srdatalog_load_all.argtypes = [ctypes.c_char_p]
lib.srdatalog_run.argtypes = [ctypes.c_ulonglong]
lib.srdatalog_size.argtypes = [ctypes.c_char_p]
lib.srdatalog_size.restype = ctypes.c_ulonglong

def checked(fn, *args):
    rc = getattr(lib, 'srdatalog_' + fn)(*args)
    if rc:
        raise RuntimeError(f'{fn} returned {rc}')

for phase, args in [('init', ()), ('load_all', (str(data_path).encode(),)), ('prepare', ())]:
    t0 = time.perf_counter()
    checked(phase, *args)
    timers[phase] = time.perf_counter() - t0
    print(phase, timers[phase], flush=True)
t0 = time.perf_counter()
checked('run', 0)
timers['run_call'] = time.perf_counter() - t0
checked('synchronize')
timers['run_synchronized'] = time.perf_counter() - t0
result['counts'] = {r.name: int(lib.srdatalog_size(r.name.encode())) for r in program.relations}
result['output_counts'] = {r.name: result['counts'][r.name] for r in program.relations if r.print_size}
checked('shutdown')
(ROOT / f'srdatalog-{name}.json').write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps(result, indent=2), flush=True)
