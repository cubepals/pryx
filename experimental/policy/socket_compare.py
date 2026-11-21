# SPDX-License-Identifier: FSL-1.1-ALv2
"""Compare fixed loopback UDP decisions and every policy map with the two CI-only policy images.

Each image gets new programs, maps, a network namespace and a leaf cgroup. Only a cooperating
child issues test traffic there. Its parent observes live socket storage through duplicated FDs.
"""
import array
from contextlib import ExitStack
import errno
import hashlib
import ipaddress
import json
import os
import signal
from pathlib import Path
import socket
import struct
import subprocess
import tempfile
import time
import traceback

from counted_object import DIGEST
from socket_hooks import ADDRESSES, PACKET_HOOKS, PAYLOAD, SOCKET_HOOKS, SocketUAPI


CLIENT, OWNED, ALLOWED, DENIED = 26401, 26402, 26403, 26404
NAMES = ("client", "owned", "allowed", "denied")
STAGES = ("ready", "deny-send", "deny-connect", "owned-send", "allow-send", "owned-connect", "allow-connect")


def rule_entries():
    value = bytearray(200)
    value[:2] = bytes([2, 2])  # PRYX_OWNED fallback, two explicit UDP port ranges.
    for slot, (port, verdict) in enumerate(((DENIED, 0), (ALLOWED, 1))):
        struct.pack_into("=HHBBH", value, 8 + slot * 8, port, port, 2, verdict, 0)
    return {struct.pack("=I", 128) + ipaddress.IPv6Address(ip).packed: bytes(value)
            for ip in ("::ffff:127.0.0.1", "::1")}


def report(control, stage, address, action, error=0, sockets=()):
    message = json.dumps([stage, address, action, error]).encode()
    rights = [(socket.SOL_SOCKET, socket.SCM_RIGHTS, array.array("i", (s.fileno() for s in sockets)))] if sockets else []
    if control.sendmsg([message], rights) != len(message) or control.recv(1) != b"1":
        raise RuntimeError("private observer did not acknowledge a complete event")


def fixed_child(control, cgroup):
    (cgroup/"cgroup.procs").write_text("0")
    for family, address in ADDRESSES:
        with ExitStack() as owned:
            sockets = []
            for port in (CLIENT, OWNED, ALLOWED, DENIED):
                sock = owned.enter_context(socket.socket(family, socket.SOCK_DGRAM))
                if family == socket.AF_INET6:
                    sock.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 1)
                sock.bind((address, port))
                sock.settimeout(2)
                sockets.append(sock)
            client, peer, allowed, denied = sockets
            report(control, "ready", address, "bound", sockets=sockets)
            for stage in STAGES[1:]:
                if stage.startswith("deny"):
                    try:
                        if stage == "deny-send":
                            client.sendto(PAYLOAD, denied.getsockname())
                        else:
                            client.connect(denied.getsockname())
                    except OSError as refusal:
                        if refusal.errno != errno.EPERM:
                            raise
                    else:
                        raise ValueError(f"{stage} unexpectedly permitted")
                    denied.setblocking(False)
                    try:
                        denied.recvfrom(128)
                    except BlockingIOError:
                        pass
                    else:
                        raise ValueError("a refused datagram reached its endpoint")
                    report(control, stage, address, "refuse", errno.EPERM)
                else:
                    destination = allowed if stage.startswith("allow") else peer
                    endpoint = destination.getsockname()
                    if stage.endswith("send"):
                        if client.sendto(PAYLOAD, endpoint) != len(PAYLOAD):
                            raise ValueError("incomplete loopback datagram")
                        data, source = destination.recvfrom(128)
                        if data != PAYLOAD or source != client.getsockname():
                            raise ValueError("wrong datagram or source endpoint")
                    else:
                        client.connect(endpoint)
                        if client.getpeername() != endpoint:
                            raise ValueError("wrong connected endpoint")
                    report(control, stage, address, "permit")


def raw_bytes(items):
    return bytes(int(item, 16) for item in items)


def map_dump(tool, maps, name):
    return json.loads(subprocess.check_output([tool, "-j", "map", "dump", "pinned", maps/name], timeout=10))


def observe(tool, maps, reader, map_fds, fds, cookies, address, stage):
    values = subprocess.check_output([reader, *map(str, (*map_fds, *fds))], pass_fds=(*map_fds, *fds), text=True, timeout=10)
    states = {}
    for name, line in zip(NAMES, values.splitlines(), strict=True):
        cookie, role, data = line.split()
        if int(role) != 1 or bytes.fromhex(data) != bytes(24):
            raise ValueError(f"{address} {stage} {name}: unexpected socket ownership or live state")
        if name == "client":
            previous = cookies.setdefault(address, int(cookie))
            if previous != int(cookie) or len(set(cookies.values())) != len(cookies):
                raise ValueError("kernel changed or reused a live fixture cookie")
        states[name] = {"role": int(role), "state": data}
    rules = {raw_bytes(row["key"]): raw_bytes(row["value"]) for row in map_dump(tool, maps, "rules")}
    config = {raw_bytes(row["key"]): raw_bytes(row["value"]) for row in map_dump(tool, maps, "config")}
    if rules != rule_entries() or config != {bytes(4): bytes(96)}:
        raise ValueError(f"{address} {stage}: policy rules/config changed")
    peers = []
    for row in map_dump(tool, maps, "peers"):
        key = raw_bytes(row["key"])
        if len(key) != 32:
            raise ValueError("unexpected peer key layout")
        cookie = struct.unpack_from("=Q", key)[0]
        ip = next((ip for ip, value in cookies.items() if value == cookie), None)
        packed = ipaddress.IPv6Address("::ffff:127.0.0.1" if ip == "127.0.0.1" else "::1").packed
        if ip is None or key[8:24] != packed or key[24:26] != struct.pack("!H", ALLOWED) or any(key[26:]):
            raise ValueError("wrong UDP socket, peer address, port or padding in peers map")
        if raw_bytes(row["value"]) != b"\x01":
            raise ValueError("unexpected peer verdict")
        peers.append(ip)
    per_cpu = {}
    totals = {}
    for row in map_dump(tool, maps, "counters"):
        index, = struct.unpack("=I", raw_bytes(row["key"]))
        per_cpu[index] = {cpu["cpu"]: struct.unpack("=QQ", raw_bytes(cpu["value"])) for cpu in row["values"]}
        totals[index] = tuple(map(sum, zip(*per_cpu[index].values(), strict=True)))
    step = STAGES.index(stage)
    prior = int(address == "::1")
    datagrams = 2*prior + int(step >= 3) + int(step >= 4)
    refusals = 2*prior + int(step >= 1) + int(step >= 2)
    # Fresh UDP sockets use the normal IPv4/IPv6 header and an 8-byte UDP header.
    packet_bytes = 2*prior*(20+8+len(PAYLOAD)) + (int(step >= 3) + int(step >= 4))*((40 if prior else 20)+8+len(PAYLOAD))
    expected_peers = ["127.0.0.1"] if prior else []
    if step >= 4:
        expected_peers.append(address)
    if sorted(peers) != sorted(expected_peers):
        raise ValueError(f"unexpected peer decisions at {address} {stage}: {peers}")
    if (set(totals) != set(range(5)) or totals[0] != (datagrams, packet_bytes)
            or totals[1] != (datagrams, packet_bytes) or totals[2] != (refusals, 0)
            or totals[3] != (0, 0) or totals[4] != (0, 0)):
        raise ValueError(f"unexpected policy counters at {address} {stage}: {totals}")
    return {"socket_storage": states, "peers": sorted(peers), "counters": totals,
            "rules_sha256": [hashlib.sha256(key + val).hexdigest() for key, val in sorted(rules.items())],
            "config_hex": next(iter(config.values())).hex()}, per_cpu


def reap_child(pid):
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        result, status = os.waitpid(pid, os.WNOHANG)
        if result == pid:
            return status
        time.sleep(0.05)
    # Only this cooperating, still-unreaped child can own the pid.
    try:
        os.kill(pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    return os.waitpid(pid, 0)[1]


def exchange(tool, maps, reader, map_fds, cgroup, output):
    parent, child = socket.socketpair(socket.AF_UNIX, socket.SOCK_SEQPACKET)
    parent.settimeout(60)
    child.settimeout(60)
    pid = os.fork()
    if pid == 0:
        parent.close()
        try:
            fixed_child(child, cgroup)
        except BaseException:
            traceback.print_exc()
            os._exit(1)
        os._exit(0)
    child.close()
    records, cookies = [], {}
    try:
        with parent, (output/"events.jsonl").open("x", buffering=1) as journal:
            for _, address in ADDRESSES:
                with ExitStack() as live:
                    fds = array.array("i")
                    for stage in STAGES:
                        msg, ancillary, flags, _ = parent.recvmsg(2048, socket.CMSG_SPACE(4*fds.itemsize))
                        if flags & (socket.MSG_CTRUNC | socket.MSG_TRUNC):
                            raise ValueError("truncated observer event")
                        for level, kind, data in ancillary:
                            if (level, kind) != (socket.SOL_SOCKET, socket.SCM_RIGHTS):
                                raise ValueError("unexpected observer ancillary message")
                            received = array.array("i", data)
                            for fd in received:
                                live.callback(os.close, fd)
                            fds.extend(received)
                        expected = ["bound", 0] if stage == "ready" else (
                            ["refuse", errno.EPERM] if stage.startswith("deny") else ["permit", 0])
                        if len(fds) != 4 or json.loads(msg) != [stage, address, *expected]:
                            raise ValueError("missing or inconsistent fixed child event")
                        state, per_cpu = observe(tool, maps, reader, map_fds, fds, cookies, address, stage)
                        event = {"stage": stage, "address": address, "action": expected[0], "errno": expected[1],
                                 **state, "per_cpu_counters": per_cpu}
                        records.append(event)
                        print(json.dumps(event), file=journal)
                        parent.sendall(b"1")
    finally:
        parent.close()
        status = reap_child(pid)
    if os.waitstatus_to_exitcode(status) != 0:
        raise RuntimeError("private UDP child failed")
    return records


def variant(tool, image, reader, output):
    os.unshare(os.CLONE_NEWNET)
    if socket.if_nameindex() != [(1, "lo")]:
        raise RuntimeError("private fixture requires only the new network namespace's loopback")
    subprocess.run(["ip", "link", "set", "dev", "lo", "up"], check=True, timeout=10)
    os.sched_setaffinity(0, {min(os.sched_getaffinity(0))})
    uapi = SocketUAPI()
    with tempfile.TemporaryDirectory(prefix="pryx-lab-udp-", dir="/sys/fs/bpf") as scratch, ExitStack() as owned:
        pins, maps = Path(scratch)/"programs", Path(scratch)/"maps"
        maps.mkdir()
        with (output/"verifier.log").open("xb") as log:
            subprocess.run([tool, "--debug", "prog", "loadall", image, pins, "pinmaps", maps],
                           check=True, stdout=log, stderr=subprocess.STDOUT, timeout=90)
        if {path.name for path in pins.iterdir()} != set(SOCKET_HOOKS+PACKET_HOOKS):
            raise ValueError("unexpected pinned policy hooks")
        if {path.name for path in maps.iterdir()} != {"rules","config","peers","counters","owners","sockets"}:
            raise ValueError("unexpected pinned policy maps")
        for key, value in rule_entries().items():
            subprocess.run([tool, "map", "update", "pinned", maps/"rules", "key", "hex", *key.hex(" ").split(),
                            "value", "hex", *value.hex(" ").split(), "noexist"], check=True, timeout=10)
        cgroup = Path(tempfile.mkdtemp(prefix="pryx-lab-udp-", dir="/sys/fs/cgroup"))
        owned.callback(cgroup.rmdir)
        cgroup_fd = os.open(cgroup, os.O_RDONLY|os.O_DIRECTORY)
        owned.callback(os.close, cgroup_fd)
        for name in SOCKET_HOOKS+PACKET_HOOKS:
            program = uapi.open_program(pins/name)
            owned.callback(os.close, program)
            link = uapi.attach(name, program, cgroup_fd)
            owned.callback(os.close, link)
        # BPF_OBJ_GET also opens these two pinned SK_STORAGE maps. The observer only looks up values.
        map_fds = []
        for name in ("owners", "sockets"):
            fd = uapi.open_program(maps/name)
            owned.callback(os.close, fd)
            map_fds.append(fd)
        records = exchange(tool, maps, reader, map_fds, cgroup, output)
    (output/"results.json").write_text(json.dumps(records, indent=2) + "\n")


def compare_private(tool: str, stock: Path, candidate: Path, output: Path):
    """Called by the named CI-only candidate driver after it builds the two pinned images."""
    if (os.environ.get("GITHUB_ACTIONS") != "true" or os.environ.get("GITHUB_REPOSITORY") != "cubepals/pryx"
            or os.environ.get("GITHUB_EVENT_NAME") != "workflow_dispatch" or os.geteuid() != 0):
        raise RuntimeError("requires manually dispatched privileged pryx CI")
    for mount, kind in (("/sys/fs/bpf", "bpf"), ("/sys/fs/cgroup", "cgroup2")):
        if not any(row.split()[4] == mount and row.split()[-3] == kind
                   for row in Path("/proc/self/mountinfo").read_text().splitlines()):
            raise RuntimeError(f"requires {kind} at {mount}")
    from candidate_image import build_images
    root = Path(__file__).resolve().parents[2]
    output.mkdir(parents=True, exist_ok=False)
    with tempfile.TemporaryDirectory(prefix="pryx-udp-parity-") as scratch:
        tmp = Path(scratch)
        verified = build_images(root, tmp/"verified")
        if (hashlib.sha256(stock.read_bytes()).hexdigest() != DIGEST
                or any(Path(a).read_bytes() != Path(b).read_bytes() for a,b in zip(verified,(stock,candidate),strict=True))):
            raise ValueError("worker received different bytes from the fixed source reconstruction")
        reader = tmp/"socket-state"
        subprocess.run(["cc", "-std=c11", "-Wall", "-Wextra", "-Werror", "-I", root/"bpf",
                        Path(__file__).with_name("socket_state.c"), "-o", reader], check=True)
        for label, image in zip(("stock","candidate"), verified, strict=True):
            folder = output/label
            folder.mkdir()
            pid = os.fork()
            if pid == 0:
                try:
                    variant(tool, image, reader, folder)
                except BaseException:
                    traceback.print_exc()
                    os._exit(1)
                os._exit(0)
            if os.waitstatus_to_exitcode(os.waitpid(pid, 0)[1]) != 0:
                raise RuntimeError(f"{label}: disposable UDP fixture failed")
    stock_events, candidate_events = (json.loads((output/label/"results.json").read_text())
                                     for label in ("stock","candidate"))
    if stock_events != candidate_events or len(stock_events) != len(STAGES)*len(ADDRESSES):
        raise ValueError("socket actions, errno, or private map state including per-CPU counters differ; see events.jsonl")
    return {"fixture":"IPv4 + IPv6 UDP on isolated loopback, two accepted destinations and explicit denial",
            "images":{label:hashlib.sha256(path.read_bytes()).hexdigest()
                      for label,path in (("stock",stock),("candidate",candidate))},
            "events_compared":len(stock_events),"equal":True,"per_cpu_counter_equality_required":True}
