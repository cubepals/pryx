#!/usr/bin/env python3
"""Run one registered native page experiment and preserve raw samples before summarizing.

The experiment owns an empty output directory. Host topology, kernel backing policy, the
binary digests and benchmark parameters accompany each table. The sleep fixture verifies
the full image outside the timer. VM integration remains a separate acceptance measurement.
"""
import csv
import hashlib
import math
import os
from pathlib import Path
import statistics
import subprocess
import sys
from collections import defaultdict

ROOT = Path(__file__).resolve().parent
BIN = ROOT / "target/release/examples"
MEMORY = "trial,page,total,threads,offset,passes,op,backend,store,prefetch,ns".split(",")
SLEEP = "trial,page,total,op,backend,store,prefetch,ns,blocks".split(",")


def run(args, **kwargs):
    return subprocess.run(args, check=True, text=True, **kwargs)


def llc_bytes():
    cpu = min(os.sched_getaffinity(0))
    sizes = []
    for cache in Path(f"/sys/devices/system/cpu/cpu{cpu}/cache").glob("index*"):
        if (cache / "type").read_text().strip() not in {"Data", "Unified"}:
            continue
        size = (cache / "size").read_text().strip()
        scale = {"K": 1024, "M": 1 << 20, "G": 1 << 30}
        amount = int(size[:-1])*scale[size[-1]] if size[-1] in scale else int(size)
        sizes.append((int((cache / "level").read_text()), amount))
    return max(sizes)[1]


def memory(out):
    llc = llc_bytes()
    with (out / "backing.txt").open("w") as metadata:
        for page in (4096, 16384, 65536, 2 << 20):
            for threads in (1, 2):
                quantum = page * threads
                hot = max(quantum, (min(llc // 4, 4 << 20) // quantum) * quantum)
                cold = math.ceil(max(128 << 20, 4 * llc) / quantum) * quantum
                for working, size in (("cache", hot), ("dram", cold)):
                    if working == "cache" and 2 * size > llc:
                        raise ValueError("The two arrays do not fit in the detected LLC")
                    for offset in (0, 1):
                        for thp in (("off", "on") if page == 2 << 20 else ("default",)):
                            name = f"{working}-{page}-{threads}-{offset}-{thp}"
                            env = os.environ.copy()
                            if thp != "default":
                                env["PRYX_LAB_THP"] = thp
                            with (out / f"{name}.csv").open("w") as raw:
                                print(",".join(MEMORY), file=raw, flush=True)
                                run([BIN / "bandwidth", *map(str, (page, size, threads, offset, 15))],
                                    env=env, stdout=raw, stderr=metadata)


def table(out):
    rows = []
    for path in sorted(out.glob("*.csv")):
        samples = defaultdict(list)
        with path.open() as f:
            for row in csv.DictReader(f):
                samples[row["op"], row["backend"], row["store"], row["prefetch"]].append(row)
        for key, group in samples.items():
            assert len(group) == 15 and len({r["trial"] for r in group}) == 15
            times = sorted(int(r["ns"]) for r in group)
            p50, p95 = statistics.median(times), times[math.ceil(.95 * len(times)) - 1]
            bytes_per_sample = int(group[0]["total"]) * int(group[0].get("passes", 1))
            rate = bytes_per_sample / p50
            fraction = ""
            if "threads" in group[0]:
                base = {"Diff":"ReadPair", "Clear":"Clear", "Copy":"Copy"}.get(key[0], "Read")
                if key[0] == "ReadPair":
                    base = "ReadPair"
                calibration = samples[(base, "library", "Native", "0")]
                fraction = f"{statistics.median(int(r['ns']) for r in calibration) / p50:.4f}"
            rows.append([path.stem, *key, int(p50), int(p95), f"{rate:.4f}", fraction])
    with (out / "summary.tsv").open("w") as f:
        writer = csv.writer(f, delimiter="\t")
        writer.writerow(["cohort", "operation", "backend", "store", "prefetch_bytes", "median_ns",
                         "p95_ns", "logical_GB_s", "fraction_of_bandwidth_control"])
        writer.writerows(rows)


def main():
    phase, name = sys.argv[1:]
    out = Path(name).resolve()
    out.mkdir(exist_ok=False)
    with (out / "host.txt").open("w") as host:
        for cmd in (["git", "rev-parse", "HEAD"], ["uname","-a"], ["rustc","-vV"], ["lscpu"],
                    ["findmnt","-T",str(out)], ["cat","/sys/kernel/mm/transparent_hugepage/enabled"]):
            run(cmd, stdout=host)
        print("affinity", sorted(os.sched_getaffinity(0)), "last-level cache bytes", llc_bytes(), file=host)
        cpu = min(os.sched_getaffinity(0))
        governor=Path(f"/sys/devices/system/cpu/cpu{cpu}/cpufreq/scaling_governor")
        print("frequency governor", governor.read_text().strip() if governor.exists() else "unexposed", file=host)
        for binary in ("bandwidth", "sleep_file"):
            print(binary,hashlib.sha256((BIN / binary).read_bytes()).hexdigest(),file=host)
    if phase == "memory":
        memory(out)
    elif phase == "sleep":
        with (out / "sleep.csv").open("w") as raw:
            print(",".join(SLEEP),file=raw,flush=True)
            run([BIN / "sleep_file", out / "fixture", "4096", "256", "15"],stdout=raw)
    else:
        raise ValueError("phase must be memory or sleep")
    table(out)


if __name__ == "__main__":
    main()
