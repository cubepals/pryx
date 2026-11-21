#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Report native generation cost, paired cached-writer savings and descriptive break-even sleeps."""
import csv
from collections import defaultdict
from io import StringIO
from pathlib import Path
import statistics
import sys


TRIALS = set(range(15))
CANDIDATES = ("generated-zero", "generated-stage-copy")
IMAGE_BYTES = 256 << 20


def strongest_static(source):
    costs = defaultdict(dict)
    generated = {}
    cpus = set()
    for row in csv.DictReader(source):
        method = row["implementation"]
        pair, elapsed = int(row["pair"]), int(row["elapsed_ns"])
        flag = row["generated"]
        if (pair in costs[method] or elapsed <= 0 or flag not in ("true", "false")
                or generated.setdefault(method, flag) != flag or int(row["logical_bytes"]) != IMAGE_BYTES):
            raise ValueError("invalid baseline row or changing implementation classification")
        cpus.add(int(row["cpu"]))
        costs[method][pair] = elapsed
    if len(cpus) != 1 or any(set(values) != TRIALS for values in costs.values()):
        raise ValueError("baseline requires 15 complete trials on one CPU")
    if {name for name, flag in generated.items() if flag == "true"} != set(CANDIDATES):
        raise ValueError("baseline is missing a generated candidate")
    static = [name for name, flag in generated.items() if flag == "false"]
    if not static:
        raise ValueError("baseline has no static writer")
    # summary.py uses the first CSV occurrence to resolve equal lowest medians.
    return min(static, key=lambda name: statistics.median(costs[name].values())), cpus.pop()


def split_trials(source, winner, cpu):
    trials = {name: {} for name in CANDIDATES}
    for row in csv.DictReader(source):
        method, pair = row["candidate"], int(row["pair"])
        if (method not in trials or pair not in TRIALS or pair in trials[method] or row["static_writer"] != winner
                or int(row["cpu"]) != cpu or int(row["logical_bytes"]) != IMAGE_BYTES
                or row["order"] != ("static-first" if pair % 2 == 0 else "cached-first")):
            raise ValueError("unexpected split candidate, comparator, CPU, trial or order")
        values = tuple(int(row[k]) for k in ("generation_ns", "cached_writer_ns", "static_writer_ns",
                                            "cached_allocated_bytes", "static_allocated_bytes"))
        if min(values[:3]) <= 0 or min(values[3:]) < 0:
            raise ValueError("invalid generation duration, writer duration or physical allocation")
        trials[method][pair] = values
    if any(set(observations) != TRIALS for observations in trials.values()):
        raise ValueError("both candidates require 15 complete reversed-order pairs")
    return trials


def ceil_ratio(numerator, denominator):
    return (numerator + denominator - 1) // denominator


def summarize(pairs, cohort):
    winner, cpu = strongest_static(pairs)
    trials = split_trials(cohort, winner, cpu)
    output = [
        f"## Cached writers and generation on CPU {cpu}", "",
        f"The fastest static writer in the original complete 256 MiB file cohort was {winner}. "
        "The separate cohort has one warmup per writer and 15 matched trials. Each trial times fresh detection "
        "and new handle creation through finalization in an already warmed process. Process startup and final handle "
        "destruction/unmapping are outside that generation timer. Cached writers reuse the same generated kernel for every trial. "
        "Cached and static writer times include data sync. Both exclude generation and verification readback.", "",
        "| Candidate | Generation median ns | 4th–12th generation ns | Cached median ms | Static median ms | Median paired saving ms | "
        "4th–12th saving ms | Positive pairs | Estimated break-even sleeps | Sensitivity range, sleeps |",
        "| --- | ---: | --- | ---: | ---: | ---: | --- | ---: | ---: | ---: |",
    ]
    allocations = []
    for method in CANDIDATES:
        generation, cached, static, cached_bytes, static_bytes = zip(*(trials[method][p] for p in range(15)))
        generation = sorted(generation)
        savings = sorted(s - c for s, c in zip(static, cached))
        gen, med = generation[7], savings[7]
        low, high = savings[3], savings[11]
        positive = sum(s > 0 for s in savings)
        estimate = sensitivity = "unestablished"
        if low > 0:
            estimate = str(ceil_ratio(gen, med))
            sensitivity = f"{ceil_ratio(generation[3], high)}–{ceil_ratio(generation[11], low)}"
        output.append(f"| {method} | {gen} | {generation[3]} to {generation[11]} | {statistics.median(cached)/1e6:.4f} | {statistics.median(static)/1e6:.4f} "
                      f"| {med/1e6:.4f} | {low/1e6:.4f} to {high/1e6:.4f} | {positive}/15 | {estimate} | {sensitivity} |")
        allocations.append(f"{method} allocated median {statistics.median(cached_bytes)/2**20:.3f} MiB "
                           f"versus {statistics.median(static_bytes)/2**20:.3f} MiB for {winner}.")
    output += ["", *allocations, "",
               "Paired savings subtract cached writer time from the corresponding static time. For 15 independent "
               "continuous draws the fourth and twelfth ordered savings give a 96.484% two-sided interval for "
               "the population median. The fourth alone gives a 98.242% one-sided lower bound. Repeated trials "
               "on one runner can be correlated.", "",
               "A positive fourth saving requires at least 12 strictly positive pairs. Only then is break-even "
               "estimated by rounding median fresh generation time divided by median paired saving upward to whole "
               "256 MiB sleeps. The sensitivity range divides the fourth generation time by the twelfth saving, "
               "and the twelfth generation time by the fourth saving, rounding each upward. That range combines "
               "marginal bounds and is descriptive. The original inclusive end-to-end gate still applies."]
    return "\n".join(output)


def self_check():
    pairs, split = StringIO(), StringIO()
    a, b = csv.writer(pairs), csv.writer(split)
    a.writerow("pair order implementation generated cpu logical_bytes allocated_bytes elapsed_ns".split())
    b.writerow(("pair candidate static_writer order cpu logical_bytes generation_ns cached_writer_ns static_writer_ns "
                "cached_allocated_bytes static_allocated_bytes").split())
    for pair in range(15):
        for order, name in enumerate(("earliest", "tied", *CANDIDATES)):
            a.writerow((pair, order, name, str(name in CANDIDATES).lower(), 3, IMAGE_BYTES, 4096, 200))
        for name in CANDIDATES:
            cost = 161 if name == CANDIDATES[0] or pair >= 4 else 201
            b.writerow((pair, name, "earliest", "cached-first" if pair % 2 else "static-first",
                        3, IMAGE_BYTES, 100, cost, 200, 8192, 8192))
    p, s = pairs.getvalue(), split.getvalue()
    assert strongest_static(StringIO(p)) == ("earliest", 3)
    result = summarize(StringIO(p), StringIO(s))
    assert result.count("| 100 | 100 to 100 |") == 2
    assert "15/15 | 3 | 3–3 |" in result and "11/15 | unestablished | unestablished |" in result
    for invalid in (s.replace("earliest", "tied"), s.replace("cached-first", "static-first"),
                    "\n".join(s.splitlines()[:-1])):
        try:
            summarize(StringIO(p), StringIO(invalid))
        except ValueError:
            pass
        else:
            raise AssertionError("accepted an incorrect comparator, ordering or incomplete cohort")


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-check"]:
        self_check()
    elif len(sys.argv) == 3 and sys.argv[1] == "--winner":
        with Path(sys.argv[2]).open(newline="") as source:
            print(strongest_static(source)[0])
    elif len(sys.argv) == 3:
        with Path(sys.argv[1]).open(newline="") as pairs, Path(sys.argv[2]).open(newline="") as split:
            print(summarize(pairs, split))
    else:
        raise SystemExit("usage: split_summary.py --winner PAIRS.csv | PAIRS.csv SPLIT.csv | --self-check")
