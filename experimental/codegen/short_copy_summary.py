#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Summarize paired short-copy batch costs at each size and alignment on one native worker."""
import csv
from contextlib import redirect_stdout
from io import StringIO
from collections import defaultdict
from fractions import Fraction
import re
import statistics
import sys


LENGTHS = (1, 4, 8, 15, 16, 17, 31, 32, 48, 63, 64, 65, 96, 127, 128, 129, 192, 256, 512, 4096)
OFFSETS = ((0, 0), (1, 3))


def flag(text):
    if text in ("true", "1"):
        return True
    if text in ("false", "0"):
        return False
    raise ValueError(f"invalid CPU feature flag {text!r}")


def check_method(name, family, group, prefix, length):
    if name == "libc-memcpy" and (family, group, prefix) == ("libc", 0, 0):
        return
    if name == "copy-w1-u1" and (family, group, prefix) == ("rep", 1, length):
        return
    plan = re.fullmatch(r"copy-w(16|32|64)-u(1|2|4)", name)
    if (plan and family == "vector" and group == int(plan[1]) * int(plan[2])
            and prefix == length // group * group):
        return
    raise ValueError(f"inconsistent copy method metadata: {name}, {family}, {group}, {prefix}, {length}")


def read(source):
    cases = defaultdict(lambda: defaultdict(dict))
    hosts = set()
    methods = set()
    for row in csv.DictReader(source):
        cpu, length, src, dst, pair, order, group, prefix, copies, elapsed = (
            int(row[k]) for k in ("cpu", "length", "src_offset", "dst_offset", "pair", "order",
                                  "group_bytes", "native_prefix_bytes", "copies", "elapsed_ns"))
        if min(cpu, length, src, dst, pair, order, group, prefix) < 0 or min(copies, elapsed) <= 0:
            raise ValueError("invalid size, worker, sequence or batch timing")
        hosts.add((cpu, flag(row["erms"]), flag(row["fsrm"])))
        method = row["implementation"]
        check_method(method, row["family"], group, prefix, length)
        methods.add(method)
        trials = cases[length, src, dst][method]
        if pair in trials:
            raise ValueError(f"duplicate sample for {method} in trial {pair}")
        trials[pair] = (order, copies, Fraction(elapsed, copies), prefix, group)
    expected = {(length, src, dst) for length in LENGTHS for src, dst in OFFSETS}
    if set(cases) != expected:
        raise ValueError(f"incomplete or unexpected length/alignment matrix (missing {len(expected - set(cases))})")
    if len(hosts) != 1 or "libc-memcpy" not in methods:
        raise ValueError("expected one native worker and the maintained libc comparator")
    host, = hosts
    if (host[1] or host[2]) and "copy-w1-u1" not in methods:
        raise ValueError("missing REP cohort on a runner reporting string features")
    return host, methods, cases


def summarize(source):
    (cpu, erms, fsrm), methods, cases = read(source)
    print(f"## Short-copy characterization on CPU {cpu}\n")
    print(f"ERMS was {erms} and FSRM was {fsrm} throughout. Each cell uses 15 reversed-order rounds. "
          "Median and nearest-rank p95 use the per-round average ns/copy. "
          "These measurements describe this worker; its CPU feature flags were held fixed.\n")
    print("| Bytes | Src offset | Dst offset | Fastest active vector | REP/libc median | REP/libc p95 | "
          "REP/vector median | REP/vector p95 | Meets both comparisons |")
    print("| ---: | ---: | ---: | --- | ---: | ---: | ---: | ---: | --- |")
    active_rows, tail_rows = [], []
    for (length, src, dst), case in sorted(cases.items()):
        if set(case) != methods:
            raise ValueError("case is missing an implementation")
        trials = sorted(case["libc-memcpy"])
        if trials not in (list(range(15)), list(range(1, 16))) or any(sorted(rows) != trials for rows in case.values()):
            raise ValueError("expected exactly 15 matching measured rounds")
        previous = None
        for pair in trials:
            orders = [case[name][pair][0] for name in case]
            if len(set(orders)) != len(orders) or len({rows[pair][1] for rows in case.values()}) != 1:
                raise ValueError("duplicate order or inconsistent batch sizes in a round")
            sequence = sorted(case, key=lambda name: case[name][pair][0])
            if previous is not None and sequence != previous[::-1]:
                raise ValueError("method order did not reverse")
            previous = sequence
        stats, active = {}, []
        for name, rows in sorted(case.items()):
            means = sorted(row[2] for row in rows.values())
            # For exactly fifteen samples, ceil(.95 * 15) is the fifteenth.
            median, p95 = statistics.median(means), means[-1]
            prefix, group = rows[trials[0]][3:]
            is_vector = name not in ("copy-w1-u1", "libc-memcpy")
            stats[name] = (median, p95)
            if is_vector and prefix:
                active.append(name)
            record = (length, src, dst, name, group, prefix, float(median), float(p95))
            (tail_rows if is_vector and not prefix else active_rows).append(record)
        libc = stats["libc-memcpy"]
        rep = stats.get("copy-w1-u1")
        vector = min(active, key=lambda name: (stats[name][0], name)) if active else None
        def ratios(base):
            return (rep[0] / base[0], rep[1] / base[1]) if rep and base else (None, None)
        lib_m, lib_p = ratios(libc)
        vec_m, vec_p = ratios(stats[vector] if vector else None)
        verdict = "unclassified"
        if rep and vector:
            # Tied medians make both comparators fastest; use the smaller p95 in that case.
            faster = min(libc, stats[vector])
            verdict = ("yes" if max(lib_m, vec_m) <= Fraction(9, 10)
                       and rep[1] / faster[1] <= Fraction(21, 20) else "no")
        ratios_text = " | ".join("—" if x is None else f"{float(x):.4f}" for x in (lib_m, lib_p, vec_m, vec_p))
        print(f"| {length} | {src} | {dst} | {vector or '—'} | {ratios_text} | {verdict} |")
    print("\nA row qualifies when REP median is at most 0.90× both libc and the fastest-by-median active vector. "
          "REP p95 must be at most 1.05× the p95 of the faster-by-median of those two comparators. "
          "An exact median tie uses the lower comparator p95. Vector eligibility requires "
          "at least one full native group at that length.\n")
    for title, records in (("Library, REP and active vectors", active_rows), ("Tail-only vector plans", tail_rows)):
        print(f"### {title}\n")
        if not records:
            print("No rows.\n")
            continue
        print("| Bytes | Src offset | Dst offset | Implementation | Group bytes | Native bytes | Median ns/copy | p95 ns/copy |")
        print("| ---: | ---: | ---: | --- | ---: | ---: | ---: | ---: |")
        for length, src, dst, name, group, prefix, median, p95 in records:
            print(f"| {length} | {src} | {dst} | {name} | {group} | {prefix} | {median:.4f} | {p95:.4f} |")
        print()


def self_check():
    """Synthetic data exposes an omitted whole case and reversed median/p95 ordering."""
    fields = "pair order cpu erms fsrm length src_offset dst_offset implementation family group_bytes native_prefix_bytes copies elapsed_ns"
    for omit in (False, True):
        raw = StringIO()
        writer = csv.writer(raw)
        writer.writerow(fields.split())
        for length in LENGTHS:
            for src, dst in OFFSETS:
                if omit and (length, src, dst) == (1, 0, 0):
                    continue
                for pair in range(15):
                    cases = [("libc-memcpy", "libc", 0, 0, 220 if pair == 14 else 90),
                             ("copy-w1-u1", "rep", 1, length, 200 if pair == 14 else 80),
                             ("copy-w16-u1", "vector", 16, length // 16 * 16, 130 if pair == 14 else 120)]
                    for order, (name, family, group, prefix, time) in enumerate(cases[::-1] if pair % 2 else cases):
                        writer.writerow((pair, order, 0, 1, 0, length, src, dst, name, family, group, prefix, 10, time))
        raw.seek(0)
        output = StringIO()
        if omit:
            try:
                read(raw)
            except ValueError as error:
                assert "length/alignment matrix" in str(error)
            else:
                raise AssertionError("accepted a missing complete case")
        else:
            with redirect_stdout(output):
                summarize(raw)
            # libc has the better median but worse p95. Only libc's p95 sets this gate.
            assert "| 16 | 0 | 0 | copy-w16-u1 | 0.8889 | 0.9091 | 0.6667 | 1.5385 | yes |" in output.getvalue()


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-check"]:
        self_check()
    else:
        with open(sys.argv[1], newline="") as source:
            summarize(source)
