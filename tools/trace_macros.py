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
(OUT / 'command.json').write_text(json.dumps({'cwd': str(COPY), 'command': command,
    'env': {k: env[k] for k in ['CARGO_TARGET_DIR', 'MINI_LINQ_TRACE_DIR']}}, indent=2)+'\n')
with (OUT / 'compile.log').open('w') as log:
    subprocess.run(command, cwd=COPY, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
assert all(hashlib.sha256((ROOT / p).read_bytes()).hexdigest() == digest
           for p, digest in sources.items()), 'Original source changed while capturing'
pairs = sorted((OUT / 'raw').glob('*/*.input.tokens'))
index = ['# Actual proc-macro boundaries', '',
         'These calls were captured during rustc expansion of the unchanged example.',
         'Raw token files are authoritative. Formatted Rust is a reading aid only.',
         'See command.json, compile.log, source-hashes.json and instrumentation.patch.', '']
for source in pairs:
    target = source.with_name(source.name.replace('.input.tokens', '.output.tokens'))
    assert target.exists(), f'Missing output for {source}'
    stem = source.name.removesuffix('.input.tokens')
    relative = source.parent.relative_to(OUT)
    page = source.parent / f'{stem}.md'
    pretty = subprocess.run(['rustfmt', '--edition', '2024', '--emit', 'stdout'],
                            input=target.read_text(), text=True, capture_output=True)
    rendered = pretty.stdout if pretty.returncode == 0 else target.read_text()
    page.write_text(f'# {stem}\n\nActual input:\n\n```rust\n{source.read_text()}\n```\n\n'
                    f'Actual output (formatted where Rust syntax permits):\n\n```rust\n{rendered}\n```\n')
    index.append(f'- [{source.parent.name}/{stem}]({relative}/{stem}.md)')
(OUT / 'README.md').write_text('\n'.join(index) + '\n')
(OUT / 'verification.txt').write_text(
    f'{len(pairs)} actual invocation pairs; {len(sources)} original source hashes unchanged.\n'
    'Recording returns the original TokenStream without reparsing.\n')
print(f'Captured {len(pairs)} actual proc-macro calls in {OUT}; original source hashes unchanged.')
