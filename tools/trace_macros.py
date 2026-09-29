#!/usr/bin/env python3
"""Capture actual proc-macro boundaries in an isolated source copy.

Only copied proc-macro implementations are instrumented. The wrapper records
textual input and output while returning the original TokenStream, preserving
compilation hygiene. This observes real rustc invocations; it does not reconstruct
a typed pass chain. Use --macro-package for external provider implementations.
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
target = parser.add_mutually_exclusive_group(required=True)
target.add_argument('--example', help='Cargo example target')
target.add_argument('--bin', help='Cargo binary target')
target.add_argument('--test', help='Cargo integration test target')
parser.add_argument('--package', default='mini-linq', help='Cargo package (default: mini-linq)')
parser.add_argument('--macro-package', action='append', default=[], metavar='PACKAGE',
                    help='also instrument this workspace proc-macro package; repeatable')
parser.add_argument('--manifest-path', type=Path, default=Path('Cargo.toml'),
                    help='repository-relative manifest included in the isolated copy')
parser.add_argument('--features', default='', help='explicit comma-separated package features')
parser.add_argument('--out', type=Path, required=True, help='new artifact directory; must not exist')
parser.add_argument('--declarative', action='store_true',
                    help='also record rustc macro_rules tracing in a separate compilation')
parser.add_argument('--stdout', action='store_true', help='print the complete step-by-step report')
parser.add_argument('program_args', nargs=argparse.REMAINDER,
                    help='arguments after -- are passed to the example, binary or test harness')
args = parser.parse_args()
copied_directories = ['crates', 'workloads', 'examples', '.cargo']
manifest = args.manifest_path
if (manifest.is_absolute() or '..' in manifest.parts
        or manifest.name != 'Cargo.toml'
        or (manifest != Path('Cargo.toml') and manifest.parts[0] not in copied_directories)
        or not (ROOT / manifest).is_file()
        or not (ROOT / manifest).resolve().is_relative_to(ROOT)):
    parser.error('--manifest-path must name an existing repository-relative Cargo.toml in the copied workspace')
program_args = args.program_args
if program_args[:1] == ['--']:
    program_args = program_args[1:]
OUT = args.out.resolve()
OUT.mkdir(parents=True, exist_ok=False)
COPY = OUT / 'workspace'
COPY.mkdir()
ignore = shutil.ignore_patterns('target', '.git', '__pycache__')
for name in copied_directories:
    shutil.copytree(ROOT / name, COPY / name, ignore=ignore, dirs_exist_ok=True)
for name in ['Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml']:
    shutil.copy2(ROOT / name, COPY / name)
sources = {str(p.relative_to(COPY)): hashlib.sha256(p.read_bytes()).hexdigest()
           for p in COPY.rglob('*') if p.is_file()}
(OUT / 'source-hashes.json').write_text(json.dumps(sources, indent=2) + '\n')

def body_end(source, opening):
    """Find a Rust body delimiter without interpreting its strings or comments."""
    depth = 0
    position = opening
    while position < len(source):
        if source.startswith('//', position):
            newline = source.find('\n', position + 2)
            position = len(source) if newline < 0 else newline + 1
            continue
        if source.startswith('/*', position):
            comments = 1
            position += 2
            while comments and position < len(source):
                if source.startswith('/*', position):
                    comments += 1
                    position += 2
                elif source.startswith('*/', position):
                    comments -= 1
                    position += 2
                else:
                    position += 1
            if comments:
                raise ValueError('unterminated Rust block comment')
            continue
        raw = re.match(r'(?:br|cr|r)(\#*)"', source[position:])
        if raw:
            terminator = '"' + raw[1]
            closing = source.find(terminator, position + raw.end())
            if closing < 0:
                raise ValueError('unterminated Rust raw string')
            position = closing + len(terminator)
            continue
        if source[position] == '"':
            position += 1
            while position < len(source) and source[position] != '"':
                position += 2 if source[position] == '\\' else 1
            if position == len(source):
                raise ValueError('unterminated Rust string')
            position += 1
            continue
        if source[position] == "'":
            # A lifetime is not a character literal. Rust's character escapes
            # include braces, so skip complete literals before counting braces.
            character = re.match(r"'(?:[^'\\\n]|\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F_]+\}|.))'",
                                 source[position:])
            if character:
                position += character.end()
                continue
        if source[position] == '{':
            depth += 1
        elif source[position] == '}':
            depth -= 1
            if depth == 0:
                return position
        position += 1
    raise ValueError('unterminated Rust function body')


ENTRY = re.compile(
    r'#\[\s*proc_macro\s*\]\s*pub\s+fn\s+(\w+|\$\w+)\s*'
    r'\(\s*(\w+)\s*:\s*(?:proc_macro::)?TokenStream\s*,?\s*\)\s*'
    r'->\s*(?:proc_macro::)?TokenStream\s*\{')
RECORDER = '''
use proc_macro::TokenStream;
use std::{fs, io::ErrorKind, path::PathBuf, sync::atomic::{AtomicUsize, Ordering}};
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
    // Proc-macro crates have independent statics. Atomically creating a shared
    // marker orders invocation starts across every instrumented crate in rustc.
    let number = loop {
        let number = SEQUENCE.fetch_add(1, Ordering::Relaxed) + 1;
        match fs::OpenOptions::new().write(true).create_new(true)
            .open(directory.join(format!("{number:03}.invocation"))) {
            Ok(_) => break number,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => panic!("cannot reserve macro invocation: {error}"),
        }
    };
    let package = env!("CARGO_PKG_NAME");
    let label = if package == "mini-linq" { name.to_owned() }
                else { format!("{package}::{name}") };
    let base = format!("{number:03}-{label}");
    fs::write(directory.join(format!("{base}.input.tokens")), input.to_string()).unwrap();
    fs::write(directory.join(format!("{base}.package")), package).unwrap();
    let output = compile(input);
    fs::write(directory.join(format!("{base}.output.tokens")), output.to_string()).unwrap();
    output
}
'''


macro_packages = list(dict.fromkeys(['mini-linq', *args.macro_package]))
macro_sources = {'mini-linq': COPY / 'facade/mini-linq/src/lib.rs'}
if args.macro_package:
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--manifest-path', str(manifest), '--no-deps',
         '--format-version=1', '--locked'], cwd=COPY, text=True))
    for package in args.macro_package:
        matches = [target for member in metadata['packages'] if member['name'] == package
                   for target in member['targets'] if 'proc-macro' in target['kind']]
        if len(matches) != 1:
            parser.error(f'--macro-package {package}: expected one workspace proc-macro target')
        source = Path(matches[0]['src_path'])
        if not source.resolve().is_relative_to(COPY):
            parser.error(f'--macro-package {package}: implementation is outside the isolated copy')
        macro_sources[package] = source

patch = ''
instrumented = []
for package in macro_packages:
    source = macro_sources[package]
    original = source.read_text()
    entries = list(ENTRY.finditer(original))
    if not entries or len(entries) != len(re.findall(r'#\[\s*proc_macro\s*\]', original)):
        parser.error(f'{source.relative_to(COPY)}: unsupported proc-macro entry signature; '
                     'expected pub fn name(input: TokenStream) -> TokenStream')
    trace_path = source.parent / '__cq_trace.rs'
    if trace_path.exists() or '__cq_trace' in original:
        parser.error(f'{source.relative_to(COPY)}: reserved instrumentation module already exists')
    modified = original
    for entry in reversed(entries):
        name, argument = entry.groups()
        opening = entry.end() - 1
        closing = body_end(original, opening)
        label = f'stringify!({name})' if name.startswith('$') else json.dumps(name)
        modified = (modified[:closing] + '\n    })\n' + modified[closing:])
        modified = (modified[:opening + 1]
                    + f'\n    crate::__cq_trace::expand({label}, {argument}, |{argument}| {{'
                    + modified[opening + 1:])
    modified += '\nmod __cq_trace;\n'
    source.write_text(modified)
    trace_path.write_text(RECORDER)
    for path, before, after in [(source, original, modified), (trace_path, '', RECORDER)]:
        relative = path.relative_to(COPY)
        patch += ''.join(difflib.unified_diff(before.splitlines(True), after.splitlines(True),
                                            fromfile=str(relative),
                                            tofile=f'instrumented/{relative}'))
    instrumented.append({'package': package, 'source': str(source.relative_to(COPY)),
                         'entry_templates': [entry[1] for entry in entries]})
(OUT / 'instrumentation.patch').write_text(patch)
(OUT / 'instrumented-packages.json').write_text(json.dumps(instrumented, indent=2) + '\n')
env = os.environ.copy()
env['CARGO_TARGET_DIR'] = str(OUT / 'build')
env['MINI_LINQ_TRACE_DIR'] = str(OUT / 'raw')
target_args = ['--manifest-path', str(manifest), '-p', args.package, '--no-default-features']
if args.features:
    target_args += ['--features', args.features]
if args.example:
    target_args += ['--example', args.example]
elif args.bin:
    target_args += ['--bin', args.bin]
else:
    target_args += ['--test', args.test]
target_args.append('--locked')
command = ['cargo', 'test' if args.test else 'run', *target_args]
if program_args:
    command += ['--', *program_args]
commands = {'cwd': str(COPY), 'command': command,
    'env': {k: env[k] for k in ['CARGO_TARGET_DIR', 'MINI_LINQ_TRACE_DIR']}}
(OUT / 'command.json').write_text(json.dumps(commands, indent=2)+'\n')
with (OUT / 'compile.log').open('w') as log:
    subprocess.run(command, cwd=COPY, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
if args.declarative:
    # Keep unstable tracing local to this subprocess and this target.
    # A second build does not duplicate the procedural recorder's events.
    trace_env = env.copy()
    trace_env.pop('MINI_LINQ_TRACE_DIR')
    trace_env['RUSTC_BOOTSTRAP'] = '1'
    trace_command = ['cargo', 'rustc', *target_args, '--', '-Ztrace-macros']
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
         'These calls were captured during rustc expansion of the unchanged target.',
         'Raw token files are authoritative. Formatted Rust is a reading aid only.',
         'Sequence numbers are invocation order within each compiler process, not runtime order.',
         'Provider calls selected by --macro-package share that process sequence.',
         'See command.json, compile.log, source-hashes.json, instrumentation.patch',
         'and instrumented-packages.json.', '',
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
    package = source.with_name(f'{stem}.package').read_text()
    calls.append({'process': int(source.parent.name), 'step': int(step), 'macro': macro,
                  'package': package,
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
