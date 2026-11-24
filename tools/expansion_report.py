#!/usr/bin/env python3
"""Aggregate SQ-ExpansionCounters stderr records into per-rule JSON.

Usage: python3 tools/expansion_report.py < counters.log
Multiple records for a stage (iterations/invocations) are summed. Admission is
repeated on each definition record: use the last observed definition's total,
not a sum of those copies. Rules may overlap: admitted totals across rules are
not the size of their union. A zero denominator is emitted as null, not infinity.
"""
import json
import sys
from collections import defaultdict


def summarize(lines):
    groups = {}
    for line in lines:
        kind, *fields = line.rstrip("\n").split("\t")
        if kind not in ("cq_expansion", "cq_expansion_output"):
            continue
        values = dict(field.split("=", 1) for field in fields)
        key = (values["rule"], int(values["variant"]), values["program"])
        group = groups.setdefault(key, {"children": defaultdict(int), "output": defaultdict(int), "admitted": defaultdict(int)})
        definition = values["definition"]
        if kind == "cq_expansion":
            group["children"][(definition, values["stage"])] += int(values["children"])
        else:
            group["output"][definition] += int(values["output"])
            group["admitted"][definition] += int(values["admitted"])
            group["last_definition"] = definition
    result = []
    for (rule, variant, program), group in sorted(groups.items()):
        largest = max(group["children"].values(), default=0)
        admitted = group["admitted"].get(group.get("last_definition"), 0)
        result.append({
            "rule": rule, "variant": variant, "program": program,
            "stages": [{"definition": definition, "stage": stage, "children": count}
                       for (definition, stage), count in sorted(group["children"].items())],
            "definition_output": dict(sorted(group["output"].items())),
            "admitted": admitted, "largest_preclosing": largest,
            "largest_preclosing_per_output": largest / admitted if admitted else None,
        })
    return result


if __name__ == "__main__":
    json.dump(summarize(sys.stdin), sys.stdout, indent=2)
    print()
