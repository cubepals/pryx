#!/usr/bin/env python3
"""Print native paired-writer medians and nearest-rank p95, including the best static baseline."""
import csv
import math
import statistics
import sys
from collections import defaultdict

by_method = defaultdict(dict)
with open(sys.argv[1], newline="") as source:
    for row in csv.DictReader(source):
        label = row["implementation"], row["plan"]
        pair = int(row["pair"])
        if pair in by_method[label]:
            raise ValueError(f"duplicate pair {pair} for {label}")
        by_method[label][pair] = int(row["elapsed_ns"])
if not by_method or any(set(rows) != set(range(15)) for rows in by_method.values()):
    raise ValueError("expected exactly 15 complete native pairs per method")
stats = {name: (statistics.median(rows.values()), sorted(rows.values())[math.ceil(.95*len(rows))-1])
         for name,rows in by_method.items()}
generated, = [name for name in stats if name[0] == "generated"]
static = min((name for name in stats if name != generated), key=lambda name:stats[name][0])
print("| Implementation | Plan | Median ms | p95 ms |")
print("| --- | --- | ---: | ---: |")
for (method,plan),(median,p95) in stats.items():
    print(f"| {method} | {plan} | {median/1e6:.3f} | {p95/1e6:.3f} |")
print(f"\nStrongest static comparator: {static}.")
print(f"Median ratio {stats[generated][0]/stats[static][0]:.4f}, p95 ratio {stats[generated][1]/stats[static][1]:.4f}.")
