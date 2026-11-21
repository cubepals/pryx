#!/usr/bin/env python3
"""Report complete-file time and per-pair ratios to the strongest resident static writer."""
import csv
import math
import statistics
import sys
from collections import defaultdict


def p95(values):
    values = sorted(values)
    return values[math.ceil(0.95 * len(values)) - 1]


def summary(source):
    methods = defaultdict(dict)
    generated = {}
    orders = set()
    cpus = set()
    for row in csv.DictReader(source):
        name = row["implementation"]
        pair, order = int(row["pair"]), int(row["order"])
        if pair in methods[name] or (pair, order) in orders:
            raise ValueError(f"duplicate pair or run order for {name}")
        if row["generated"] not in ("true", "false") or int(row["logical_bytes"]) != 256 << 20:
            raise ValueError("unexpected method classification or logical image size")
        if generated.setdefault(name, row["generated"] == "true") != (row["generated"] == "true"):
            raise ValueError("method classification changed")
        elapsed, allocated = int(row["elapsed_ns"]), int(row["allocated_bytes"])
        if elapsed <= 0 or allocated < 0:
            raise ValueError("invalid elapsed time or allocated blocks")
        cpus.add(int(row["cpu"]))
        methods[name][pair] = elapsed, allocated, order
        orders.add((pair, order))
    if not methods or len(cpus) != 1 or any(set(rows) != set(range(15)) for rows in methods.values()):
        raise ValueError("expected 15 complete rounds on one CPU per method")
    if {name for name, is_generated in generated.items() if is_generated} != {"generated-zero", "generated-stage-copy"}:
        raise ValueError("missing generated writer")
    for name, rows in methods.items():
        for pair in range(1, 15):
            if rows[pair][2] != len(methods) - 1 - rows[pair - 1][2]:
                raise ValueError(f"order did not reverse for {name} in round {pair}")
    times = {name: [rows[p][0] for p in range(15)] for name, rows in methods.items()}
    static = min((name for name in times if not generated[name]), key=lambda name: statistics.median(times[name]))
    print("| Complete writer | Median ms | p95 ms | Median allocated MiB |")
    print("| --- | ---: | ---: | ---: |")
    for name, values in sorted(times.items(), key=lambda item: statistics.median(item[1])):
        blocks = [entry[1] for entry in methods[name].values()]
        print(f"| {name} | {statistics.median(values)/1e6:.3f} | {p95(values)/1e6:.3f} | {statistics.median(blocks)/2**20:.3f} |")
    print(f"\nFastest static writer by median complete-file time was {static}.")
    for name in ("generated-zero", "generated-stage-copy"):
        ratios = [a / b for a, b in zip(times[name], times[static])]
        print(f"\n{name} / {static} paired ratio had median {statistics.median(ratios):.4f} and p95 {p95(ratios):.4f}. "
              f"Whole-cohort p95 ratio was {p95(times[name]) / p95(times[static]):.4f}.")


if __name__ == "__main__":
    with open(sys.argv[1], newline="") as source:
        summary(source)
