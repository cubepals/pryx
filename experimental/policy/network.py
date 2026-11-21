# SPDX-License-Identifier: FSL-1.1-ALv2
"""Read the shipped cgroup BPF object's native verifier evidence without attaching it.

Keep the disassembly, raw verifier log and kernel program info. A static slot count, the
number of verifier visits and the number of instructions executed for an event are distinct.
"""
import argparse
import hashlib
import json
from pathlib import Path
import platform
import subprocess
import tempfile

from corpus import packets


def record(directory, name, command):
    with (directory / name).open("xb") as output:
        subprocess.run(command, stdout=output, stderr=subprocess.STDOUT, check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="new directory for this invocation")
    parser.add_argument("--verify", action="store_true", help="load all hooks temporarily, without attaching them")
    parser.add_argument("--skb", action="store_true", help="also test ingress/egress in the kernel packet harness")
    args = parser.parse_args()
    if args.skb and not args.verify:
        parser.error("--skb requires --verify")
    source = Path(__file__).resolve().parents[2] / "bpf/network.bpf.o"
    args.output.mkdir(exist_ok=False, parents=True)
    record(args.output, "disassembly.txt", ["llvm-objdump-18", "--disassemble", source])
    result = {"object_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
              "kernel": platform.release(), "arch": platform.machine()}
    if args.verify:
        # bpftool owns no pre-existing pin. Refuse a host without a mounted bpffs.
        if not any(row.split()[-3] == "bpf" and row.split()[4] == "/sys/fs/bpf"
                   for row in Path("/proc/self/mountinfo").read_text().splitlines()):
            raise RuntimeError("a mounted BPF filesystem is required for --verify")
        with tempfile.TemporaryDirectory(prefix="pryx-lab-", dir="/sys/fs/bpf") as scratch:
            pins = Path(scratch) / "programs"
            record(args.output, "verifier.txt", ["bpftool", "--debug", "prog", "loadall", source, pins])
            result["hooks"] = {}
            for pin in sorted(pins.iterdir()):
                info = json.loads(subprocess.check_output(["bpftool", "-j", "prog", "show", "pinned", pin]))
                if isinstance(info, list):
                    info, = info
                result["hooks"][pin.name] = info
            if args.skb:
                result["skb"] = []
                for label, packet in packets():
                    input_file = args.output / (label + ".bin")
                    input_file.write_bytes(packet)
                    for hook in ["ingress", "egress"]:
                        raw = subprocess.check_output(["bpftool", "-j", "prog", "run", "pinned", pins / hook,
                                                       "data_in", input_file, "repeat", "1000"], text=True)
                        result["skb"].append({"packet": label, "hook": hook, "kernel": json.loads(raw)})
            if not result["hooks"]:
                raise RuntimeError("no verified hooks reported")
    (args.output / "metadata.json").write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    main()
