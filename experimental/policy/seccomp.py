# SPDX-License-Identifier: FSL-1.1-ALv2
"""Measure syscall batches in fresh processes, with paired control and filter observations."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import statistics
import subprocess


def distribution(values):
    return {"p50": statistics.median(values), "p95": sorted(values)[math.ceil(0.95 * len(values)) - 1]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--pairs", type=int, default=31)
    parser.add_argument("--iterations", type=int, default=100_000)
    args = parser.parse_args()
    if args.pairs < 2 or args.iterations < 1:
        parser.error("at least two pairs and one iteration are required")

    binary = args.binary.resolve(strict=True)
    cpu = min(os.sched_getaffinity(0))
    os.sched_setaffinity(0, {cpu})
    samples = []
    # Alternate which cohort runs first, and which syscall runs first.
    for pair in range(args.pairs):
        calls = ["getpid", "bind"] if pair % 4 < 2 else ["bind", "getpid"]
        for call in calls:
            sample = {"pair": pair, "call": call}
            modes = ["host", "process"] if pair % 2 == 0 else ["process", "host"]
            for mode in modes:
                duration = subprocess.check_output([binary, mode, call, str(args.iterations)], text=True)
                sample[mode] = int(duration) / args.iterations
            sample["process_minus_host"] = sample["process"] - sample["host"]
            samples.append(sample)

    summaries = {}
    for call in ["getpid", "bind"]:
        rows = [row for row in samples if row["call"] == call]
        summaries[call] = {key: distribution([row[key] for row in rows]) for key in
                           ["host", "process", "process_minus_host"]}
    print(json.dumps({"arch": platform.machine(), "kernel": platform.release(), "cpu": cpu,
                      "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                      "clock": "Rust Instant", "unit": "ns/syscall", "iterations": args.iterations,
                      "summary": summaries, "samples": samples}, indent=2))


if __name__ == "__main__":
    main()
