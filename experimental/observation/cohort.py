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
    if timed <= 0 or duration <= 0:
        raise ValueError("incomplete syscall batch")
    if mode in ("raw", "prepared-off"):
        if observed != 0 or warm != 0:
            raise ValueError(f"{mode} reported an observer")
    elif warm <= 0 or observed - warm != timed:
        raise ValueError("incomplete syscall observation")
    if mode == "prepared-on" and (timed != 4096 or warm != 128 or observed != 4224):
        raise ValueError("the active prepared counter missed a fixed fixture event")
    return {"duration_ns": duration, "timed_calls": timed, "observed_including_warmup": observed,
            "observed_warmup": warm, "ns_per_call": duration / timed}


def prepared_build(binary):
    files = {mode: binary.with_name(binary.name + "-" + mode) for mode in ("prepared-off", "prepared-on")}
    verification = binary.with_name("prepared-verification.json")
    proof = json.loads(verification.read_text())
    old = files["prepared-off"].read_bytes()
    new = files["prepared-on"].read_bytes()
    if (len(old) != len(new) or proof["architecture"] != platform.machine()
            or hashlib.sha256(old).hexdigest() != proof["inactive_sha256"]
            or hashlib.sha256(new).hexdigest() != proof["transformed_sha256"]
            or proof["transformed_sha256"] != proof["linked_oracle_sha256"]):
        raise ValueError("measured files do not match the prepared executable build proof")
    differences = (offset for offset, (a, b) in enumerate(zip(old, new, strict=True)) if a != b)
    offset = next(differences, None)
    if offset is None or (old[offset] ^ new[offset]).bit_count() != 1 or next(differences, None) is not None:
        raise ValueError("prepared executables must differ by exactly one bit")
    before = bytes.fromhex(proof["inactive_instruction_hex"])
    after = bytes.fromhex(proof["active_instruction_hex"])
    if proof["section"] != ".pryx_observe_count" or not before or len(before) != len(after):
        raise ValueError("missing fixed counter instruction in the build proof")
    changed = [i for i, (a, b) in enumerate(zip(before, after, strict=True)) if a != b]
    if len(changed) != 1:
        raise ValueError("unexpected instruction change in the build proof")
    start = offset - changed[0]
    if start < 0 or old[start:start+len(before)] != before or new[start:start+len(after)] != after:
        raise ValueError("counter instruction differs from the measured file bytes")
    return {"raw": binary, **files}, {"build_verification": proof,
                                     "verification_sha256": hashlib.sha256(verification.read_bytes()).hexdigest(),
                                     "changed_file_offset": offset, "inactive_byte": old[offset], "active_byte": new[offset]}


def prepared_trials(binary, directory, result, output):
    files, verification = prepared_build(binary)
    modes = ("raw", "prepared-off", "prepared-on")
    trials = result["prepared"] = {**verification, "triples": [], "complete": False,
                                  "forward_order": list(modes), "reverse_order": list(reversed(modes))}
    comparisons = (("active_minus_raw", "prepared-on", "raw"),
                   ("active_minus_inactive", "prepared-on", "prepared-off"),
                   ("inactive_minus_raw", "prepared-off", "raw"))
    for index in range(15):
        order = modes if index % 2 == 0 else modes[::-1]
        record = {"round": index + 1, "order": list(order), "samples": {}}
        trials["triples"].append(record)
        for mode in order:
            row = sample(files[mode], directory, mode)
            record["samples"][mode] = row
            output.write_text(json.dumps(result, indent=2) + "\n")
            if "unavailable" in row:
                result["summary"]["prepared"] = {"verdict": "unavailable", "why": row["unavailable"]}
                return
            if row["timed_calls"] != 4096:
                raise ValueError("prepared comparison requires 4096 real getpid calls per batch")
        samples = record["samples"]
        record["paired_delta_ns_per_call"] = {
            label: samples[active]["ns_per_call"] - samples[control]["ns_per_call"]
            for label, active, control in comparisons
        }
    # Recheck the measured files after the final child finishes.
    if prepared_build(binary)[1] != verification:
        raise ValueError("prepared executable changed during the cohort")
    summary = {}
    for label, _, _ in comparisons:
        values = [record["paired_delta_ns_per_call"][label] for record in trials["triples"]]
        summary[label] = {"paired_median_ns": statistics.median(values),
                          "paired_p95_ns": sorted(values)[math.ceil(.95 * len(values)) - 1]}
    median = summary["active_minus_raw"]["paired_median_ns"]
    result["summary"]["prepared"] = {**summary, "verdict": "within local budget" if median <= 100 else "exceeds local budget"}
    trials["complete"] = True


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    linked = binary.with_name(binary.name + "-link").resolve(strict=True)
    cpu = min(os.sched_getaffinity(0))
    os.sched_setaffinity(0, {cpu})
    result = {"commit": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
              "cpu": cpu, "machine": platform.machine(), "kernel": platform.release(),
              "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
              "link_time_binary_sha256": hashlib.sha256(linked.read_bytes()).hexdigest(), "pairs": [], "summary": {}}
    with tempfile.TemporaryDirectory() as tmp:
        directory = Path(tmp)
        (directory / "allowed").mkdir(mode=0o700)
        (directory / "read-control").write_text("ordinary-readable sentinel")
        unavailable = set()
        methods = ["trace", "notify", "dispatch", "link-time"]
        for iteration in range(15):
            offset = iteration % len(methods)
            for mode in methods[offset:] + methods[:offset]:
                if mode in unavailable:
                    continue
                pair = {"pair": iteration + 1, "method": mode}
                for variant in (["raw", mode] if iteration % 2 == 0 else [mode, "raw"]):
                    pair[variant] = sample(linked if variant == "link-time" else binary, directory, variant)
                    if "unavailable" in pair[variant]:
                        unavailable.add(mode)
                        result["summary"][mode] = {"verdict": "unavailable", "why": pair[variant]["unavailable"]}
                        break
                if mode not in unavailable:
                    if pair[mode]["timed_calls"] != pair["raw"]["timed_calls"]:
                        raise ValueError("pair has different syscall batch sizes")
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
        try:
            prepared_trials(binary, directory, result, args.output)
        finally:
            args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result["summary"], indent=2))


if __name__ == "__main__":
    main()
