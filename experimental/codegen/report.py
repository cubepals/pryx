#!/usr/bin/env python3
"""Extract the checked-in static loop bodies from actual object disassembly for both models."""
import argparse
import re
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent

def run(args, **kwargs):
    return subprocess.run(args, text=True, capture_output=True, check=True, **kwargs).stdout

def report(tools, arch, cpu, model, out):
    source = ROOT / "machine" / "src" / f"{arch}.S"
    with tempfile.TemporaryDirectory() as scratch:
        obj = Path(scratch) / "scans.o"
        run([tools / "llvm-mc", f"-triple={arch}", "-filetype=obj", source, "-o", obj])
        results = {name: [] for name in ("llvm-mca", "osaca")}
        for width in ([16, 32, 64] if arch == "x86_64" else [16]):
            for count in (1, 2, 4):
                for operation in ("", "copy_"):
                    symbol = f"pryx_static_{operation}{width}_{count}"
                    text = run([tools / "llvm-objdump", "-d", "--no-show-raw-insn", "--no-leading-addr",
                                f"--disassemble-symbols={symbol}", obj])
                    lines = text.split(f"<{symbol}>:", 1)[1].strip().splitlines()
                    body = []
                    for line in lines[1 if not operation else 0:]:
                        op = line.strip().split(None, 1)[0]
                        if op in ("jne", "b.ne"):
                            body.append(f"{op} .Lloop")
                            break
                        body.append(line.strip())
                    else:
                        raise ValueError(f"no back edge in {symbol}")
                    section = f"\n{symbol} ({width * count} bytes per iteration)\n"
                    asm = "\n".join([".Lloop:", *body, ""])
                    results["llvm-mca"].append(section + run([tools / "llvm-mca", f"-mtriple={arch}",
                                                               f"-mcpu={cpu}", "-iterations=100"], input=asm))
                    prediction = run(["osaca", "--arch", model, "--consider-flag-deps", "/dev/stdin"], input=asm)
                    results["osaca"].append(section + re.sub(r"^Timestamp:.*\n", "", prediction, flags=re.M))
        if arch == "x86_64":
            note = ("\npryx_static_copy_1_1 uses REP MOVSB with RCX bytes per invocation. "
                    "These static loop models do not model data-dependent fast-string throughput.\n")
            for sections in results.values():
                sections.append(note)
        for name, sections in results.items():
            version = run([tools / name, "--version"] if name == "llvm-mca" else ["osaca", "-V"]).splitlines()[0]
            report_text = version + "\n" + "".join(sections)
            (out / f"{arch}-{name}.txt").write_text("\n".join(line.rstrip() for line in report_text.splitlines()) + "\n")

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("llvm_bin", type=Path)
    parser.add_argument("--out", type=Path, default=ROOT/"reports")
    args = parser.parse_args()
    args.out.mkdir(exist_ok=True, parents=True)
    report(args.llvm_bin, "x86_64", "skylake-avx512", "SKX", args.out)
    report(args.llvm_bin, "aarch64", "neoverse-n1", "N1", args.out)
