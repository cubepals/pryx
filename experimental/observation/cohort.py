# SPDX-License-Identifier: FSL-1.1-ALv2
"""Paired native syscall costs on one CPU. Only complete, validated pairs receive a local verdict."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import statistics
import subprocess
import tempfile


def sample(binary, directory, mode):
    result = subprocess.run([binary, mode], cwd=directory, capture_output=True, text=True, timeout=50)
    if result.returncode == 77:
        return {"unavailable": result.stderr.strip() or "host does not provide the production policy or observer"}
    if result.returncode:
        raise RuntimeError(f"{mode} fixture failed ({result.returncode}): {result.stderr}")
    duration, timed, observed, warm = map(int, result.stdout.split())
    if timed <= 0 or duration <= 0 or (mode != "raw" and observed - warm != timed):
        raise ValueError("incomplete syscall observation")
    return {"duration_ns": duration, "timed_calls": timed, "observed_including_warmup": observed,
            "observed_warmup": warm, "ns_per_call": duration / timed}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    cpu = min(os.sched_getaffinity(0))
    os.sched_setaffinity(0, {cpu})
    result = {"commit": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
              "cpu": cpu, "machine": platform.machine(), "kernel": platform.release(),
              "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "pairs": [], "summary": {}}
    with tempfile.TemporaryDirectory() as tmp:
        directory = Path(tmp)
        (directory / "allowed").mkdir(mode=0o700)
        (directory / "read-control").write_text("ordinary-readable sentinel")
        unavailable = set()
        methods = ["trace", "notify", "dispatch"]
        for iteration in range(15):
            offset = iteration % len(methods)
            for mode in methods[offset:] + methods[:offset]:
                if mode in unavailable:
                    continue
                pair = {"pair": iteration + 1, "method": mode}
                for variant in (["raw", mode] if iteration % 2 == 0 else [mode, "raw"]):
                    pair[variant] = sample(binary, directory, variant)
                    if "unavailable" in pair[variant]:
                        unavailable.add(mode)
                        result["summary"][mode] = {"verdict": "unavailable", "why": pair[variant]["unavailable"]}
                        break
                if mode not in unavailable:
                    pair["delta_ns"] = pair[mode]["ns_per_call"] - pair["raw"]["ns_per_call"]
                result["pairs"].append(pair)
                args.output.write_text(json.dumps(result, indent=2) + "\n")
    for mode in methods:
        deltas = [p["delta_ns"] for p in result["pairs"] if p["method"] == mode and "delta_ns" in p]
        if len(deltas) == 15:
            median = statistics.median(deltas)
            result["summary"][mode] = {"paired_median_ns": median,
                    "paired_p95_ns": sorted(deltas)[math.ceil(0.95 * len(deltas)) - 1],
                    "verdict": "within local budget" if median <= 100 else "exceeds local budget"}
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result["summary"], indent=2))


if __name__ == "__main__":
    main()
