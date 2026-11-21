#!/usr/bin/env python3
"""Compare same-host PMU cycles per 16 bytes with the host LLVM scheduling model."""
import argparse
import csv
import re
import statistics
import subprocess
from pathlib import Path

p = argparse.ArgumentParser(description=__doc__)
p.add_argument("asm", type=Path)
p.add_argument("samples", type=Path)
p.add_argument("out", type=Path)
p.add_argument("--mca", default="llvm-mca-19")
args = p.parse_args()
text = args.asm.read_text().split("OSACA-BEGIN", 1)[1].split("OSACA-END", 1)[0]
lines = [line for line in text.splitlines() if line.strip() and line.strip() not in ("//","#")]
hot = ".Lblocks:\n" + "\n".join(lines) + "\n"
model = subprocess.run([args.mca,"-mcpu=native","-iterations=100"], input=hot, text=True,
                       capture_output=True, check=True)
args.out.write_text(model.stdout + model.stderr)
rows = list(csv.DictReader(args.samples.open()))
if len(rows) != 15:
    raise ValueError("expected 15 paired samples")
nanos = [(int(r["loop_ns"])-int(r["call_ns"]))/int(r["iterations"]) for r in rows]
print(f"Hot-loop median {statistics.median(nanos):.5f} ns per 16 bytes.")
pred = re.search(r"Block RThroughput:\s+([\d.]+)",model.stdout)
if not pred:
    raise RuntimeError("missing scheduling model result")
estimate=float(pred.group(1))
if not model.stderr and all(r["pmu_valid"] == "1" for r in rows):
    cycles = [(int(r["loop_cycles"])-int(r["call_cycles"]))/int(r["iterations"]) for r in rows]
    observed = statistics.median(cycles)
    error = abs(observed-estimate)/observed
    print(f"Median {observed:.4f} cycles per 16 bytes; LLVM {estimate:.4f}; error {error:.1%}.")
    print("Within 20%." if error <= .20 else "Outside 20%.")
else:
    print(f"LLVM predicts {estimate:.4f} cycles per 16 bytes. Native PMU or exact CPU model unavailable.")

version = subprocess.check_output([args.mca, "--version"], text=True)
host = re.search(r"Host CPU:\s+(\S+)", version)
models = {"znver1":"ZEN1","znver2":"ZEN2","znver3":"ZEN3","znver4":"ZEN4",
          "haswell":"HSW","broadwell":"BDW","skylake-avx512":"SKX","icelake-server":"ICX",
          "sapphirerapids":"SPR","neoverse-n1":"N1","neoverse-v2":"V2"}
cpu = host.group(1) if host else "unknown"
if cpu not in models:
    print(f"OSACA has no selected matching model for LLVM CPU {cpu}.")
else:
    from ruamel.yaml import YAML
    raw = args.out.with_name("osaca.yaml")
    analysis = subprocess.run(["osaca","--arch",models[cpu],"--consider-flag-deps","--yaml-out",str(raw),"/dev/stdin"],
                              input=hot,text=True,capture_output=True,check=True)
    args.out.with_name("osaca.txt").write_text(analysis.stdout + analysis.stderr)
    parsed=YAML(typ="safe").load(raw.read_text())
    if parsed.get("Warnings") or "No final analysis" in analysis.stdout:
        print(f"OSACA withheld a complete prediction for {models[cpu]}.")
    else:
        summary=parsed["Summary"]
        predicted=max(summary["LCD"], *summary["PortPressure"].values())
        print(f"OSACA {models[cpu]} predicts {predicted:.4f} cycles per 16 bytes.")
        if all(r["pmu_valid"] == "1" for r in rows):
            native = statistics.median((int(r["loop_cycles"])-int(r["call_cycles"]))/int(r["iterations"]) for r in rows)
            print(f"OSACA relative error {abs(native-predicted)/native:.1%}.")
