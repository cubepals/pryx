# SPDX-License-Identifier: FSL-1.1-ALv2
"""Sample the repository's storage benchmark inside its existing snapshot and archive phases.

The benchmark's ordinary user owns the workload. The collector runs perf on that child tree
only. Monotonic timestamps when Rust emits its phase summaries bound the attribution window.
The synthetic boot-contention phase is outside the analysis.
"""
import collections
import decimal
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time


LABELS = ["world:", "snapshot:", "archive (everything):", "archive (a move, without libraries/):",
          "restore from an archive:", "restore from a snapshot on the node:"]


def phases(perf, binary, output, data):
    command = ["sudo", perf, "record", "--clockid", "mono", "-F", "99", "-e", "cpu-clock:u",
               "-o", str(data), "--", "sudo", "-u", os.environ["USER"], "env", "BENCH_WORLD_MB=256",
               str(binary), "--ignored", "--nocapture", "--test-threads=1"]
    stamps = []
    with subprocess.Popen(command, stdout=subprocess.PIPE, text=True) as child, output.open("w") as stream:
        for line in child.stdout:
            now = time.monotonic_ns()
            stream.write(line)
            if len(stamps) < len(LABELS) and LABELS[len(stamps)] in line:
                stamps.append(now)
        if child.wait():
            raise RuntimeError("profiled storage benchmark failed")
    if len(stamps) != len(LABELS):
        raise ValueError("missing phase boundaries in the stock benchmark output")
    return stamps


def parse_samples(lines, stamps):
    totals = [collections.Counter() for _ in stamps[1:]]
    for line in lines:
        match = re.match(r"\s*(\d+\.\d+):\s+(\d+)\s+(.+?)\s*$", line)
        if not match:
            continue
        instant = int(decimal.Decimal(match[1]) * 10**9)
        for index, (start, end) in enumerate(zip(stamps, stamps[1:])):
            if start <= instant < end:
                totals[index][match[3]] += int(match[2])
                break
    rows = []
    for index, counts in enumerate(totals):
        wall = stamps[index + 1] - stamps[index]
        rows.append({"phase": LABELS[index + 1], "wall_ns": wall,
                     "sampled_user_ns": sum(counts.values()),
                     "symbols": [{"name": name, "sampled_ns": weight,
                                  "fraction_of_wall": weight / wall}
                                 for name, weight in counts.most_common(25)]})
    if not any(total for total in totals):
        raise ValueError("no attributable user CPU samples")
    return rows


if __name__ == "__main__":
    perf, binary, out = sys.argv[1:]
    directory = Path(out)
    directory.mkdir()
    data = directory / "perf.data"
    stamps = phases(perf, Path(binary).resolve(), directory / "workload.txt", data)
    raw = subprocess.check_output(["sudo", perf, "script", "--ns", "-i", data, "-F", "time,period,sym"],
                                  text=True)
    (directory / "samples.txt").write_text(raw)
    (directory / "profile.json").write_text(json.dumps(
        {"source": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
         "clock": "CLOCK_MONOTONIC", "boundaries_ns": stamps, "phases": parse_samples(raw.splitlines(), stamps)},
        indent=2) + "\n")
