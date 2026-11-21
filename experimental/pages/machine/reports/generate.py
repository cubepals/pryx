#!/usr/bin/env python3
# SPDX-License-Identifier: FSL-1.1-ALv2
"""Expand the actual assembly and model the equal-block or aligned steady-state paths.

The path walker follows the named branch destinations for each scenario, keeping branch
instructions themselves. This avoids charging both the scalar peel and a vector iteration
to one path. The full tool reports retain unknown instructions and microcoded operations.
"""

from concurrent.futures import ThreadPoolExecutor
import os
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parent
MC = os.environ.get("LLVM_MC", "llvm-mc")
MCA = os.environ.get("LLVM_MCA", "llvm-mca")
OSACA = os.environ.get("OSACA", "osaca")
MODELS = {"x86_64": ("skylake-avx512", "SKX"), "aarch64": ("neoverse-n1", "N1")}


def expand(arch):
    command = [MC, f"-triple={arch}", "-filetype=asm"]
    if arch == "x86_64":
        command.append("-x86-asm-syntax=intel")
    path = ROOT.parent / "src" / f"{arch}.S"
    asm = subprocess.check_output([*command, path], text=True)
    functions = {}
    for symbol, body in re.findall(r"^(pryx_page_\w+):\n(.*?)^\s*\.size\s+\1,", asm, re.M | re.S):
        functions[symbol] = [line.strip() for line in body.splitlines() if line.strip()]
    if not functions:
        raise ValueError(f"no assembled kernels in {path}")
    return functions


def scenarios(symbol, arch):
    if symbol.endswith("_rep"):
        return [("count in RCX; internal string loop is microcoded", None, None)]
    if symbol.endswith("_zva64"):
        return [("one-time DC ZVA capability probe", None, None)]
    if symbol.endswith("_zva"):
        return [("aligned 64-byte interior", ".Lzva_loop", None)]
    if "_hash_" in symbol:
        return [("full 16-byte group", ".Lhash", None)]
    if "_diff_" in symbol or "_value_" in symbol:
        return [("whole equal block", ".Lblock", None)]
    if symbol.endswith(("_nt", "_gpr")):
        loop = ".Lalign" if arch == "x86_64" else ".Lbulk"
        if "_clear_" in symbol:
            return [("aligned whole block", loop, None)]
        return [("aligned copy without prefetch", loop, False), ("aligned copy with in-bounds prefetch", loop, True)]
    raise ValueError(f"new assembly symbol needs a path: {symbol}")


def hot_path(body, loop, prefetch):
    labels = {line[:-1]: i for i, line in enumerate(body) if line.endswith(":")}
    if loop:
        starts = [i for name, i in labels.items() if re.fullmatch(re.escape(loop) + r"\d*", name)]
        if len(starts) != 1:
            raise ValueError(f"expected one location for {loop}")
        index = starts[0]
    else:
        index = 0
    seen, output = set(), []
    while index < len(body) and index not in seen:
        seen.add(index)
        line = body[index]
        index += 1
        if line.endswith(":"):
            continue
        if line.startswith(("ret", ".")):
            break
        output.append(line)
        opcode = line.split()[0]
        target = line.split()[-1]
        if target not in labels:
            continue
        jump = (opcode in ("jmp", "b") or target.startswith(".Lbulk") or
                (target.startswith(".Lload") and prefetch is False) or
                (loop and re.fullmatch(re.escape(loop) + r"\d*", target)))
        if jump:
            index = labels[target]
    if loop and index not in seen:
        raise ValueError(f"path left its loop {loop}")
    if not output:
        raise ValueError("empty analytical path")
    return "\n".join(output) + "\n"


def collect(command, instructions):
    result = subprocess.run(command, text=True, input=instructions, capture_output=True)
    # OSACA's wall clock is unrelated to the source or model.
    output = re.sub(r"^Timestamp:.*\n", "", result.stdout + result.stderr, flags=re.M)
    if result.returncode != 0:
        return f"Model unavailable (tool exit {result.returncode}).\n" + output
    return output


def model(work):
    arch, symbol, body = work
    cpu, model_name = MODELS[arch]
    name = symbol.removeprefix("pryx_page_")
    sections = [f"{symbol} / {arch}\nInput: ../src/{arch}.S (llvm-mc macro expansion)\n"]
    for case, loop, hint in scenarios(symbol, arch):
        instructions = hot_path(body, loop, hint)
        sections.append(f"Path: {case}\n\n{instructions}")
        sections.append(f"llvm-mca 19.1.7 -mtriple={arch} -mcpu={cpu} -iterations=100\n" +
                        collect([MCA, f"-mtriple={arch}", f"-mcpu={cpu}", "-iterations=100"], instructions))
        sections.append(f"OSACA 0.7.1 --arch {model_name} --consider-flag-deps\n" +
                        collect([OSACA, "--arch", model_name, "--consider-flag-deps", "/dev/stdin"], instructions))
    (ROOT / f"{arch}-{name}.txt").write_text("\n".join(sections))


if __name__ == "__main__":
    for command, version in [([MC, "--version"], "19.1.7"), ([MCA, "--version"], "19.1.7"), ([OSACA, "-V"], "0.7.1")]:
        if version not in subprocess.check_output(command, text=True):
            raise SystemExit(f"expected {command[0]} {version}")
    tasks = [(arch, name, body) for arch in MODELS for name, body in expand(arch).items()]
    with ThreadPoolExecutor(max_workers=3) as pool:
        list(pool.map(model, tasks))
