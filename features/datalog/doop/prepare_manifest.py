#!/usr/bin/env python3
"""Certify DOOP file keys without rewriting input data.

Run after external DOOP/SRDatalog preparation. --manifest can place the result
outside a read-only dataset; loaders select it with DOOP_SET_MANIFEST.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path


# Raw DOOP decoder headings (s = interned string, i = signed i32).
RAW_TYPES = {
    "DirectSuperclass": "ss", "DirectSuperinterface": "ss", "MainClass": "s",
    "FormalParam": "iss", "ComponentType": "ss", "AssignReturnValue": "ss",
    "ActualParam": "iss", "Method-Modifier": "ss", "Var-Type": "ss",
    "ClassType": "s", "ArrayType": "s", "InterfaceType": "s",
    "Var-DeclaringMethod": "ss", "ApplicationClass": "s", "ThisVar": "ss",
    "NormalHeap": "ss", "StringConstant": "s", "AssignHeapAllocation": "sisssi",
    "AssignLocal": "sisss", "AssignCast": "sissss", "Field": "ssss",
    "StaticMethodInvocation": "siss", "SpecialMethodInvocation": "sisss",
    "VirtualMethodInvocation": "sisss", "Method": "ssssssi",
    "StoreInstanceField": "sissss", "LoadInstanceField": "sissss",
    "StoreStaticField": "sisss", "LoadStaticField": "sisss",
    "StoreArrayIndex": "sisss", "LoadArrayIndex": "sisss", "Return": "siss",
}


def schema(format: str) -> dict[str, list[str]]:
    if format == "raw":
        return {name + ".facts": ["string" if t == "s" else "i32" for t in types]
                for name, types in RAW_TYPES.items()}
    path = Path(__file__).parent / "srd/src/generated.schema.json"
    inputs = json.loads(path.read_text())["inputs"]
    return {relation["file"]: ["i32"] * relation["arity"] for relation in inputs.values()}


def decode(line: bytes, types: list[str], skip_empty: bool) -> tuple | None:
    text = line.decode("utf-8").removesuffix("\n").removesuffix("\r")
    if skip_empty and not text:
        return None
    columns = text.split("\t")
    if len(columns) != len(types):
        raise ValueError(f"expected {len(types)} columns, got {len(columns)}")
    row = []
    for value, kind in zip(columns, types):
        if kind == "i32":
            if not re.fullmatch(r"[+-]?[0-9]+", value):
                raise ValueError(f"invalid i32 {value!r}")
            value = int(value)
            if not -(2**31) <= value < 2**31:
                raise ValueError(f"i32 out of range: {value}")
        row.append(value)
    return tuple(row)


def certify(path: Path, types: list[str], keys: list[list[int]], skip_empty: bool) -> dict:
    digest = hashlib.sha256()
    seen = [set() for _ in keys]
    unique = [True] * len(keys)
    for key in keys:
        if not key or len(set(key)) != len(key) or any(i < 0 or i >= len(types) for i in key):
            raise ValueError(f"{path}: invalid key {key}")
    with path.open("rb") as source:
        for number, line in enumerate(source, 1):
            digest.update(line)
            try:
                row = decode(line, types, skip_empty)
            except ValueError as error:
                raise ValueError(f"{path}:{number}: {error}") from error
            if row is None:
                continue
            for i, key in enumerate(keys):
                if unique[i]:
                    projected = tuple(row[column] for column in key)
                    if projected in seen[i]:
                        unique[i] = False
                        seen[i].clear()
                    else:
                        seen[i].add(projected)
    # Premise of LD-DoopVerifiedKey: certify only duplicate-free decoded keys,
    # binding them to the same bytes examined above, not a later file read.
    return {"types": types, "verified_keys": [
        {"columns": key, "set": "verified", "sha256": digest.hexdigest()}
        for key, valid in zip(keys, unique) if valid
    ]}


def prepare(directory: Path, format: str, extra_keys: dict[str, list[list[int]]] | None = None) -> dict:
    headings = schema(format)
    extra_keys = extra_keys or {}
    unknown = extra_keys.keys() - headings.keys()
    if unknown:
        raise ValueError(f"unknown input files: {sorted(unknown)}")
    relations = {}
    for filename, types in headings.items():
        path = directory / filename
        if format == "raw" and not path.exists():
            continue  # Raw DOOP's omitted relations are empty.
        keys = [list(range(len(types))), *extra_keys.get(filename, [])]
        relations[filename] = certify(path, types, keys, format == "prepared")
    return {"version": 1, "relations": relations}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--format", choices=("raw", "prepared"), required=True)
    parser.add_argument("--manifest", type=Path, help="output path; default DIRECTORY/doop-set-manifest.json")
    parser.add_argument("--key", action="append", default=[], metavar="FILE:0,1",
                        help="also verify a zero-based column key (repeatable)")
    args = parser.parse_args()
    keys = {}
    for specification in args.key:
        filename, columns = specification.rsplit(":", 1)
        keys.setdefault(filename, []).append([int(column) for column in columns.split(",")])
    output = args.manifest or args.directory / "doop-set-manifest.json"
    if any(output.resolve() == (args.directory / name).resolve()
           for name in [*schema(args.format), "meta.json"]):
        parser.error("manifest output must not overwrite an input file")
    manifest = prepare(args.directory, args.format, keys)
    output.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
