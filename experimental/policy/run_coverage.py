# SPDX-License-Identifier: FSL-1.1-ALv2
"""Measure basic-block visits in the kernel test harness for the repository's packet fixture.

Load fresh private maps for each round, pin the caller to one CPU and compare the stock
return, packet, context outside the diagnostic words and map effects after every fixture.
Count original instructions only on an unambiguous complete entry-to-exit path.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import struct
import subprocess
import tempfile

from counted_object import DIGEST, HOOKS, build, executed_path
from corpus import packets


def digest(data):
    return hashlib.sha256(data).hexdigest()


def map_state(tool, maps):
    # The two pinned skb programs only mutate counters. Their socket-storage lookups
    # never create values. Some kernels decline enumeration of a socket-storage map.
    raw = subprocess.check_output([tool, "-j", "map", "dump", "pinned", maps/"counters"])
    return {"counters":digest(json.dumps(json.loads(raw),sort_keys=True).encode())}


def run_image(tool, image, output, layout):
    size, cb, width, _, _ = layout
    events = {}
    with tempfile.TemporaryDirectory(prefix="pryx-lab-trace-", dir="/sys/fs/bpf") as path:
        pins, maps = Path(path)/"programs", Path(path)/"maps"
        maps.mkdir()
        with (output/"verifier.log").open("wb") as log:
            subprocess.run([tool, "--debug", "prog", "loadall", image, pins, "pinmaps", maps],
                           stdout=log, stderr=subprocess.STDOUT, check=True)
        initial = map_state(tool, maps)
        ctx = output/"zero.ctx"
        ctx.write_bytes(bytes(size))
        for label, packet in packets():
            (output/"packet.in").write_bytes(packet)
            for hook in HOOKS:
                received, context = output/"packet.out", output/"packet.ctx"
                raw = subprocess.check_output([tool,"-j","prog","run","pinned",pins/hook,
                                               "data_in",output/"packet.in","data_out",received,
                                               "data_size_out","4096","ctx_in",ctx,"ctx_out",context,
                                               "ctx_size_out",str(size),"repeat","1"])
                result = json.loads(raw)
                record = bytearray(context.read_bytes())
                if len(record) < cb+width or result["retval"] not in (0, 1):
                    raise ValueError("kernel returned an incomplete context or an invalid policy action")
                words = struct.unpack_from("<5I", record, cb)
                record[cb:cb+width] = bytes(width)
                events[(label,hook)] = {
                    "words":words,"verdict":result["retval"],"packet":digest(received.read_bytes()),
                    "context":digest(record),"maps":map_state(tool,maps)}
        for f in ("packet.in","packet.out","packet.ctx","zero.ctx"):
            (output/f).unlink()
    return initial, events


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("object",type=Path)
    parser.add_argument("output",type=Path)
    parser.add_argument("--bpftool",required=True)
    parser.add_argument("--objcopy",default="llvm-objcopy-18")
    args = parser.parse_args()
    if (os.environ.get("GITHUB_REPOSITORY") != "cubepals/pryx"
        or os.environ.get("GITHUB_ACTIONS") != "true"
        or hashlib.sha256(args.object.read_bytes()).hexdigest() != DIGEST):
        parser.error("requires the pinned pryx object in its native CI")
    os.sched_setaffinity(0, {min(os.sched_getaffinity(0))})
    args.output.mkdir(exist_ok=False,parents=True)
    with tempfile.TemporaryDirectory(prefix="pryx-coverage-files-") as tmp:
        work = Path(tmp)
        binary = work/"layout"
        # The checked-in source queries UAPI structure offsets and helper IDs.
        subprocess.run(["cc","-Wall","-Wextra","-Werror",Path(__file__).with_name("layout.c"),"-o",binary],check=True)
        layout = list(map(int,subprocess.check_output([binary],text=True).split()))
        if layout[2] != 20:
            raise ValueError("unexpected kernel scratch size")
        stock = args.output/"stock"
        stock.mkdir()
        initial, baseline = run_image(args.bpftool,args.object,stock,layout)
        copy = work/"counted.o"
        blocks = build(args.objcopy,args.object,copy,layout[1],*layout[3:])
        folder = args.output/"counted"
        folder.mkdir()
        initial_copy, compare = run_image(args.bpftool,copy,folder,layout)
        if initial_copy != initial or compare.keys() != baseline.keys():
            raise ValueError("test fixtures or initial map states differ")
    report = []
    for key,result in compare.items():
        packet,hook = key
        lo,hi,valid,rest1,rest2 = result.pop("words")
        before = dict(baseline[key])
        if (any(before.pop("words")) or before != result or valid != 1 or rest1 or rest2):
            raise ValueError(f"stock and diagnostic observables differ at {key}")
        rank = lo | (hi << 32)
        path = executed_path(blocks[hook],rank)
        report.append({"packet":packet,"hook":hook,"path_rank":rank,"stock":baseline[(packet,hook)],
                       "block_entries":[b["entry"] for b in path],
                       "original_pcs":[p for b in path for p in b["pcs"]],
                       "instructions":sum(len(b["pcs"]) for b in path),
                       "map_lookups":sum(b["map_lookups"] for b in path),
                       "socket_storage_gets":sum(b["socket_storage_gets"] for b in path)})
    (args.output/"basic-blocks.json").write_text(json.dumps(blocks)+"\n")
    (args.output/"instruction-counts.json").write_text(json.dumps({"object":DIGEST,"cases":report})+"\n")


if __name__ == "__main__":
    main()
