# SPDX-License-Identifier: FSL-1.1-ALv2
"""Attribute perf cpu-clock periods to storage_bench phases using CLOCK_MONOTONIC."""
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
TIMESTAMP = re.compile(r"^\s*(\d+\.\d+):\s*(.*?)\s*$")
SAMPLE = re.compile(r"^(\d+)\s+(?:0x)?[a-fA-F0-9]+\s+(.*?)\s+\((.*)\)$")


def phases(command, output, log):
    observations = []
    with log.open("w") as errors, output.open("w") as stream:
        with subprocess.Popen(command, stdout=subprocess.PIPE, stderr=errors, text=True) as child:
            for line in child.stdout:
                now = time.monotonic_ns()
                stream.write(line)
                if len(observations) >= len(LABELS):
                    continue
                label = LABELS[len(observations)]
                if label in line:
                    tail = line.split(label, 1)[1]
                    duration = re.match(r"\s*(\d+\.\d+) s(?:\b|,)", tail)
                    observations.append({"end_of": label, "observed_ns": now,
                                         "workload_reported_s": float(duration[1]) if duration else None})
        code = child.returncode
    return code, observations


def parse_samples(lines, observations):
    stamps = [item["observed_ns"] for item in observations]
    weights = [collections.Counter() for _ in stamps[1:]]
    samples = [collections.Counter() for _ in stamps[1:]]
    invalid = [0 for _ in stamps[1:]]
    for line in lines:
        header = TIMESTAMP.match(line)
        if not header:
            continue
        instant = int(decimal.Decimal(header[1]) * 10**9)
        for index, (start, end) in enumerate(zip(stamps, stamps[1:])):
            if start <= instant < end:
                fields = SAMPLE.match(header[2])
                if not fields or int(fields[1]) <= 0:
                    invalid[index] += 1
                else:
                    key = fields[2], fields[3]
                    weights[index][key] += int(fields[1])
                    samples[index][key] += 1
                break
    rows = []
    for index, counts in enumerate(weights):
        wall = stamps[index + 1] - stamps[index]
        if wall <= 0:
            raise ValueError("nonpositive phase observation window")
        unknown = sum(weight for (name, _), weight in counts.items()
                      if not name or "[unknown]" in name or re.fullmatch(r"(?:0x[\da-fA-F]+|[\da-fA-F]{6,})", name))
        total = sum(counts.values())
        rows.append({"rankable": bool(total) and invalid[index] == 0 and total - unknown >= 0.9 * total,
                     "phase": LABELS[index + 1], "wall_ns": wall,
                     "workload_reported_s": observations[index + 1]["workload_reported_s"],
                     "sample_count": sum(samples[index].values()), "malformed_sample_count": invalid[index],
                     "unresolved_sample_ns": unknown, "sampled_user_ns": total,
                     "symbols": [{"name": name, "dso": dso, "samples": samples[index][(name, dso)],
                                  "sampled_ns": weight, "fraction_of_wall": weight / wall}
                                 for (name, dso), weight in counts.most_common(25)]})
    return rows


def main(perf, binary, out):
    directory = Path(out)
    directory.mkdir(exist_ok=True)
    data = directory / "perf.data"
    binary = str(Path(binary).resolve())
    workload = ["env", "BENCH_WORLD_MB=256", f"BENCH_DIR={os.environ['RUNNER_TEMP']}",
                binary, "--ignored", "--nocapture", "--test-threads=1"]
    options = ["--clockid", "mono", "-F", "99", "-e", "cpu-clock:u"]
    report = {"source": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
              "clock": "CLOCK_MONOTONIC", "event": "cpu-clock:u", "frequency_hz": 99, "phases": []}

    def save():
        (directory / "profile.json").write_text(json.dumps(report, indent=2) + "\n")

    # Test the exact event/record attributes separately; missing capability must not conceal a workload failure.
    with (directory / "sampling-probe.txt").open("w") as log:
        probe = subprocess.run(["sudo", perf, "record", *options, "-o", str(directory / "probe.data"),
                                "--", "/usr/bin/true"], stdout=log, stderr=subprocess.STDOUT)
    if probe.returncode:
        report.update(status="sampling unavailable", reason=(directory / "sampling-probe.txt").read_text().strip())
        command = workload
    else:
        command = ["sudo", perf, "record", *options, "-o", str(data), "--",
                   "sudo", "-u", os.environ["USER"], *workload]
    code, observations = phases(command, directory / "workload.txt", directory / "record-stderr.txt")
    report["boundaries"] = observations
    if code or len(observations) != len(LABELS) or any(item["workload_reported_s"] is None for item in observations[1:]):
        report.update(status="workload or recording failed", returncode=code)
        save()
        raise RuntimeError("see workload.txt, record-stderr.txt and profile.json")
    raw = ""
    if not probe.returncode:
        with (directory / "script-stderr.txt").open("w") as log:
            decode = subprocess.run(["sudo", perf, "script", "--ns", "-i", str(data), "-F", "time,period,ip,sym,dso"],
                                    text=True, stdout=subprocess.PIPE, stderr=log)
        raw = decode.stdout
        (directory / "samples.txt").write_text(raw)
        if decode.returncode:
            report.update(status="symbolization failed", returncode=decode.returncode)
            save()
            raise RuntimeError("see script-stderr.txt and profile.json")
    report["phases"] = parse_samples(raw.splitlines(), observations)
    if not probe.returncode:
        report["status"] = ("sampled" if any(phase["rankable"] for phase in report["phases"])
                            else "insufficient attributable samples")
    save()
    print("storage profile", report["status"])


if __name__ == "__main__":
    main(*sys.argv[1:])
