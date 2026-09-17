#!/usr/bin/env python3
"""Capture actual proc-macro boundaries in an isolated source copy.

Only the copied frontend is instrumented. The wrapper records textual input and
output while returning the original TokenStream, preserving compilation hygiene.
This observes real rustc invocations; it does not reconstruct a typed pass chain.
"""
import argparse
from pathlib import Path
import difflib
import hashlib
import json
import os
import re
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--example', required=True, help='mini-linq example target')
parser.add_argument('--features', required=True, help='explicit comma-separated frontend features')
parser.add_argument('--out', type=Path, required=True, help='new artifact directory; must not exist')
parser.add_argument('--declarative', action='store_true',
                    help='also record rustc macro_rules tracing in a separate compilation')
parser.add_argument('--stdout', action='store_true', help='print the complete step-by-step report')
args = parser.parse_args()
OUT = args.out.resolve()
OUT.mkdir(parents=True, exist_ok=False)
COPY = OUT / 'workspace'
COPY.mkdir()
ignore = shutil.ignore_patterns('target', '.git', '__pycache__')
for name in ['crates', 'workloads', 'examples', '.cargo']:
    shutil.copytree(ROOT / name, COPY / name, ignore=ignore, dirs_exist_ok=True)
for name in ['Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml']:
    shutil.copy2(ROOT / name, COPY / name)
sources = {str(p.relative_to(COPY)): hashlib.sha256(p.read_bytes()).hexdigest()
           for p in COPY.rglob('*') if p.is_file()}
(OUT / 'source-hashes.json').write_text(json.dumps(sources, indent=2) + '\n')

frontend = COPY / 'crates/mini-linq/src'
original_lib = (frontend / 'lib.rs').read_text()
current = None
lines = []
for line in original_lib.splitlines(keepends=True):
    match = re.search(r'pub fn (\w+|\$name)\(input: TokenStream\)', line)
    if match:
        current = match[1]
    if 'entry::stage(' in line:
        assert current is not None
        name = 'stringify!($name)' if current == '$name' else json.dumps(current)
        line = line.replace('entry::stage(', f'entry::stage({name}, ')
    if 'workload::expand(input)' in line:
        line = line.replace('workload::expand(input)',
                            'trace::expand("workload_query", input, workload::expand)')
    lines.append(line)
new_lib = ''.join(lines).replace('mod workload;', 'mod workload;\nmod trace;')
(frontend / 'lib.rs').write_text(new_lib)
original_entry = (frontend / 'entry.rs').read_text()
new_entry = original_entry.replace('pub(crate) fn stage<', 'fn untraced_stage<', 1)
new_entry += '''
pub(crate) fn stage<S: Parse, T: ToTokens>(
    name: &str,
    input: TokenStream,
    next: Option<syn::Path>,
    lower: impl FnOnce(&S) -> syn::Result<T>,
) -> TokenStream {
    crate::trace::expand(name, input, |input| untraced_stage(input, next, lower))
}
'''
(frontend / 'entry.rs').write_text(new_entry)
(frontend / 'trace.rs').write_text('''
use proc_macro::TokenStream;
use std::{fs, path::PathBuf, sync::atomic::{AtomicUsize, Ordering}};
static SEQUENCE: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn expand(
    name: &str,
    input: TokenStream,
    compile: impl FnOnce(TokenStream) -> TokenStream,
) -> TokenStream {
    let Some(directory) = std::env::var_os("MINI_LINQ_TRACE_DIR") else {
        return compile(input);
    };
    let directory = PathBuf::from(directory).join(std::process::id().to_string());
    fs::create_dir_all(&directory).unwrap();
    let number = SEQUENCE.fetch_add(1, Ordering::Relaxed) + 1;
    let base = format!("{number:03}-{name}");
    fs::write(directory.join(format!("{base}.input.tokens")), input.to_string()).unwrap();
    let output = compile(input);
    fs::write(directory.join(format!("{base}.output.tokens")), output.to_string()).unwrap();
    output
}
''')
patch = ''.join(difflib.unified_diff(original_lib.splitlines(True), new_lib.splitlines(True),
                                   fromfile='lib.rs', tofile='instrumented/lib.rs'))
patch += ''.join(difflib.unified_diff(original_entry.splitlines(True), new_entry.splitlines(True),
                                    fromfile='entry.rs', tofile='instrumented/entry.rs'))
(OUT / 'instrumentation.patch').write_text(patch)
env = os.environ.copy()
env['CARGO_TARGET_DIR'] = str(OUT / 'build')
env['MINI_LINQ_TRACE_DIR'] = str(OUT / 'raw')
command = ['cargo', 'run', '-p', 'mini-linq', '--no-default-features', '--features', args.features, '--example', args.example, '--locked']
commands = {'cwd': str(COPY), 'command': command,
    'env': {k: env[k] for k in ['CARGO_TARGET_DIR', 'MINI_LINQ_TRACE_DIR']}}
(OUT / 'command.json').write_text(json.dumps(commands, indent=2)+'\n')
with (OUT / 'compile.log').open('w') as log:
    subprocess.run(command, cwd=COPY, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
if args.declarative:
    # Keep unstable tracing local to this subprocess and this example target.
    # A second build does not duplicate the procedural recorder's events.
    trace_env = env.copy()
    trace_env.pop('MINI_LINQ_TRACE_DIR')
    trace_env['RUSTC_BOOTSTRAP'] = '1'
    trace_command = ['cargo', 'rustc', *command[2:], '--', '-Ztrace-macros']
    commands['declarative'] = {'command': trace_command,
        'env': {'CARGO_TARGET_DIR': trace_env['CARGO_TARGET_DIR'], 'RUSTC_BOOTSTRAP': '1'},
        'unset_env': ['MINI_LINQ_TRACE_DIR']}
    (OUT / 'command.json').write_text(json.dumps(commands, indent=2)+'\n')
    with (OUT / 'declarative.log').open('w') as log:
        subprocess.run(trace_command, cwd=COPY, env=trace_env,
                       stdout=log, stderr=subprocess.STDOUT, check=True)
assert all(hashlib.sha256((ROOT / p).read_bytes()).hexdigest() == digest
           for p, digest in sources.items()), 'Original source changed while capturing'
pairs = sorted((OUT / 'raw').glob('*/*.input.tokens'),
               key=lambda path: (int(path.parent.name), int(path.name.split('-', 1)[0])))
index = ['# Actual proc-macro boundaries', '',
         'These calls were captured during rustc expansion of the unchanged example.',
         'Raw token files are authoritative. Formatted Rust is a reading aid only.',
         'Sequence numbers are invocation order within each compiler process, not runtime order.',
         'See command.json, compile.log, source-hashes.json and instrumentation.patch.', '',
         '[Print all procedural steps](steps.md)', '']
if args.declarative:
    index.extend(['[Declarative macro input/output trace](declarative.log) was captured in a',
                  'second compilation of the same source with target-local `-Ztrace-macros`.',
                  'The two logs are separate observations, not one merged event timeline.', ''])
index.extend(['| Process / step | Macro | Input begins | Output bytes |',
              '| --- | --- | --- | ---: |'])
pages = []
calls = []
for source in pairs:
    target = source.with_name(source.name.replace('.input.tokens', '.output.tokens'))
    assert target.exists(), f'Missing output for {source}'
    stem = source.name.removesuffix('.input.tokens')
    relative = source.parent.relative_to(OUT)
    page = source.parent / f'{stem}.md'
    pretty = subprocess.run(['rustfmt', '--edition', '2024', '--emit', 'stdout'],
                            input=target.read_text(), text=True, capture_output=True)
    rendered = pretty.stdout if pretty.returncode == 0 else target.read_text()
    content = (f'## {source.parent.name}/{stem}\n\n'
               f'[Raw input]({source.name}) · [Raw output]({target.name})\n\n'
               f'Actual input:\n\n```rust\n{source.read_text()}\n```\n\n'
               f'Actual output (formatted where Rust syntax permits):\n\n```rust\n{rendered}\n```\n')
    page.write_text(content)
    pages.append(content.replace(f']({source.name})', f']({relative}/{source.name})')
                        .replace(f']({target.name})', f']({relative}/{target.name})'))
    step, macro = stem.split('-', 1)
    beginning = source.read_text().split(maxsplit=1)[0] if source.read_text().strip() else '(empty)'
    output_bytes = len(target.read_bytes())
    index.append(f'| [{source.parent.name}/{step}]({relative}/{stem}.md) | `{macro}` | '
                 f'`{beginning.replace("|", "&#124;")}` | {output_bytes} |')
    calls.append({'process': int(source.parent.name), 'step': int(step), 'macro': macro,
                  'input': str(source.relative_to(OUT)), 'output': str(target.relative_to(OUT)),
                  'output_bytes': output_bytes})
(OUT / 'README.md').write_text('\n'.join(index) + '\n')
report = '\n'.join(index[:index.index('| Process / step | Macro | Input begins | Output bytes |')])
report += '\n\n' + '\n---\n\n'.join(pages)
if args.declarative:
    report += '\n---\n\n## Declarative macros: separate compiler trace\n\n```text\n'
    report += (OUT / 'declarative.log').read_text() + '\n```\n'
(OUT / 'steps.md').write_text(report)
(OUT / 'calls.json').write_text(json.dumps(calls, indent=2) + '\n')
(OUT / 'verification.txt').write_text(
    f'{len(pairs)} actual invocation pairs; {len(sources)} original source hashes unchanged.\n'
    'Recording returns the original TokenStream without reparsing.\n')
if args.stdout:
    print(report)
else:
    print(f'Captured {len(pairs)} actual proc-macro calls in {OUT}; original source hashes unchanged.')
