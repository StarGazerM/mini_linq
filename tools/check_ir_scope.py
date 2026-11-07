#!/usr/bin/env python3
"""Fail when a branch adds grammar to an IR it does not own.

Grammar is the data shape of a program: structs, fields, enums, variants,
unions and type aliases. Functions, trait impls and parameter changes are
API and are not compared (P7 restricts grammar, not API).

A branch declares the IR crate(s) whose decision it changes (`--owner`).
In every other `languages/cq-lang-*` crate it may delete or rename public
items but not add them: an order-only change adds nothing below VO because
no later semantics changes (design-notes/ir-redesign.md, P7).

The comparison uses the reviewed public-API snapshots in tools/public-api
(one sorted line per exported path and normalized signature) at the merge
base versus the working tree. Run `python3 tools/check_public_api.py
--update` first so the working snapshots reflect the code.
"""
import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(subprocess.run(["git", "rev-parse", "--show-toplevel"], capture_output=True,
                           text=True, check=True).stdout.strip())
SNAPSHOTS = ROOT / "tools" / "public-api"
IR_CRATES = ("cq-lang-cq", "cq-lang-vo", "cq-lang-ra", "cq-lang-ix",
             "cq-lang-physical", "cq-lang-structures", "cq-lang-sequence",
             "cq-syntax")
GRAMMAR_KINDS = ('{"struct"', '{"struct_field"', '{"enum"', '{"variant"',
                 '{"union"', '{"type_alias"')


def lines_at(revision, crate):
    path = f"tools/public-api/{crate}.txt"
    if revision is None:
        file = SNAPSHOTS / f"{crate}.txt"
        return set(file.read_text().splitlines()) if file.exists() else set()
    result = subprocess.run(["git", "show", f"{revision}:{path}"], cwd=ROOT,
                            capture_output=True, text=True)
    return set(result.stdout.splitlines()) if result.returncode == 0 else set()


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--owner", action="append", default=[],
                        help="IR crate the branch owns (repeatable)")
    parser.add_argument("--base", default=None,
                        help="merge base revision (default: merge-base with codex/all-answers)")
    parser.add_argument("--allow", action="append", default=[],
                        help="path substring whose addition or signature change is a declared rename consequence (repeatable)")
    args = parser.parse_args()
    unknown = [o for o in args.owner if o not in IR_CRATES]
    if unknown:
        raise SystemExit(f"unknown owner crates: {unknown}; IR crates: {list(IR_CRATES)}")
    base = args.base or subprocess.run(
        ["git", "merge-base", "HEAD", "codex/all-answers"], cwd=ROOT,
        capture_output=True, text=True, check=True).stdout.strip()
    added, changed = {}, {}
    for crate in IR_CRATES:
        if crate in args.owner:
            continue
        # Grammar = the data shape of a program: types, fields, variants. Functions,
        # trait impls and parameters are API, not grammar (P7 restricts grammar).
        grammar = lambda sig: sig.lstrip().startswith(GRAMMAR_KINDS)
        before = dict(line.split("\t", 1) for line in lines_at(base, crate) if "\t" in line)
        after = dict(line.split("\t", 1) for line in lines_at(None, crate) if "\t" in line)
        before = {p: s for p, s in before.items() if grammar(s)}
        after = {p: s for p, s in after.items() if grammar(s)}
        allowed = lambda path: any(a in path for a in args.allow)
        new = sorted(p for p in after.keys() - before.keys() if not allowed(p))
        # A signature that only lost tokens (a deleted field or variant) is a deletion.
        tokens = lambda sig: set(re.findall(r"[A-Za-z_][A-Za-z0-9_]*", sig))
        differing = sorted(p for p in after.keys() & before.keys()
                           if after[p] != before[p] and not allowed(p)
                           and not tokens(after[p]) <= tokens(before[p]))
        if new:
            added[crate] = new
        if differing:
            changed[crate] = differing
    for label, table in (("added", added), ("signature changed", changed)):
        for crate, items in table.items():
            print(f"{crate}: {len(items)} public item(s) {label} outside the declared owner")
            for item in items[:20]:
                print(f"  + {item}")
            if len(items) > 20:
                print(f"  ... {len(items) - 20} more")
    if added or changed:
        raise SystemExit("IR scope: grammar added or changed outside the owning IR (P7); declare renames with --allow")
    print(f"PASS: no IR grammar added outside {args.owner or 'any owner'} since {base[:12]}")


if __name__ == "__main__":
    main()
