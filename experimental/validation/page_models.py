#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Compare 15 measured page/call pairs with the same A1 loop on the native CPU."""
import argparse
import csv
import hashlib
import importlib.util
import json
import os
import platform
import re
import statistics
import subprocess
from collections import defaultdict
from pathlib import Path
from ruamel.yaml import YAML

PIN = "07b4732183b684ab1775951314c3b5d3ae99ba74"
OSACA_CPUS = {"znver1": "ZEN1", "znver2": "ZEN2", "znver3": "ZEN3", "znver4": "ZEN4",
              "haswell": "HSW", "broadwell": "BDW", "skylake-avx512": "SKX", "icelake-server": "ICX",
              "sapphirerapids": "SPR", "neoverse-n1": "N1", "neoverse-v2": "V2"}
CLOCK_FIELDS = ("kernel_cycles", "kernel_enabled", "kernel_running", "call_cycles", "call_enabled", "call_running")


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def command(args, **kwargs):
    return subprocess.run(args, capture_output=True, text=True, **kwargs)


def load_generator(pages):
    path = pages / "machine/reports/generate.py"
    spec = importlib.util.spec_from_file_location("pinned_a1_paths", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def inputs(pages, out, arch):
    generator = load_generator(pages)
    result = {}
    for symbol, body in generator.expand(arch).items():
        name = symbol.removeprefix("pryx_page_")
        report = pages / "machine/reports" / f"{arch}-{name}.txt"
        fixed_model = report.read_text()
        for case, loop, hint in generator.scenarios(symbol, arch):
            hot = generator.hot_path(body, loop, hint)
            start = f"Path: {case}\n\n{hot}\nllvm-mca"
            if start not in fixed_model:
                raise ValueError(f"{report} differs from expanded input {case}")
            section = fixed_model.split(start, 1)[1].split("\nPath:", 1)[0]
            estimate = re.search(r"Block RThroughput:\s+([\d.]+)", section)
            description = {"path": case, "pinned_report": report.name, "report_sha256": sha(report),
                           "pinned_fixed_cpu": generator.MODELS[arch][0],
                           "pinned_rthroughput": float(estimate[1]) if estimate else None}
            reason = None
            if name.endswith("_rep"):
                reason = "variable-count string engine; single-instruction model does not predict a page"
            elif name == "clear_zva":
                reason = "DC ZVA cache maintenance has no validated per-block model"
            elif name == "zva64":
                reason = "one-time capability check; no repeatable data-path throughput"
            if reason:
                description["pinned_instruction_only"] = description["pinned_rthroughput"]
                description["pinned_rthroughput"] = None
            result[name, bool(hint)] = (hot, description, reason)
    for name in ("rust-copy", "rust-clear"):
        source = out / (name + ".s")
        description = {"path": "ordinary cached Rust vector transfer", "pinned_fixed_cpu": None,
                       "pinned_report": None, "pinned_rthroughput": None}
        if source.is_file():
            description["emitted_bytes_per_block"] = int((out / (name + ".width")).read_text())
            description["source_sha256"] = sha(source)
            result[name, False] = (source.read_text(), description, None)
        else:
            lowering = out / (name + "-lowering.txt")
            reason = lowering.read_text().splitlines()[0] if lowering.is_file() else "no identified fixed emitted loop"
            result[name, False] = (None, description, reason)
    return result


def validate_cohorts(paths, groups):
    # A1 takes these fixed strides in its assembled full-traversal paths.
    # REP and ordinary Rust raw records use a complete page before lowering analysis.
    units = {"diff_avx2": 32, "value_avx2": 32, "diff_avx512": 64, "value_avx512": 64,
             "diff_neon": 16, "value_neon": 16, "hash_lanes": 16, "clear_zva": 64,
             "copy_nt": 32, "clear_nt": 32, "copy_gpr": 8, "clear_gpr": 8,
             "copy_rep": 4096, "clear_rep": 4096, "rust-copy": 4096, "rust-clear": 4096}
    expected = {(name, units[name] if hinted else 0, fill) for name, hinted in paths if name != "zva64"
                for fill in ((0, 90) if name.startswith("value_") else (90,))}
    if set(groups) != expected:
        raise ValueError("native scan values or prefetch distances differ from the A1 report set")
    for (name, _, _), rows in groups.items():
        if any(int(row["unit_bytes"]) != units[name] or int(row["page_bytes"]) != 4096 for row in rows):
            raise ValueError("native unit size differs from the A1 full-traversal path")


def llvm_model(hot, arch, cpu, out):
    if not cpu or cpu.startswith("generic") or cpu in ("unknown", "x86-64"):
        return {"reason": "unrecognized native scheduling CPU"}
    result = command(["llvm-mca-19", f"-mtriple={arch}", "-mcpu=native", "-iterations=100"], input=hot)
    out.with_suffix(".llvm.txt").write_text(result.stdout + result.stderr)
    estimate = re.search(r"Block RThroughput:\s+([\d.]+)", result.stdout)
    cycles = re.search(r"Total Cycles:\s+(\d+)", result.stdout)
    if result.returncode or result.stderr.strip() or not estimate or not cycles:
        return {"reason": "LLVM warned, rejected the path, or lacks a complete scheduling model"}
    return {"cycles_per_block": float(estimate[1]), "simulated_cycles_per_iteration": int(cycles[1]) / 100}


def osaca_model(hot, cpu, out):
    if cpu not in OSACA_CPUS:
        return {"reason": "no selected matching OSACA scheduling CPU"}
    model = OSACA_CPUS[cpu]
    yam = out.with_suffix(".osaca.yaml")
    result = command(["osaca", "--arch", model, "--consider-flag-deps", "--yaml-out", yam, "/dev/stdin"], input=hot)
    out.with_suffix(".osaca.txt").write_text(result.stdout + result.stderr)
    if result.returncode or not yam.is_file():
        return {"cpu_model": model, "reason": "OSACA could not model this path"}
    parsed = YAML(typ="base").load(yam.read_text())
    if not parsed or parsed.get("Warnings") or "No final analysis" in result.stdout:
        return {"cpu_model": model, "reason": "OSACA withheld a complete prediction"}
    summary = parsed.get("Summary") or {}
    pressure = summary.get("PortPressure")
    if not pressure or summary.get("LCD") is None:
        return {"cpu_model": model, "reason": "OSACA omitted dependence or throughput data"}
    return {"cpu_model": model, "cycles_per_block": max(float(summary["LCD"]), *(float(v) for v in pressure.values()))}


def distribution(values):
    return {"median": statistics.median(values), "p95": max(values)}  # nearest rank for 15 values


def observations(rows, width):
    if len(rows) == 1 and rows[0]["status"].startswith("unsupported"):
        return {"pairs": 0, "reason": rows[0]["status"]}
    if len(rows) != 15 or {int(r["pair"]) for r in rows} != set(range(15)):
        raise ValueError("missing or duplicated native pair")
    ns, cycles = [], []
    for row in rows:
        if int(row["calls"]) != 4096 or int(row["page_bytes"]) != 4096 or width <= 0 or 4096 % width:
            raise ValueError("unexpected kernel invocation or block geometry")
        if row["order"] != ("call-first" if int(row["pair"]) % 2 else "kernel-first"):
            raise ValueError("expected alternating call order")
        iterations = int(row["calls"]) * (4096 // width)
        ns.append((int(row["kernel_ns"]) - int(row["call_ns"])) / iterations)
        kc, ke, kr, cc, ce, cr = [int(row[key]) for key in CLOCK_FIELDS]
        valid = ke == kr and ke > 0 and ce == cr and ce > 0
        if (row["status"] == "counted") != valid:
            raise ValueError("PMU status disagrees with raw enabled/running times")
        if valid and kc > cc:
            cycles.append((kc - cc) / iterations)
    result = {"pairs": 15, "ns_per_block": distribution(ns), "usable_cycle_pairs": len(cycles)}
    if len(cycles) == 15:
        result["cycles_per_block"] = distribution(cycles)
    else:
        result["reason"] = "15 uninterrupted positive paired cycle deltas unavailable"
    return result


def grade(model, observed):
    if "cycles_per_block" in model and "cycles_per_block" in observed:
        median = observed["cycles_per_block"]["median"]
        error = abs(model["cycles_per_block"] - median) / median
        model.update(relative_error=error, verdict="within 20%" if error <= .20 else "outside 20%")
    else:
        model["verdict"] = "unavailable"
    return model


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("pages", type=Path)
    parser.add_argument("artifacts", type=Path)
    args = parser.parse_args()
    pages, out = args.pages.resolve(), args.artifacts.resolve()
    if subprocess.check_output(["git", "-C", pages, "rev-parse", "HEAD"], text=True).strip() != PIN:
        raise ValueError("unexpected A1 source revision")
    arch = platform.machine()
    samples = out / "page-samples.csv"
    with samples.open() as stream:
        rows = list(csv.DictReader(stream))
    cpus = {int(row["cpu"]) for row in rows}
    if len(cpus) != 1:
        raise ValueError("expected native measurement on one pinned CPU")
    os.sched_setaffinity(0, cpus)
    mca_version = subprocess.check_output(["llvm-mca-19", "--version"], text=True)
    match = re.search(r"Host CPU:\s+(\S+)", mca_version)
    cpu = match[1] if match else "unknown"
    paths = inputs(pages, out, arch)
    groups = defaultdict(list)
    for row in rows:
        groups[row["kernel"], int(row["hint"]), int(row["fill"])].append(row)
    validate_cohorts(paths, groups)
    results = {"a1_commit": PIN, "architecture": arch, "cpu": cpu, "logical_cpu": min(cpus),
               "samples_sha256": sha(samples), "results": []}
    cache = {}
    if ("zva64", False) in paths:
        _, info, reason = paths["zva64", False]
        results["zva_probe"] = dict(info, reason=reason)
    print("| Kernel | Prefetch B | Byte | Unit B | Native cycles median / p95 | LLVM cycles / verdict | OSACA cycles / verdict |")
    print("| --- | ---: | ---: | ---: | ---: | --- | --- |")
    for (name, hint, fill), records in groups.items():
        key = name, bool(hint)
        hot, info, reason = paths[key]
        width = info.get("emitted_bytes_per_block", int(records[0]["unit_bytes"]))
        observation = observations(records, width)
        if key not in cache:
            dest = out / f"{name}-pf{hint}"
            if reason:
                cache[key] = ({"reason": reason}, {"reason": reason})
            else:
                dest.with_suffix(".path.s").write_text(hot)
                cache[key] = (llvm_model(hot, arch, cpu, dest), osaca_model(hot, cpu, dest))
        llvm, osaca = [grade(dict(model), observation) for model in cache[key]]
        results["results"].append(dict(info, kernel=name, hint=hint, fill=fill, bytes_per_block=width,
                                       observed=observation, llvm=llvm, osaca=osaca))
        obs = observation.get("cycles_per_block")
        observed = f"{obs['median']:.3f} / {obs['p95']:.3f}" if obs else "unavailable"
        def cell(model):
            value = model.get("cycles_per_block")
            return (f"{value:.3f} / " if value is not None else "") + model["verdict"]
        print(f"| {name} | {hint} | {fill} | {width} | {observed} | {cell(llvm)} | {cell(osaca)} |")
    (out / "page-models.json").write_text(json.dumps(results, indent=2) + "\n")


if __name__ == "__main__":
    main()
