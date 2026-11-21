# SPDX-License-Identifier: FSL-1.1-ALv2
"""Collect stock cgroup program statistics for a fixed, private IPv4/IPv6 UDP exchange.

Only the disposable manual worker runs the socket workload. Its runtime statistics
include kernel helpers and apply to whole invocations of each separately loaded program.
"""
import argparse
from contextlib import ExitStack
import ctypes as ct
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import tempfile
import traceback

from counted_object import DIGEST

SOCKET_HOOKS = ("sock_create", "connect4", "connect6", "sendmsg4", "sendmsg6", "recvmsg4", "recvmsg6")
PACKET_HOOKS = ("ingress", "egress")
ADDRESSES = ((socket.AF_INET, "127.0.0.1"), (socket.AF_INET6, "::1"))
PAYLOAD = b"pryx policy fixture"
BATCH = 128
SAMPLES = 15


class SocketUAPI:
    def __init__(self):
        with tempfile.TemporaryDirectory(prefix="pryx-socket-uapi-") as scratch:
            tool = Path(scratch) / "layout"
            subprocess.run(["cc", "-std=c11", "-Wall", "-Wextra", "-Werror",
                            Path(__file__).with_name("socket_uapi.c"), "-o", tool], check=True)
            output = subprocess.check_output([tool], text=True)
        self.fields = {key: int(number) for key, number in (row.split() for row in output.splitlines())}
        self.syscall = ct.CDLL(None, use_errno=True).syscall
        self.syscall.restype = ct.c_long

    def _call(self, command, words=(), path=None):
        # The kernel receives a zeroed native union; each caller sets only its own UAPI fields.
        attr = ct.create_string_buffer(self.fields["attr_size"])
        for field, value in words:
            ct.c_uint32.from_buffer(attr, self.fields[field]).value = value
        if path is not None:
            pathname = ct.create_string_buffer(os.fsencode(path))
            ct.c_uint64.from_buffer(attr, self.fields["pathname"]).value = ct.addressof(pathname)
        result = self.syscall(ct.c_long(self.fields["syscall"]), ct.c_int(self.fields[command]),
                              ct.byref(attr), ct.c_uint(ct.sizeof(attr)))
        return -ct.get_errno() if result == -1 else result

    def open_program(self, path):
        return checked(self._call("object_get", path=path), str(path))

    def direct_test(self, program):
        return self._call("test_run", [("test_program", program)])

    def attach(self, name, program, group):
        return checked(self._call("link_create", [
            ("link_program", program), ("link_target", group), ("link_type", self.fields[name]),
        ]), name)

    def runtime_stats(self):
        return checked(self._call("enable_stats", [("stats_type", self.fields["runtime_stats"])]),
                       "BPF_ENABLE_STATS / BPF_STATS_RUN_TIME")


def checked(result, action):
    if result < 0:
        raise OSError(-result, f"{action}: {os.strerror(-result)}")
    return result


def local_rules(tool, rules):
    # bpf/network.h and src/bpf/object.rs: exact 128-bit prefix, fallback OWNED, zero ranges.
    value = bytes([2]) + bytes(199)
    for address in ("::ffff:127.0.0.1", "::1"):
        key = struct.pack("=I", 128) + ipaddress.IPv6Address(address).packed
        subprocess.run([tool, "map", "update", "pinned", rules, "key", "hex", *key.hex(" ").split(),
                        "value", "hex", *value.hex(" ").split(), "noexist"], check=True)


def traffic(rounds):
    for _ in range(rounds):
        for family, address in ADDRESSES:
            with socket.socket(family, socket.SOCK_DGRAM) as peer, socket.socket(family, socket.SOCK_DGRAM) as client:
                peer.settimeout(2)
                peer.bind((address, 0))
                client.bind((address, 0))
                destination = peer.getsockname()
                if client.sendto(PAYLOAD, destination) != len(PAYLOAD):
                    raise RuntimeError("short loopback datagram")
                received, source = peer.recvfrom(128)
                if received != PAYLOAD or source != client.getsockname():
                    raise RuntimeError("unexpected payload or source")
                client.connect(destination)
                if client.getpeername() != destination:
                    raise RuntimeError("unexpected connected peer")


def run_child(cgroup, rounds):
    # Only the fixed workload enters this fresh cgroup. No helper command executes there.
    child = os.fork()
    if child == 0:
        try:
            (cgroup / "cgroup.procs").write_text("0")
            traffic(rounds)
        except BaseException:
            traceback.print_exc()
            os._exit(1)
        os._exit(0)
    _, status = os.waitpid(child, 0)
    if os.waitstatus_to_exitcode(status) != 0:
        raise RuntimeError("the owned socket workload failed")


def snapshot(tool, pins):
    result = {}
    for name in SOCKET_HOOKS + PACKET_HOOKS:
        info = json.loads(subprocess.check_output([tool, "-j", "prog", "show", "pinned", pins / name]))
        if isinstance(info, list):
            info, = info
        result[name] = {field: info[field] for field in ("id", "tag", "run_cnt", "run_time_ns")}
    return result


def batch_delta(before, after, rounds):
    records = {}
    for hook in SOCKET_HOOKS + PACKET_HOOKS:
        first, last = before[hook], after[hook]
        calls = last["run_cnt"] - first["run_cnt"]
        nanos = last["run_time_ns"] - first["run_time_ns"]
        if (first["id"], first["tag"]) != (last["id"], last["tag"]) or calls <= 0 or nanos <= 0:
            raise ValueError(f"{hook}: missing activity on a single pinned program: {first} -> {last}")
        expected = rounds * (4 if hook == "sock_create" else 1)
        if hook in SOCKET_HOOKS and calls != expected:
            raise ValueError(f"{hook}: expected {expected} socket invocations, saw {calls}")
        records[hook] = {"invocations": calls, "run_time_ns": nanos, "ns_per_invocation": nanos / calls}
    return records


def collect(tool, image, output):
    uapi = SocketUAPI()
    os.unshare(os.CLONE_NEWNET)
    if socket.if_nameindex() != [(1, "lo")]:
        raise RuntimeError("requires a fresh loopback-only network namespace")
    subprocess.run(["ip", "link", "set", "dev", "lo", "up"], check=True)
    cpu = min(os.sched_getaffinity(0))
    os.sched_setaffinity(0, {cpu})
    with tempfile.TemporaryDirectory(prefix="pryx-socket-", dir="/sys/fs/bpf") as scratch, ExitStack() as owned:
        pins, maps = Path(scratch) / "programs", Path(scratch) / "maps"
        maps.mkdir()
        with (output / "socket-verifier.log").open("xb") as log:
            subprocess.run([tool, "--debug", "prog", "loadall", image, pins, "pinmaps", maps],
                           stdout=log, stderr=subprocess.STDOUT, check=True)
        local_rules(tool, maps / "rules")
        cgroup = Path(tempfile.mkdtemp(prefix="pryx-lab-socket-", dir="/sys/fs/cgroup"))
        owned.callback(cgroup.rmdir)
        group_fd = os.open(cgroup, os.O_RDONLY | os.O_DIRECTORY)
        owned.callback(os.close, group_fd)
        direct = {}
        for name in SOCKET_HOOKS + PACKET_HOOKS:
            prog_fd = uapi.open_program(pins / name)
            owned.callback(os.close, prog_fd)
            if name in SOCKET_HOOKS:
                direct[name] = uapi.direct_test(prog_fd)
            link = uapi.attach(name, prog_fd, group_fd)
            owned.callback(os.close, link)
        (output / "socket-direct-test.json").write_text(json.dumps(direct, indent=2) + "\n")
        # Kernel ENOTSUPP (-524) is the expected missing callback. Other direct-test
        # results are preserved verbatim and do not invalidate real socket accounting.
        stats = uapi.runtime_stats()
        owned.callback(os.close, stats)
        run_child(cgroup, 4)
        measurements = [snapshot(tool, pins)]
        batches = []
        with (output / "socket-snapshots.jsonl").open("x", buffering=1) as journal:
            print(json.dumps(measurements[0]), file=journal)
            for _ in range(SAMPLES):
                run_child(cgroup, BATCH)
                measurements.append(snapshot(tool, pins))
                print(json.dumps(measurements[-1]), file=journal)
                batches.append(batch_delta(measurements[-2], measurements[-1], BATCH))
        return {"object_sha256": DIGEST, "kernel": os.uname().release, "arch": os.uname().machine,
                "cpu": cpu, "rounds_per_batch": BATCH, "warmup_rounds": 4, "private_udp_loopback": True,
                "stats": "Linux bpf_prog_info run_time_ns and run_cnt deltas with runtime stats enabled",
                "socket_hook_original_instruction_counts": None, "snapshots": measurements, "batches": batches}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--bpftool", required=True)
    args = parser.parse_args()
    if (os.environ.get("GITHUB_REPOSITORY") != "cubepals/pryx" or os.environ.get("GITHUB_ACTIONS") != "true"
            or os.environ.get("GITHUB_EVENT_NAME") != "workflow_dispatch" or os.geteuid() != 0):
        parser.error("requires the manually dispatched privileged pryx CI worker")
    image = Path(__file__).resolve().parents[2] / "bpf/network.bpf.o"
    if hashlib.sha256(image.read_bytes()).hexdigest() != DIGEST:
        parser.error("unexpected policy object")
    for path, filesystem in [("/sys/fs/bpf", "bpf"), ("/sys/fs/cgroup", "cgroup2")]:
        if not any(line.split()[4] == path and line.split()[-3] == filesystem
                   for line in Path("/proc/self/mountinfo").read_text().splitlines()):
            parser.error(f"requires {filesystem} at {path}")
    args.output.mkdir(parents=True, exist_ok=False)
    report = collect(args.bpftool, image, args.output)
    (args.output / "socket-runtime.json").write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
